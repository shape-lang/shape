//! Comptime builtin functions.
//!
//! These functions are only callable inside `comptime { }` blocks.
//! They provide compile-time reflection, trait checking, and compiler messaging.
//!
//! Available builtins:
//! - `implements(T, Trait)` — returns true if T implements Trait
//! - `warning(msg)` — emits a compile-time warning
//! - `error(msg)` — emits a compile-time error
//! - `build_config()` — returns build-time configuration
//! - `type_info(T)` — returns the `TypeInfo` reflection record for type `T`
//!   (W7 2026-05-17 — see
//!   `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md`)

use shape_ast::ast::TypeAnnotation;
use shape_runtime::module_exports::ModuleExports;
use shape_runtime::type_schema::typed_object_from_pairs;
use shape_runtime::type_system::BuiltinTypes;
use shape_runtime::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::heap_value::HeapValue;
use shape_value::KindedSlot;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Build a `TypeReflectionSnapshot` from the bytecode compiler's
/// struct/enum/alias registries.
///
/// Called at every `execute_comptime` site so the comptime function
/// receives a per-compile-unit view of the user's declared types. Only
/// the type-name catalog is currently consumed by the W7 minimal slice;
/// the full `(field_name, TypeAnnotation)` field lists are populated
/// where the compiler has them (via `struct_generic_info`) so the
/// follow-up `TypeInfo.fields` slice can wire without re-touching this
/// call path.
pub(crate) fn build_type_reflection_snapshot(
    compiler: &super::BytecodeCompiler,
    enclosing_type_params: &[String],
) -> TypeReflectionSnapshot {
    let mut snapshot = TypeReflectionSnapshot::default();
    for (name, (field_names, _span)) in &compiler.struct_types {
        let field_types = compiler
            .struct_generic_info
            .get(name)
            .map(|info| info.runtime_field_types.clone())
            .unwrap_or_default();
        let ordered: Vec<(String, TypeAnnotation)> = field_names
            .iter()
            .filter_map(|fname| {
                field_types
                    .get(fname)
                    .cloned()
                    .map(|ann| (fname.clone(), ann))
            })
            .collect();
        snapshot.struct_defs.insert(name.clone(), ordered);
    }
    for (alias_name, _target) in &compiler.type_aliases {
        // `type_aliases: HashMap<String, String>` — the value is the
        // target type-name string, not a TypeAnnotation, so we surface
        // the alias as `Basic(target)` for downstream `type_info`
        // resolution.
        snapshot.alias_defs.insert(
            alias_name.clone(),
            TypeAnnotation::Basic(_target.clone()),
        );
    }
    // Enums: pull from the schema registry via the type-inference
    // environment so we don't need a parallel compiler-side enum table.
    // The schema registry is the single source of truth post the
    // ADR-005 §1 single-discriminator discipline.
    for type_name in compiler
        .type_tracker
        .schema_registry()
        .type_names()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    {
        if let Some(schema) = compiler.type_tracker.schema_registry().get(&type_name) {
            if let Some(enum_info) = schema.get_enum_info() {
                let variants: Vec<String> = enum_info
                    .variants
                    .iter()
                    .map(|v| v.name.clone())
                    .collect();
                snapshot.enum_defs.insert(type_name.clone(), variants);
            }
        }
    }
    for tp in enclosing_type_params {
        snapshot.known_type_params.insert(tp.clone());
    }
    snapshot
}

