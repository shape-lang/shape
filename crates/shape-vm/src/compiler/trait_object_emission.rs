//! Trait-object emission tier — compiler-side `Arc<VTable>` construction
//! per `(impl Trait for Type)` pair, plus dyn-coerce + dyn-method-call
//! detection helpers consumed by `statements.rs::Statement::VariableDecl`
//! and `expressions/function_calls.rs::compile_expr_method_call`.
//!
//! ADR-006 §2.7.24 Q25.C — emission-tier companion to W17-trait-object-
//! storage. The storage tier landed:
//!  - `HeapKind::TraitObject = 29` + `HeapValue::TraitObject(Arc<TraitObjectStorage>)`
//!  - `TraitObjectStorage { value: Arc<TypedObjectStorage>, vtable: Arc<VTable> }`
//!  - 6-variant `VTableEntry` enum: `Direct` / `Closure` / `BoxedReturn` /
//!    `SelfArg` / `Generic` / `Compound`
//!  - `Erase_T` substitution operator + `ThunkSignature::build` per
//!    (impl, method) pair
//!  - `KindedSlot::from_trait_object` + 4-table lockstep dispatch
//!
//! This module wires the compiler-emission half:
//!  1. **VTable construction** — `build_and_register_vtable` walks each
//!     `impl Trait for Type` block, looks up the trait's declared method
//!     signatures, runs `Erase_T` on each method's return type, and
//!     builds the resulting `Arc<VTable>` keyed by `"Trait::Type"` in
//!     `program.trait_vtables`. Wave 2.6 round-2 handles `Direct` +
//!     `BoxedReturn` paths; `SelfArg` / `Generic` / `Compound` are
//!     surfaced as `NotImplemented` at dispatch time (see
//!     `executor/trait_object_ops.rs`).
//!  2. **Dyn-coerce detection** — exposed via
//!     `trait_name_from_annotation`. Called from `Statement::VariableDecl`
//!     when the type annotation is `TypeAnnotation::Dyn(traits)`; after
//!     the RHS is compiled the compiler emits an `OpCode::BoxTraitObject`
//!     instruction with the trait-name string id as operand.
//!  3. **Dyn-method-call detection** — exposed via
//!     `is_dyn_local`. Called from `compile_expr_method_call` to decide
//!     whether to emit `OpCode::DynMethodCall` vs the standard
//!     `OpCode::CallMethod` path.
//!
//! **Scope per round-2 budget.** No IC devirtualization (§Q25.C.6 —
//! deferred). No LSP cost-class inlay hints (§Q25.C.7 — deferred). No
//! method-generic `Generic`-variant thunk emission (§Q25.C.3 — surfaced
//! at dispatch). No `Self`-arg vtable-identity-check thunk emission
//! (§Q25.C.2 — surfaced at dispatch). No `Compound`-variant emission
//! (§Q25.C.5 — surfaced at dispatch). No nested-Self `BoxedReturn`
//! (`Result<Self, E>`, `(Self, Self)`, etc.) thunk emission — only
//! top-level `Self` in return position (path=[]) is handled; nested
//! cases surface at dispatch.

use shape_ast::ast::{TypeAnnotation, types::ImplBlock, types::TraitMember};
use shape_ast::error::Result;
use shape_value::value::{
    ThunkSignature, VTable, VTableEntry, VTableEntryFlags, WrapTarget,
};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