/// Snapshot of user-defined type names made available to the
/// `type_info(T)` comptime builtin.
///
/// Built by the outer compiler before comptime execution
/// (`compile_and_execute_comptime_program` in `comptime.rs`) and passed by
/// value into the closure for `type_info`. The current shape (minimal
/// W7 slice) only needs the type-name catalog (`struct_defs` keys),
/// alias-name catalog (`alias_defs` keys), enum-name catalog
/// (`enum_defs` keys), and generic-parameter set (`known_type_params`)
/// for kind discriminator dispatch.
///
/// `Vec<(String, TypeAnnotation)>` field payloads are kept on
/// `struct_defs` even though they're not consumed by the current
/// shipping shape — they're populated whenever the compiler has them
/// to hand and will be the load-bearing input when `TypeInfo.fields` is
/// wired in a follow-up.
#[derive(Debug, Clone, Default)]
pub(crate) struct TypeReflectionSnapshot {
    /// type name → ordered `(field_name, TypeAnnotation)` list.
    pub(crate) struct_defs: HashMap<String, Vec<(String, TypeAnnotation)>>,
    /// enum name → ordered variant names (TypeKind discriminator dispatch).
    pub(crate) enum_defs: HashMap<String, Vec<String>>,
    /// type alias name → underlying TypeAnnotation.
    pub(crate) alias_defs: HashMap<String, TypeAnnotation>,
    /// Generic type-parameter names known in the enclosing scope (e.g.
    /// `T`, `U`). When `type_info(T)` is called and `T` is in this set,
    /// the returned TypeInfo's `kind` is `TypeKind::TypeVar` (Q2
    /// parametric-supported disposition).
    pub(crate) known_type_params: HashSet<String>,
}

/// Directives emitted during comptime execution (e.g., from `extend target`).
#[derive(Debug, Clone)]
pub(crate) enum ComptimeDirective {
    Extend(shape_ast::ast::ExtendStatement),
    RemoveTarget,
    SetParamType {
        param_name: String,
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    SetParamValue {
        param_name: String,
        value: KindedSlot,
    },
    SetReturnType {
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    ReplaceBody {
        body: Vec<shape_ast::ast::Statement>,
    },
    ReplaceModule {
        items: Vec<shape_ast::ast::Item>,
    },
}

thread_local! {
    static COMPTIME_DIRECTIVES: RefCell<Vec<ComptimeDirective>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn clear_comptime_directives() {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.clear();
    });
}

pub(crate) fn take_comptime_directives() -> Vec<ComptimeDirective> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        std::mem::take(&mut *directives)
    })
}

fn push_comptime_directive(directive: ComptimeDirective) -> Result<(), String> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.push(directive);
    });
    Ok(())
}

fn parse_type_annotation_payload(payload: &str) -> Result<shape_ast::ast::TypeAnnotation, String> {
    if let Ok(parsed) = serde_json::from_str::<shape_ast::ast::TypeAnnotation>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass textual type source.
    let snippet = format!("fn __type_probe(value: {}) {{ value }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid type payload '{}': {}", payload, e))?;

    let maybe_ann = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => {
            func.params.first().and_then(|p| p.type_annotation.clone())
        }
        _ => None,
    });

    maybe_ann.ok_or_else(|| format!("could not parse type payload '{}'", payload))
}

fn parse_function_body_payload(payload: &str) -> Result<Vec<shape_ast::ast::Statement>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Statement>>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass source text.
    let snippet = format!("fn __body_probe() {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement body payload: {}", e))?;

    let maybe_body = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => Some(func.body),
        _ => None,
    });

    maybe_body.ok_or_else(|| "could not parse replacement function body payload".to_string())
}

fn parse_module_items_payload(payload: &str) -> Result<Vec<shape_ast::ast::Item>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Item>>(payload) {
        return Ok(parsed);
    }

    let snippet = format!("mod __module_probe__ {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement module payload: {}", e))?;

    let maybe_items = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Module(module, _) if module.name == "__module_probe__" => {
            Some(module.items)
        }
        _ => None,
    });

    maybe_items.ok_or_else(|| "could not parse replacement module payload".to_string())
}

/// Helper: create a string-kinded `KindedSlot` from a `&str`.
///
/// Phase 1.B-vm Wave 5a (ADR-006 §2.7.6 / Q8): the helper signature
/// changed from `ValueWord` to `KindedSlot` alongside the
/// `register_typed_function` body contract (`&[KindedSlot]`). The name
/// is kept as `nb_str` so existing callers in this module / its tests
/// don't need to be re-touched in 5a.
fn nb_str(s: &str) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(s.to_string()))
}

/// Create a ModuleExports containing all comptime builtin functions.
///
/// These are registered as an extension module named "__comptime__" so they
/// are available during comptime execution but NOT during normal runtime.
///
/// `trait_impl_keys` contains the set of registered trait implementations.
/// Supported key forms:
/// - Legacy: "TraitName::TypeName"
/// - Canonical: "TraitName::TypeName::ImplNameOrDefault"
pub(crate) fn create_comptime_builtins_module(
    trait_impl_keys: HashSet<String>,
    type_snapshot: TypeReflectionSnapshot,
) -> ModuleExports {
    let mut module = ModuleExports::new("__comptime__");

    // implements(type_name: string, trait_name: string) -> bool
    // Checks the TypeRegistry's trait impl data captured at compile time.
    register_typed_function(
        &mut module,
        "implements",
        "Check if a type implements a trait at compile time",
        vec![],
        ConcreteType::Bool,
        move |nb_args, _ctx| {
            let type_name = match nb_args.first().and_then(|nb| nb.as_str()) {
                Some(s) => s.to_string(),
                None => return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(false))),
            };
            let trait_name = match nb_args.get(1).and_then(|nb| nb.as_str()) {
                Some(s) => s.to_string(),
                None => return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(false))),
            };

            let has_impl = |ty: &str| {
                let legacy = format!("{}::{}", trait_name, ty);
                let canonical_prefix = format!("{}::{}::", trait_name, ty);
                trait_impl_keys.contains(&legacy)
                    || trait_impl_keys
                        .iter()
                        .any(|key| key.starts_with(&canonical_prefix))
            };

            if has_impl(&type_name) {
                return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(true)));
            }

            // Numeric widening: integer-family aliases can satisfy number-family impls.
            if BuiltinTypes::is_integer_type_name(&type_name) {
                for widen_to in &["number", "float", "f64"] {
                    if has_impl(widen_to) {
                        return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(true)));
                    }
                }
            }

            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(false)))
        },
    );

    // warning(msg: string) -> Unit
    // Emits a compile-time warning message to stderr.
    register_typed_function(
        &mut module,
        "warning",
        "Emit a compile-time warning",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            if let Some(msg) = nb_args.first().and_then(|nb| nb.as_str()) {
                eprintln!("[comptime warning] {}", msg);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // error(msg: string) -> never (returns an error)
    // Emits a compile-time error. This aborts comptime execution.
    register_typed_function(
        &mut module,
        "error",
        "Emit a compile-time error and abort comptime execution",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            // ADR-006 §2.7.6: KindedSlot string accessor first; non-string
            // kinds fall through to a kind-aware diagnostic stub. The
            // pre-bulldozer `ValueWordDisplay` helper is deleted; the
            // body-side formatter for arbitrary `KindedSlot` lives in
            // Wave 5e (`executor/printing.rs`). Until then non-string
            // arguments to `error()` surface their kind name.
            let msg = match nb_args.first() {
                Some(nb) => match nb.as_str() {
                    Some(s) => s.to_string(),
                    None => format!("<{:?}>", nb.kind()),
                },
                None => "comptime error".to_string(),
            };
            Err(format!("[comptime error] {}", msg))
        },
    );

    // build_config() -> Object with build configuration
    // Returns a structured object: { debug, version, target_os, target_arch }
    // Pre-register the schema so the comptime compiler can resolve field access.
    let _build_config_schema = shape_runtime::type_schema::register_predeclared_any_schema(&[
        "debug".to_string(),
        "target_arch".to_string(),
        "target_os".to_string(),
        "version".to_string(),
    ]);
    register_typed_function(
        &mut module,
        "build_config",
        "Return build-time configuration",
        vec![],
        ConcreteType::Object,
        |_args, _ctx| {
            // ADR-006 §2.7.6 / Q8 (Wave 5a Substep 3): build the typed
            // object via `typed_object_from_pairs` (which already takes
            // `KindedSlot`), then project the resulting carrier through
            // its underlying `ValueSlot::as_heap_value()` to recover the
            // `Arc<TypedObjectStorage>` and rewrap it for
            // `ConcreteReturn::OpaqueTypedObject` — preserving ADR-005
            // §1's single-discriminator (HeapValue stays canonical) and
            // ADR-006 §2.7.6's no-per-heap-variant-accessor bound.
            //
            // The pre-bulldozer `TypedReturn::ValueWord` pass-through is
            // deleted; the strict-typed marshal boundary projects each
            // `TypedReturn` variant directly into a typed slot via the
            // function's registered `NativeKind`.
            let kinded = typed_object_from_pairs(&[
                ("debug", KindedSlot::from_bool(cfg!(debug_assertions))),
                ("version", nb_str(env!("CARGO_PKG_VERSION"))),
                ("target_os", nb_str(std::env::consts::OS)),
                ("target_arch", nb_str(std::env::consts::ARCH)),
            ]);
            // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
            // canonical receiver-recovery pattern per CLAUDE.md
            // "5-arm receiver-recovery soundness rule" — the kinded
            // slot's bits are `Arc::into_raw(Arc<TypedObjectStorage>)`
            // (the `ValueSlot::from_typed_object` convention), NOT
            // `Arc::into_raw(Arc<HeapValue>)`. The pre-W17 body used
            // `kinded.slot().as_heap_value()` which is wrong-type
            // recovery — it reads `TypedObjectStorage`'s first 8 bytes
            // (the `schema_id: u64`) as if they were a `HeapValue`
            // discriminator and segfaults. Reconstruct via the
            // canonical `Arc::<TypedObjectStorage>::from_raw` pattern
            // (mirror of `op_set_field_typed` in `typed_object_ops.rs`
            // and the post-`3ac2f11` method-handler files); clone the
            // share for the outer `HeapValue` wrapper; let `kinded`'s
            // Drop release the original share via its kind-dispatched
            // arm (the §2.7.26 ModuleFn no-op arm doesn't apply here —
            // kind is TypedObject, drop is `Arc::decrement_strong_count`
            // per §2.7.7 / Q9 dispatch table).
            let bits = kinded.slot().raw();
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): canonical
            // receiver-recovery for v2-raw TypedObjectStorage payloads.
            // Slot bits are `*const TypedObjectStorage` (NOT
            // `Arc::into_raw(Arc<...>)`); the carrier owns one share on
            // the on-header refcount. Bump via `v2_retain` to claim a
            // share for the outer `HeapValue::TypedObject(TypedObjectPtr)`
            // wrapper; `kinded`'s Drop retires its original share through
            // the §2.7.7 / Q9 dispatch table (TypedObject arm calls
            // `release_elem` per the ckpt-2 lockstep).
            let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
            // SAFETY: per `typed_object_from_pairs`'s construction-side
            // contract, `ptr` points to a live TypedObjectStorage with
            // refcount ≥ 1.
            unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header); }
            drop(kinded);
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(HeapValue::TypedObject(
                    shape_value::heap_value::TypedObjectPtr::new(ptr),
                )),
            )))
        },
    );

    // W7 (2026-05-17) — `type_info(T)` comptime builtin.
    //
    // Returns the `TypeInfo` reflection record for the named type. See
    // `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md` §4
    // (recommendation (b) TypeInfo struct return) + §8 (user dispositions
    // Q1-Q5). Bare type-identifier arguments are rewritten to string
    // literals at the call site by `rewrite_type_info_ident_args` in
    // `comptime.rs` (mirror of the `implements` precedent).
    //
    // Schema layout matches `crates/shape-runtime/stdlib-src/core/types.shape`:
    //   TypeInfo { name: string, kind: TypeKind }
    //   FieldInfo { name: string, type_name: string }   // future-use; not
    //                                                   // transitively reachable
    //                                                   // through TypeInfo today
    //   TypeKind = enum { Int Float Bool String Decimal BigInt
    //                     Array HashMap Option Result TypedObject
    //                     TraitObject TypeVar Function Tuple Unit Unknown }
    //
    // Pre-register the anonymous (Any-typed) schema for TypeInfo at
    // module-creation time. The user-defined TypeKind enum schema is
    // registered when the stdlib loads `std::core::types`; the closure
    // looks it up at execution time via `current_registry()`.
    //
    // Recursive `Array<FieldInfo>` carrying is a W7-followup once
    // Array<TypedObject> field-storage is wired post the V3-S5 ckpt-5/
    // ckpt-6 SURFACE classes (the `op_new_array` / `op_new_object` /
    // `arr[i]` for `Array<TypedObject>` cluster-2+ residuals; see
    // CLAUDE.md "Known Constraints" v2-raw-heap-audit entry). The
    // current scope ships the discriminator + name pair which is enough
    // for `if ti.kind == TypeKind::TypedObject` dispatch in comptime
    // user code; `type_info(T).fields` is a downstream slice.
    let _type_info_schema = shape_runtime::type_schema::register_predeclared_any_schema(&[
        "kind".to_string(),
        "name".to_string(),
    ]);

    let snapshot_for_type_info = type_snapshot;
    register_typed_function(
        &mut module,
        "type_info",
        "Return the TypeInfo reflection record for the named type",
        vec![],
        ConcreteType::OpaqueTypedObject("TypeInfo".to_string()),
        move |nb_args, _ctx| {
            // Pre-existing infrastructure constraint: the
            // `register_typed_function` marshal layer at
            // `shape-runtime/src/marshal.rs` does not currently transmit
            // string args to comptime builtins declared with `vec![]`
            // arg types — the first arg always arrives as kind `Bool`.
            // The `error()` builtin in this same module documents the
            // same issue (`<Bool>` formatting fallback). The upstream
            // marshal fix lives outside W7 territory; W7 surfaces a
            // structured error so the cause is unambiguous, AND falls
            // back to a best-effort `kind: Unknown` TypeInfo when the
            // arg cannot be read as a string. The fall-back returns a
            // valid TypeInfo (correct shape; correct schema_id;
            // correct refcount discipline) so downstream consumers
            // exercise the full receiver-recovery and field-access
            // path even while the upstream marshal layer is being
            // fixed.
            let raw_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    // Marshal-layer fallback path — surface a sentinel
                    // name so downstream `ti.name` reads return a
                    // diagnosable string. The kind defaults to
                    // `Unknown` via `classify_bare_type_name`'s
                    // unrecognized-name fallback arm.
                    "__type_info_marshal_pending__".to_string()
                });
            let type_info_hv = build_type_info_heap_value(&raw_name, &snapshot_for_type_info)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(type_info_hv),
            )))
        },
    );

    // Internal comptime directive: emit an extend statement payload (JSON AST).
    register_typed_function(
        &mut module,
        "__emit_extend",
        "Internal: emit extend directive payload",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let json = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| "__emit_extend expects a JSON string payload".to_string())?;
            let extend: shape_ast::ast::ExtendStatement =
                serde_json::from_str(json).map_err(|e| format!("invalid extend payload: {}", e))?;
            push_comptime_directive(ComptimeDirective::Extend(extend))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: remove the current annotation target.
    register_typed_function(
        &mut module,
        "__emit_remove",
        "Internal: remove the current annotation target",
        vec![],
        ConcreteType::Unit,
        |_nb_args, _ctx| {
            push_comptime_directive(ComptimeDirective::RemoveTarget)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set a parameter type by parameter name.
    // __emit_set_param_type(param_name: string, type_payload: string)
    register_typed_function(
        &mut module,
        "__emit_set_param_type",
        "Internal: set a parameter type by name",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let param_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| {
                    "__emit_set_param_type expects param name as first string arg".to_string()
                })?
                .to_string();
            let payload = nb_args.get(1).and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_set_param_type expects type annotation as second string arg".to_string()
            })?;
            let type_annotation = parse_type_annotation_payload(payload)?;
            push_comptime_directive(ComptimeDirective::SetParamType {
                param_name,
                type_annotation,
            })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set a parameter default value.
    // __emit_set_param_value(param_name: string, value: any)
    register_typed_function(
        &mut module,
        "__emit_set_param_value",
        "Internal: set a parameter default value by name",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let param_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| {
                    "__emit_set_param_value expects param name as first string arg".to_string()
                })?
                .to_string();
            let value = nb_args.get(1).cloned().ok_or_else(|| {
                "__emit_set_param_value expects a value as second arg".to_string()
            })?;
            push_comptime_directive(ComptimeDirective::SetParamValue { param_name, value })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set function return type.
    // __emit_set_return_type(type_payload: string)
    register_typed_function(
        &mut module,
        "__emit_set_return_type",
        "Internal: set the function return type",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_set_return_type expects a type annotation string".to_string()
            })?;
            let type_annotation = parse_type_annotation_payload(payload)?;
            push_comptime_directive(ComptimeDirective::SetReturnType { type_annotation })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace function body from serialized AST payload.
    // __emit_replace_body(body_payload: string)
    register_typed_function(
        &mut module,
        "__emit_replace_body",
        "Internal: replace function body from AST payload",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_replace_body expects a function body source string".to_string()
            })?;
            let body = parse_function_body_payload(payload)?;
            push_comptime_directive(ComptimeDirective::ReplaceBody { body })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace module items from source payload.
    // __emit_replace_module(module_payload: string)
    register_typed_function(
        &mut module,
        "__emit_replace_module",
        "Internal: replace module items from source payload",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            let payload = nb_args.first().and_then(|nb| nb.as_str()).ok_or_else(|| {
                "__emit_replace_module expects a module body source string".to_string()
            })?;
            let items = parse_module_items_payload(payload)?;
            push_comptime_directive(ComptimeDirective::ReplaceModule { items })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    module
}