use super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Build a `VTable` for the given `(impl Trait for Type)` pair and
    /// register it in `program.trait_vtables` keyed by
    /// `"Trait::Type"`.
    ///
    /// The vtable walks the impl's method list; for each method it
    /// looks up the trait declaration to find the **abstract** return
    /// type (which may name `Self`). If the return type contains
    /// `Self`, the method needs a `BoxedReturn` thunk to wrap the
    /// concrete return value into a `dyn Trait` carrier. Otherwise
    /// it's a `Direct` entry that calls the impl's function directly.
    pub(super) fn build_and_register_vtable(
        &mut self,
        trait_basename: &str,
        type_name: &str,
        impl_block: &ImplBlock,
    ) -> Result<()> {
        // Resolve the trait definition so we can read each method's
        // declared signature (the impl method's `return_type` is the
        // concrete form; we need the abstract form with `Self` to know
        // which methods need boxing).
        let (canonical_trait, _) = self.resolve_trait_name(trait_basename);
        let trait_def = match self.trait_defs.get(&canonical_trait) {
            Some(t) => t.clone(),
            None => {
                // No trait def visible — skip vtable build. Direct
                // dispatch via `trait_method_symbols` still works for
                // concrete-typed receivers; trait-object dispatch on
                // this trait will fail at `op_box_trait_object` runtime
                // with a clear "no vtable registered" error.
                return Ok(());
            }
        };

        // Build a method-name → declared signature map from the
        // trait's `Required`/`Default` members.
        let mut trait_method_returns: HashMap<String, TypeAnnotation> =
            HashMap::new();
        let mut trait_method_self_args: HashMap<String, SmallVec<[u8; 4]>> =
            HashMap::new();
        let mut trait_method_generic_count: HashMap<String, u8> =
            HashMap::new();
        for member in &trait_def.members {
            let (mname, return_type, params, type_params) = match member {
                TraitMember::Required(
                    shape_ast::ast::types::InterfaceMember::Method {
                        name,
                        params,
                        return_type,
                        ..
                    },
                ) => (name.clone(), Some(return_type.clone()), Some(params.clone()), None),
                TraitMember::Default(method) => (
                    method.name.clone(),
                    method.return_type.clone(),
                    None, // `MethodDef::params` is a different shape; we
                          // only need the receiver-style params for
                          // Self-arg detection, and required-method
                          // declarations cover the common case.
                    method.type_params.clone(),
                ),
                _ => continue,
            };
            if let Some(rt) = return_type {
                trait_method_returns.insert(mname.clone(), rt);
            }
            // Detect `Self`-typed arguments (excluding the receiver) in
            // the required-method signature shape.
            if let Some(ps) = params {
                let mut self_positions: SmallVec<[u8; 4]> = SmallVec::new();
                for (i, p) in ps.iter().enumerate() {
                    if type_annotation_references_self(&p.type_annotation) {
                        // Receiver is at index 0 in the trait
                        // declaration; we report indices RELATIVE
                        // to the receiver excluded.
                        let receiver_excluded_idx = i.saturating_sub(1);
                        self_positions.push(receiver_excluded_idx as u8);
                    }
                }
                if !self_positions.is_empty() {
                    trait_method_self_args
                        .insert(mname.clone(), self_positions);
                }
            }
            // Detect method-generic parameter count.
            if let Some(tp) = type_params {
                let n = tp.len();
                if n > 0 {
                    trait_method_generic_count
                        .insert(mname.clone(), n.min(u8::MAX as usize) as u8);
                }
            }
        }

        // Build the methods map.
        let mut methods: HashMap<String, VTableEntry> = HashMap::new();
        for method in &impl_block.methods {
            // Resolve the compiled function name (matches the
            // desugared `Type::method` or `Trait::Type::Impl::method`
            // shape registered by `register_function`).
            let impl_name = impl_block.impl_name.as_deref();
            let compiled_fn_name = if let Some(name) = impl_name {
                format!(
                    "{}::{}::{}::{}",
                    trait_basename, type_name, name, method.name
                )
            } else {
                format!("{}::{}", type_name, method.name)
            };
            let func_idx = match self.find_function(&compiled_fn_name) {
                Some(idx) => idx as u16,
                None => continue,
            };

            // Look up the abstract return type from the trait
            // declaration (falls back to the impl's concrete return
            // type if the trait has no entry — covers `Default` methods
            // and conservative-handled gaps).
            let declared_return = trait_method_returns
                .get(&method.name)
                .cloned()
                .or_else(|| method.return_type.clone());

            // Detect whether the declared return type names `Self` at
            // the top level (path=[]). Nested-Self cases
            // (`Result<Self, E>`, `Option<Self>`, `(Self, Self)`) are
            // explicitly **NOT** handled at round-2; they surface as
            // NotImplemented at dispatch time.
            let return_is_self = declared_return
                .as_ref()
                .map(is_top_level_self)
                .unwrap_or(false);
            let return_has_nested_self = declared_return
                .as_ref()
                .map(has_nested_self)
                .unwrap_or(false);

            let self_arg_positions = trait_method_self_args
                .get(&method.name)
                .cloned()
                .unwrap_or_default();
            let type_param_count = trait_method_generic_count
                .get(&method.name)
                .copied()
                .unwrap_or(0);

            // Build a `ThunkSignature` to get the right
            // `VTableEntry::*` shape.
            //
            // Round-2 scope: emit Direct (no rewriting) or BoxedReturn
            // with `wrap_targets = [{ path: [], wrap_as_trait_id: 0 }]`.
            // Wider shapes (SelfArg / Generic / Compound, nested
            // wrap-targets) get the descriptor populated but flagged
            // via the `Compound` arm; the dispatch executor surfaces
            // unsupported variants as NotImplemented.
            let mut wrap_targets: SmallVec<[WrapTarget; 2]> = SmallVec::new();
            if return_is_self {
                // Top-level Self → wrap at path=[]. `wrap_as_trait_id`
                // is the trait id; we don't have a stable u32 trait
                // identity scheme yet — use 0 as a sentinel meaning
                // "the surrounding trait" (the op_box_trait_object
                // dispatch resolves this against the receiver's vtable
                // at dispatch time).
                wrap_targets.push(WrapTarget {
                    path: SmallVec::new(),
                    wrap_as_trait_id: 0,
                });
            }
            if return_has_nested_self {
                // Nested-Self case: surface-and-stop at dispatch.
                // Emit a Compound entry that the executor will reject.
                let mut flags = VTableEntryFlags::empty();
                flags.set(VTableEntryFlags::BOXED_RETURN);
                // We can't compute the right wrap_target paths
                // here without a richer `Erase_T` walk over the
                // compiler's `Type`; flag the entry and the executor
                // surfaces as NotImplemented per §Q25.C.5.
                methods.insert(
                    method.name.clone(),
                    VTableEntry::Compound {
                        thunk_id: func_idx,
                        flags,
                        wrap_targets,
                        self_arg_positions: SmallVec::new(),
                        type_param_count: 0,
                    },
                );
                continue;
            }

            let sig = ThunkSignature::build(
                /*impl_type_id=*/ 0,
                /*trait_id=*/ 0,
                method.name.clone(),
                wrap_targets,
                self_arg_positions,
                type_param_count,
            );
            let entry = sig.to_vtable_entry(func_idx);
            methods.insert(method.name.clone(), entry);
        }

        let vtable = VTable {
            trait_names: vec![trait_basename.to_string()],
            concrete_type_id: 0, // not yet computed; round-2 uses
                                  // `Arc::ptr_eq` on the vtable for
                                  // identity per §Q25.C.2
            methods,
        };
        let key = format!("{}::{}", trait_basename, type_name);
        self.program
            .trait_vtables
            .insert(key, Arc::new(vtable));
        Ok(())
    }
}

/// Whether a `TypeAnnotation` names `Self` directly at the top level
/// (path=[]).
///
/// Per `Erase_T` (§Q25.C.1 row 1): a top-level `Self` return type
/// rewrites to `dyn T` with `wrap_targets = [{ path: [], ... }]`.
pub(super) fn is_top_level_self(ann: &TypeAnnotation) -> bool {
    match ann {
        TypeAnnotation::Basic(name) => name == "Self",
        TypeAnnotation::Reference(path) => path.as_str() == "Self",
        _ => false,
    }
}

/// Whether a `TypeAnnotation` has `Self` inside a generic constructor
/// (`Option<Self>`, `Result<Self, E>`, `(Self, Self)`, etc.).
///
/// Returns `true` for ANY nested Self — we don't distinguish nesting
/// shapes here; the executor surfaces unsupported shapes as
/// NotImplemented at dispatch (§Q25.C.5).
pub(super) fn has_nested_self(ann: &TypeAnnotation) -> bool {
    fn walk(ann: &TypeAnnotation, inside_generic: bool) -> bool {
        match ann {
            TypeAnnotation::Basic(name) => {
                inside_generic && (name == "Self")
            }
            TypeAnnotation::Reference(path) => {
                inside_generic && (path.as_str() == "Self")
            }
            TypeAnnotation::Generic { args, .. } => {
                args.iter().any(|a| walk(a, true))
            }
            TypeAnnotation::Tuple(items) => {
                items.iter().any(|a| walk(a, true))
            }
            TypeAnnotation::Function { params, returns } => {
                params.iter().any(|p| walk(&p.type_annotation, true))
                    || walk(returns, true)
            }
            TypeAnnotation::Array(inner) => walk(inner, true),
            _ => false,
        }
    }
    walk(ann, false)
}

/// Whether a `TypeAnnotation` mentions `Self` anywhere (top level OR
/// nested). Used for `Self`-typed argument detection in
/// `build_and_register_vtable`.
pub(super) fn type_annotation_references_self(ann: &TypeAnnotation) -> bool {
    is_top_level_self(ann) || has_nested_self(ann)
}

/// If the `TypeAnnotation` is a `dyn T` (or `dyn T1 + T2 + ...`),
/// returns the **primary** trait name (the first one in the bound
/// list — the rest contribute to multi-trait inheritance support
/// per §Q25.C.5 `trait_names` field).
///
/// Returns `None` for non-dyn annotations.
pub(crate) fn trait_name_from_annotation(ann: &TypeAnnotation) -> Option<&str> {
    match ann {
        TypeAnnotation::Dyn(traits) if !traits.is_empty() => {
            Some(traits[0].as_str())
        }
        _ => None,
    }
}