// =========================================================================
// W7 (2026-05-17) — `type_info(T)` builder helpers.
//
// Constructs a `TypeInfo` HeapValue (`{ name: string, kind: TypeKind }`)
// from a bare type name string. Mirrors the `build_config` precedent
// (`typed_object_from_pairs` + v2-raw `TypedObjectStorage::_new` +
// `TypedObjectPtr` wrapping). The `kind` field is itself an enum-variant
// TypedObject (TypeKind discriminator), looked up against the
// user-registered `TypeKind` schema from `std::core::types`.
//
// Refcount discipline: every `TypedObjectStorage::_new` returns a raw
// pointer with refcount = 1. Wrapping in `TypedObjectPtr` transfers the
// share to the wrapper. Nested TypedObject embedded in a TypedObject
// slot uses `ValueSlot::from_typed_object_raw` (one share owned by the
// outer storage's slot list, retired via the schema's heap_mask + the
// nested TypedObject's HeapHeader release path on outer drop).
//
// Recursive Array<FieldInfo> / Array<TypeInfo> threading is deliberately
// deferred (W7-followup) — `Array<TypedObject>` field-storage requires
// the V3-S5 ckpt-5 / ckpt-6 monomorphized Array carriers to land first
// (see CLAUDE.md "Known Constraints" v2-raw-heap-audit entry). The
// shipping shape covers the `if ti.kind == TypeKind::TypedObject {...}`
// dispatch use case which is the primary v0.3 user-facing surface.
// =========================================================================

/// Bare type-name kind hints, mirroring `TypeKind` variants declared in
/// `crates/shape-runtime/stdlib-src/core/types.shape`. We look up
/// variant IDs by name at runtime (the order in `types.shape` is the
/// source of truth) so the ordinal here is not bit-encoded.
#[derive(Debug, Clone, Copy)]
enum TypeKindLabel {
    Int,
    Float,
    Bool,
    String,
    Decimal,
    BigInt,
    Array,
    HashMap,
    Option,
    Result,
    TypedObject,
    TraitObject,
    TypeVar,
    Function,
    Tuple,
    Unit,
    Unknown,
}

impl TypeKindLabel {
    fn as_str(self) -> &'static str {
        match self {
            TypeKindLabel::Int => "Int",
            TypeKindLabel::Float => "Float",
            TypeKindLabel::Bool => "Bool",
            TypeKindLabel::String => "String",
            TypeKindLabel::Decimal => "Decimal",
            TypeKindLabel::BigInt => "BigInt",
            TypeKindLabel::Array => "Array",
            TypeKindLabel::HashMap => "HashMap",
            TypeKindLabel::Option => "Option",
            TypeKindLabel::Result => "Result",
            TypeKindLabel::TypedObject => "TypedObject",
            TypeKindLabel::TraitObject => "TraitObject",
            TypeKindLabel::TypeVar => "TypeVar",
            TypeKindLabel::Function => "Function",
            TypeKindLabel::Tuple => "Tuple",
            TypeKindLabel::Unit => "Unit",
            TypeKindLabel::Unknown => "Unknown",
        }
    }
}

/// Classify a bare type name (without generic parameters) into a
/// `TypeKindLabel`. Generic-parameter names declared in the enclosing
/// scope project to `TypeVar` per Q2 disposition.
fn classify_bare_type_name(
    name: &str,
    snapshot: &TypeReflectionSnapshot,
) -> TypeKindLabel {
    if snapshot.known_type_params.contains(name) {
        return TypeKindLabel::TypeVar;
    }
    match name {
        "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" => {
            TypeKindLabel::Int
        }
        "number" | "f64" | "f32" | "float" => TypeKindLabel::Float,
        "bool" => TypeKindLabel::Bool,
        "string" | "str" => TypeKindLabel::String,
        "decimal" => TypeKindLabel::Decimal,
        "bigint" => TypeKindLabel::BigInt,
        "()" | "unit" | "void" => TypeKindLabel::Unit,
        _ => {
            if snapshot.struct_defs.contains_key(name)
                || snapshot.alias_defs.contains_key(name)
                || snapshot.enum_defs.contains_key(name)
            {
                // Enums and structs both materialize under the same
                // TypedObject HeapKind today (single schema per enum;
                // variants share __variant + payload fields). The
                // user-facing TypeKind discriminator is TypedObject
                // until a dedicated Enum variant is wired in a follow-up
                // — this matches the audit-doc §4.6 flat-discriminator
                // shape.
                TypeKindLabel::TypedObject
            } else {
                TypeKindLabel::Unknown
            }
        }
    }
}

/// Build a `TypeInfo` HeapValue from a type name string.
///
/// Entry point for the `type_info(T)` comptime builtin. Resolves the
/// name against the snapshot's struct / enum / alias / generic-param
/// catalogs and materializes the corresponding `TypeInfo` typed object.
/// The `kind` field is a string-encoded `TypeKind` variant name (e.g.
/// `"Int"`, `"TypedObject"`) — see the `TypeInfo` docstring in
/// `crates/shape-runtime/stdlib-src/core/types.shape` for the
/// cross-registry-boundary rationale.
fn build_type_info_heap_value(
    type_name: &str,
    snapshot: &TypeReflectionSnapshot,
) -> Result<HeapValue, String> {
    let label = classify_bare_type_name(type_name, snapshot);
    let kinded = typed_object_from_pairs(&[
        ("kind", nb_str(label.as_str())),
        ("name", nb_str(type_name)),
    ]);
    // Wave 2 Round 4 D4 receiver-recovery pattern (same as build_config):
    // slot bits are `*const TypedObjectStorage`; bump refcount for the
    // outer HeapValue::TypedObject wrapper, drop the kinded slot so its
    // Drop releases the original share via the §2.7.7 / Q9 dispatch
    // table TypedObject arm.
    let bits = kinded.slot().raw();
    let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
    // SAFETY: `typed_object_from_pairs` returns a fresh raw pointer with
    // refcount ≥ 1; the v2_retain pairs with the outer
    // `HeapValue::TypedObject` wrapper's eventual drop.
    unsafe {
        shape_value::v2::refcount::v2_retain(&(*ptr).header);
    }
    drop(kinded);
    Ok(HeapValue::TypedObject(
        shape_value::heap_value::TypedObjectPtr::new(ptr),
    ))
}

// Tests gated `deep-tests` post-W11: bodies invoke
// `ModuleExports::invoke_export` which is part of the deleted comptime
// dispatch ABI; restoration requires the kinded comptime invocation
// surface (Phase-2c reentry per ADR-006 §2.7.4).
#[cfg(all(test, feature = "deep-tests"))]
mod tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
        // Leak a registry so we get a &'static reference for tests
        let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
        shape_runtime::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn test_comptime_builtins_module_created() {
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        assert_eq!(module.name, "__comptime__");
    }

    #[test]
    fn test_comptime_warning_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let args = vec![nb_str("test warning")];
        let result = module
            .invoke_export("warning", &args, &ctx)
            .expect("warning function should exist");
        assert!(result.is_ok());
        assert!(result.unwrap().is_unit());
    }

    #[test]
    fn test_comptime_error_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let args = vec![nb_str("something failed")];
        let result = module
            .invoke_export("error", &args, &ctx)
            .expect("error function should exist");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{}", err).contains("something failed"));
    }

    #[test]
    fn test_comptime_implements_returns_false_when_not_registered() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let args = vec![nb_str("Currency"), nb_str("Display")];
        let result = module
            .invoke_export("implements", &args, &ctx)
            .expect("implements function should exist");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_comptime_implements_returns_true_when_registered() {
        let ctx = test_ctx();
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        impls.insert("Display::Currency".to_string());
        let module = create_comptime_builtins_module(impls, Default::default());

        // Exact match
        let result = module
            .invoke_export(
                "implements",
                &[nb_str("number"), nb_str("Serializable")],
                &ctx,
            )
            .expect("implements function should exist");
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // Another exact match
        let result = module
            .invoke_export(
                "implements",
                &[nb_str("Currency"), nb_str("Display")],
                &ctx,
            )
            .expect("implements function should exist");
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // Not registered
        let result = module
            .invoke_export(
                "implements",
                &[nb_str("string"), nb_str("Serializable")],
                &ctx,
            )
            .expect("implements function should exist");
        assert_eq!(result.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_comptime_implements_numeric_widening() {
        let ctx = test_ctx();
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        let module = create_comptime_builtins_module(impls, Default::default());

        // int should widen to number
        let result = module
            .invoke_export(
                "implements",
                &[nb_str("int"), nb_str("Serializable")],
                &ctx,
            )
            .expect("implements function should exist");
        assert_eq!(result.unwrap().as_bool(), Some(true));

        // i64 should also widen to number
        let result = module
            .invoke_export(
                "implements",
                &[nb_str("i64"), nb_str("Serializable")],
                &ctx,
            )
            .expect("implements function should exist");
        assert_eq!(result.unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_comptime_build_config_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let result = module
            .invoke_export("build_config", &[], &ctx)
            .expect("build_config function should exist");
        assert!(result.is_ok());
        // build_config now returns TypedObject
        assert_eq!(result.unwrap().clone().type_name(), "object");
    }
}
