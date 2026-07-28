//! Comptime builtin functions.
//!
//! These functions are only callable inside `comptime { }` blocks.
//! They provide compile-time reflection, trait checking, and compiler messaging.
//!
//! Available builtins:
//! - `warning(msg)` — emits a compile-time warning
//! - `error(msg)` — emits a compile-time error
//! - `build_config()` — returns build-time configuration
//! - `type_ref(T)` / `type_category(T)` / `reflect(T)` / `find_impl(...)` —
//!   the typed reflection surface (registered in
//!   `register_frozen_reflection_builtins` / `trait_evidence`)

use shape_runtime::marshal::register_typed_fn_1;
use shape_runtime::module_exports::ModuleExports;
use shape_runtime::type_schema::typed_object_for_named_schema;
use shape_runtime::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectStorage};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{ELEM_TYPE_STRING, TypedArray, read_elem_type};
use shape_value::{KindedSlot, NativeKind};
// ADR-009 E2 #18 (slice 2): the typed `item_fn` carrier (E2-D10).
use super::comptime_fragments::CheckedItem;
// ADR-009 C3 #14 (slice 2): the public hook-template comptime API —
// `before_hook`/`after_hook` are PRODUCERS of the S1 `CheckedTemplate`
// carrier through its typestate chokepoint (never a second carrier), and
// `capture` lifts declared capture values through the S2 ConstLift seam.
use super::comptime_fragments::checked_template::{CheckedTemplate, CheckedTemplateBuilder, TemplateHookKind};
use super::template_specialization::const_lift::{self, LiftedConst};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

// ADR-009 (ticket C1, slice 1): THE one closure-capture selector. The two
// coupled vectors that used to drive capture emission (`mutable_flags` +
// `capture_kinds`) are fused here into a single `CapturePlan` producer, so the
// declared capture mode (slice 3) has exactly one place to enter.
pub(crate) mod capture_plan;
pub(crate) mod existential;
pub(crate) mod expansion_provenance;
pub(crate) mod semantic_freeze;
// ADR-009 (ticket B2, slice S3): opaque TraitRef/ImplRef carriers + the
// schema-name-checked evidence decode. New code lives in the submodule, not
// in this (already oversized) parent.
mod trait_evidence;
mod type_reflection;

pub(crate) use semantic_freeze::FreezeOverlay;
pub(crate) use type_reflection::{
    FrozenTypeCategory, FrozenTypeIdentity, build_frozen_type_category_heap_value,
    build_frozen_type_ref_heap_value, frozen_type_category_from_ref, frozen_type_from_ref,
};
// ADR-009 C3 S1c: the frozen callable-signature descriptor (payloads.rs:83),
// re-exported visibility-only so `template_specialization` can carry it for
// Sig IDENTITY/equality ONLY (slice-0 report §7.4 — never a type source).
pub(in crate::compiler) use type_reflection::payloads::CallableDescriptor;
// Test-only sibling: `template_specialization` tests fabricate descriptors
// per the `type_reflection/tests.rs:1779` pattern.
#[cfg(test)]
pub(in crate::compiler) use type_reflection::payloads::ParamDescriptor;
// ADR-009 §4.1 "one kind vocabulary" (ticket E5): the legacy `type_info`
// intrinsic + its `TypeKindLabel` / `build_type_info_heap_value` /
// `__ComptimeTypeInfo` carrier are DELETED. The typed reflection surface
// (`type_ref` / `type_category` / `reflect` / `find_impl`) is the only
// reflection vocabulary. Sentinel: `type_reflection/tests.rs::legacy_type_info_vocabulary_is_gone`.

pub(crate) const TYPE_REF_INTRINSIC: &str = "\u{1}comptime:type-ref";
pub(crate) const TYPE_CATEGORY_INTRINSIC: &str = "\u{1}comptime:type-category";
/// ADR-009 B1 S3: `reflect(TypeRef<T>) -> FrozenType<T>` intrinsic name.
/// Unspellable (SOH-prefixed) like its siblings; the spellable `reflect`
/// name reaches it only through the comptime forwarder (`comptime.rs`) —
/// the runtime `reflect` builtin mapping (`helpers.rs`) is untouched.
pub(crate) const REFLECT_INTRINSIC: &str = "\u{1}comptime:reflect";
/// ADR-009 B5 (Stage 2, Dec 56): `reflect_repr(TypeRef<T>,
/// RepresentationAccess<T>) -> FrozenType<T>` intrinsic name. Unspellable like
/// `reflect`; reached only through the comptime forwarder (`comptime.rs`).
pub(crate) const REFLECT_REPR_INTRINSIC: &str = "\u{1}comptime:reflect-repr";
/// ADR-009 B5 (Stage 2, Dec 56): the compiler-only capability mint. It is
/// SOH-prefixed AND is never registered as a spellable forwarder — the compiler
/// injects a call to it ONLY into an annotation expand-hook scope
/// (`functions_annotations.rs`), so user code can never obtain a
/// `RepresentationAccess<T>`.
pub(crate) const MINT_REPRESENTATION_ACCESS_INTRINSIC: &str =
    "\u{1}comptime:mint-representation-access";

// ADR-009 (ticket B2, slice S4): trait-identity + implementation-evidence
// intrinsics. Registered in `trait_evidence.rs`; the SOH prefix keeps them
// unspellable in Shape source (identity-literal transport only, Dec 49).
pub(crate) const TRAIT_REF_INTRINSIC: &str = "\u{1}comptime:trait-ref";
pub(crate) const FIND_IMPL_INTRINSIC: &str = "\u{1}comptime:find-impl";

// ADR-009 B4 (Stage 2, Dec 54): uniform nominal-application intrinsics. All
// SOH-prefixed (unspellable); reached only through the comptime forwarders /
// method-call site rewrite in `comptime.rs` (identity-literal transport).
pub(crate) const TYPE_CONSTRUCTOR_INTRINSIC: &str = "\u{1}comptime:type-constructor";
pub(crate) const CONST_ARG_INTRINSIC: &str = "\u{1}comptime:const-arg";
pub(crate) const APPLY_INTRINSIC: &str = "\u{1}comptime:apply";
pub(crate) const REFINE_INTRINSIC: &str = "\u{1}comptime:refine";
pub(crate) const TYPE_ARGUMENT_INTRINSIC: &str = "\u{1}comptime:type-argument";

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
    /// ADR-009 E2 #18: the `replace module` route. Its items arrive from a typed
    /// generation carrier (`item_fn` -> `__CheckedItem`) — no source/JSON string
    /// ever exists — and the module-target consumer routes them through
    /// `build_checked_module` (generated-provenance stamp + hygienic export
    /// reservation), producing a `comptime_fragments::CheckedModule`. (Slice 5
    /// deleted the legacy source-string `ReplaceModule` sibling this staged
    /// alongside; this is now the sole `replace module` directive.)
    ReplaceModuleChecked {
        items: Vec<shape_ast::ast::Item>,
    },
    /// §4.5.7: ADD generated items at the annotated item's module scope. Unlike
    /// `ReplaceModuleChecked` (which is only valid on a module target and replaces
    /// its body), `ExtendItems` is additive and valid on type/function/module
    /// targets: the parsed items are registered + compiled alongside the
    /// existing program.
    ExtendItems {
        items: Vec<shape_ast::ast::Item>,
    },
    /// ADR-009 C3 #14 (slice 2, S2b): install a constructed hook template onto
    /// the annotation's target function. `template_index` is the opaque
    /// `__CheckedTemplate` handle's index into the per-run
    /// `COMPTIME_HOOK_TEMPLATES` store (execute-populated; the store stays
    /// intact until the NEXT per-run clear). Fix-round-1: the pass-2 consumer
    /// SNAPSHOT-resolves every install index to its `BoundTemplate` at
    /// directive-loop ENTRY (`snapshot_install_hook_template_handles`),
    /// before any directive applies — a directive apply can trigger a NESTED
    /// handler run that clears + repopulates this store, so lazy
    /// per-directive resolution would cross store generations (see the
    /// store's lifecycle doc). The target is IMPLICIT (the annotation's
    /// target), matching every other directive. Consumed ONLY by the
    /// authoritative pass-2 function-target consumer
    /// (`process_comptime_directives_for_function`); the pre-pass consumers
    /// are documented no-ops (never double-install), and every non-function
    /// target consumer is a named rejection.
    InstallHookTemplate {
        template_index: usize,
    },
}

/// A non-fatal warning emitted from inside the comptime mini-VM by
/// `warning()` (comptime-excellence §4.4). Collected on a thread-local while
/// the block / handler runs, drained by the driver, and re-emitted by the
/// compiler with the driving construct's source span so `warning()` output is
/// spanned and LSDS-routed instead of a bare `eprintln!`.
#[derive(Debug, Clone)]
pub(crate) struct ComptimeDiagnostic {
    pub message: String,
}

thread_local! {
    static COMPTIME_DIRECTIVES: RefCell<Vec<ComptimeDirective>> = const { RefCell::new(Vec::new()) };
    static COMPTIME_DIAGNOSTICS: RefCell<Vec<ComptimeDiagnostic>> = const { RefCell::new(Vec::new()) };
    /// ADR-009 E2 #18 (slice 2): the `item_fn` typed-carrier store (E2-D10). A
    /// comptime builtin has no `&mut` compiler access, so `item_fn` cannot hand
    /// the driver an AST `Item` through a compiler table — it stashes the built
    /// `CheckedItem` here and returns a `__CheckedItem` handle carrying its
    /// INDEX; the consumer (`parse_extend_items_slot`, running inside
    /// `__emit_extend_items` / `__emit_replace_module` in the SAME comptime
    /// execution) resolves the index back to the item. Cleared before each
    /// handler run alongside `COMPTIME_DIRECTIVES` (`comptime.rs`), so indices
    /// start fresh per execution and never leak across runs; the read clones,
    /// leaving the store intact until that clear.
    static COMPTIME_CHECKED_ITEMS: RefCell<Vec<CheckedItem>> = const { RefCell::new(Vec::new()) };
    /// ADR-009 E2 #18 (slice 5, Part A): the block-form `replace body { ... }`
    /// typed-carrier store. Unlike `COMPTIME_CHECKED_ITEMS` (populated during VM
    /// EXECUTE by `item_fn`), a block-form body's statements are known at handler
    /// COMPILE time — `emit_comptime_replace_body_directive` stashes the
    /// `Vec<Statement>` here and emits `__emit_replace_body_checked(index)` (an
    /// int handle) instead of a JSON source string. So this store is CLEARED at
    /// `execute_comptime_with_annotation_handler` ENTRY (BEFORE the inner compile
    /// at `comptime.rs`), NOT at the pre-execute clear point where
    /// `COMPTIME_CHECKED_ITEMS` clears — clearing there would wipe the
    /// compile-populated stash before the VM reads it. Indices start fresh per
    /// handler run, so the pre-pass/pass-2 double-compile never leaks a stale
    /// body across runs; the read clones, leaving the store intact until the next
    /// per-run clear. Replaced the U03 JSON body transport (`parse_function_body_payload`),
    /// which slice 5 deleted whole.
    static COMPTIME_REPLACE_BODIES: RefCell<Vec<Vec<shape_ast::ast::Statement>>> =
        const { RefCell::new(Vec::new()) };
    /// ADR-009 E1 #17 (slice 3): the U01 literal-type carrier store. A literal
    /// `set param x: T` / `set return T` type is known at handler COMPILE time —
    /// `emit_comptime_set_param_type_directive` / `_set_return_type_directive`
    /// stash the typed `TypeAnnotation` here and emit the directive's INDEX (an
    /// int handle) instead of a `serialize_directive_payload` JSON round-trip.
    /// Same lifecycle as `COMPTIME_REPLACE_BODIES` (compile-populated ⇒ CLEARED
    /// at `execute_comptime_with_annotation_handler` ENTRY, before the inner
    /// compile, NOT at the pre-execute `COMPTIME_CHECKED_ITEMS` clear); indices
    /// start fresh per handler run so the pre-pass/pass-2 double-compile never
    /// leaks a stale type; the read clones, leaving the store intact until the
    /// next per-run clear. The U02 expr form (`set … (…type_ref)`) and the legacy
    /// JSON-string path are UNTOUCHED (slice 5 / slice 6 respectively).
    static COMPTIME_DIRECTIVE_TYPES: RefCell<Vec<shape_ast::ast::TypeAnnotation>> =
        const { RefCell::new(Vec::new()) };
    /// ADR-009 E1 #17 (slice 4): the direct-block `extend Type { … }` carrier
    /// store. The methods of a direct block are literal AST known at handler
    /// COMPILE time (like a `replace body { … }`), so this shares the
    /// `COMPTIME_REPLACE_BODIES` compile-populated lifecycle — CLEARED at
    /// `execute_comptime_with_annotation_handler` ENTRY (before the inner compile
    /// stashes into it), per-run indices, read clones. `emit_comptime_extend_directive`
    /// stashes the `ExtendStatement` here and emits `__emit_extend_checked(index)`
    /// instead of a `serialize_directive_payload` JSON round-trip. This is NOT a
    /// parallel extend-item carrier: the E2 COMPUTED-extend path
    /// (`__emit_extend_items` ← `parse_extend_items_slot` ← a `__CheckedItem`
    /// handle) is EXECUTE-populated with a different source and is untouched; only
    /// the `ComptimeDirective::Extend` transport moves off JSON.
    static COMPTIME_EXTEND_STATEMENTS: RefCell<Vec<shape_ast::ast::ExtendStatement>> =
        const { RefCell::new(Vec::new()) };
    /// ADR-009 C3 #14 (slice 2): the hook-template BODY-FN store. The
    /// `before_hook`/`after_hook` body argument is a bare module-scope fn
    /// identifier transported by an emit-side rewrite
    /// (`rewrite_template_hook_body_args`, comptime.rs — the type_ref/
    /// trait_ref identity-literal-transport precedent): the rewrite resolves
    /// the identifier against the compiler's AST fn table (threaded as a
    /// PARAMETER into `execute_comptime_with_annotation_handler`), stashes the
    /// `FunctionDef` here, and rewrites the arg to the index literal. So this
    /// store is COMPILE-populated (during the handler-body rewrite, BEFORE the
    /// inner compile) and shares the `COMPTIME_REPLACE_BODIES` lifecycle:
    /// CLEARED at `execute_comptime_with_annotation_handler` ENTRY, NOT at the
    /// pre-execute clear point where the execute-populated stores clear —
    /// clearing there would wipe the compile-time stash before the builtin
    /// reads it. Indices start fresh per handler run (pre-pass and pass-2 each
    /// index a fresh store); the read clones, leaving the store intact until
    /// the next per-run clear.
    static COMPTIME_TEMPLATE_BODY_FNS: RefCell<Vec<shape_ast::ast::FunctionDef>> =
        const { RefCell::new(Vec::new()) };
    /// ADR-009 C3 #14 (slice 2): the constructed hook-template store. The
    /// `before_hook`/`after_hook` builtins run the FULL `CheckedTemplate`
    /// typestate chokepoint EAGERLY at execute time and stash the resulting
    /// [`BoundTemplate`] (template + lifted capture values) here, returning a
    /// `__CheckedTemplate` handle carrying its INDEX (the E2 `__CheckedItem`
    /// opaque-index pattern). EXECUTE-populated ⇒ shares the
    /// `COMPTIME_CHECKED_ITEMS` lifecycle: cleared at the pre-execute clear
    /// point (`comptime.rs`, beside `clear_comptime_checked_items`) so indices
    /// are fresh per run; reads clone, and the store stays intact until the
    /// NEXT per-run clear. Fix-round-1 CAVEAT (why the pass-2 consumer
    /// snapshot-resolves): "the next per-run clear" can arrive DURING the
    /// current run's directive PROCESSING — applying a directive can trigger
    /// a nested annotation-handler run (a polymorphic template
    /// specialization's nested `compile_function` re-enters
    /// `execute_comptime_handlers`; an `ExtendItems` compile does the same),
    /// which clears + repopulates this store. So the driver resolves EVERY
    /// install handle at directive-loop entry
    /// (`snapshot_install_hook_template_handles`, install_registry.rs) —
    /// the same value-snapshot discipline `take_comptime_directives` applies
    /// to the directive buffer — and never reads this store lazily
    /// per-directive.
    static COMPTIME_HOOK_TEMPLATES: RefCell<Vec<BoundTemplate>> = const { RefCell::new(Vec::new()) };
    /// ADR-009 C3 #14 (slice 2; S3b compositional): the capture-binding
    /// store. `capture(name, value)` lifts the value through the ConstLift
    /// seam (`const_lift::lift_capture_value` — since S3b the full C3-G5
    /// compositional domain) and stashes `(name, LiftedConst)` here,
    /// returning a `__CaptureBinding` handle carrying its INDEX.
    /// EXECUTE-populated ⇒ same lifecycle as `COMPTIME_HOOK_TEMPLATES` /
    /// `COMPTIME_CHECKED_ITEMS`: cleared at the pre-execute clear point,
    /// fresh indices per run, reads clone.
    static COMPTIME_CAPTURE_BINDINGS: RefCell<Vec<(String, LiftedConst)>> =
        const { RefCell::new(Vec::new()) };
    /// True while the §4.5.1 whole-program pre-pass speculatively runs a
    /// type-target comptime handler to materialize generated function
    /// signatures. The pre-pass is not the authoritative run — pass-2 re-runs
    /// the same handler while compiling the annotated type. So any raw
    /// side-effecting output (`print`) the handler produces during the pre-pass
    /// must be discarded, exactly as `warning()`/`error()` diagnostics drained
    /// here are discarded by the pre-pass; otherwise a handler that prints
    /// would emit its output twice. Set only around the pre-pass handler
    /// invocation; the authoritative pass-2 run leaves it clear so the handler
    /// prints exactly once.
    static COMPTIME_OUTPUT_SUPPRESSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set the comptime speculative-output suppression flag (§4.5.1 pre-pass).
/// Returns the previous value so the caller can restore it.
pub(crate) fn set_comptime_output_suppressed(suppressed: bool) -> bool {
    COMPTIME_OUTPUT_SUPPRESSED.with(|c| c.replace(suppressed))
}

/// True while the §4.5.1 pre-pass is speculatively running a comptime handler;
/// consulted by `builtin_print` to discard speculative output.
pub fn is_comptime_output_suppressed() -> bool {
    COMPTIME_OUTPUT_SUPPRESSED.with(|c| c.get())
}

/// Clear the thread-local comptime-diagnostics buffer before a comptime run.
pub(crate) fn clear_comptime_diagnostics() {
    COMPTIME_DIAGNOSTICS.with(|d| d.borrow_mut().clear());
}

/// Drain the comptime-diagnostics collected during the last comptime run.
pub(crate) fn take_comptime_diagnostics() -> Vec<ComptimeDiagnostic> {
    COMPTIME_DIAGNOSTICS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

fn push_comptime_diagnostic(diag: ComptimeDiagnostic) {
    COMPTIME_DIAGNOSTICS.with(|d| d.borrow_mut().push(diag));
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

/// ADR-009 E2 #18 (slice 2): clear the `item_fn` carrier store before a comptime
/// run (called from `comptime.rs` alongside `clear_comptime_directives`), so
/// each execution's handles index a fresh store.
pub(crate) fn clear_comptime_checked_items() {
    COMPTIME_CHECKED_ITEMS.with(|items| items.borrow_mut().clear());
}

/// Stash a built `CheckedItem` and return the index the `__CheckedItem` handle
/// carries. The index refers to the CURRENT execution's store (cleared per run),
/// and the returned index is exactly the just-pushed slot, so the handle and the
/// item stay consistent regardless of how many `item_fn` calls precede it.
fn push_comptime_checked_item(item: CheckedItem) -> usize {
    COMPTIME_CHECKED_ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        items.push(item);
        items.len() - 1
    })
}

/// Resolve a `__CheckedItem` handle's index back to its `CheckedItem` (cloned;
/// the store stays intact until the next per-run clear).
fn comptime_checked_item_at(index: usize) -> Option<CheckedItem> {
    COMPTIME_CHECKED_ITEMS.with(|items| items.borrow().get(index).cloned())
}

/// ADR-009 E2 #18 (slice 5, Part A): clear the block-form `replace body` carrier
/// store at `execute_comptime_with_annotation_handler` ENTRY — BEFORE the handler
/// is compiled (its `replace body { ... }` statements stash here during that
/// compile) and thus BEFORE its VM run reads them. This is deliberately NOT the
/// pre-execute clear point where `clear_comptime_checked_items` runs (see the
/// store's declaration): the body stash is compile-populated, so clearing it
/// pre-execute would wipe it before the read.
pub(crate) fn clear_comptime_replace_bodies() {
    COMPTIME_REPLACE_BODIES.with(|bodies| bodies.borrow_mut().clear());
}

/// Stash a block-form replacement body and return the index the
/// `__emit_replace_body_checked(index)` call carries. Called from the emit side
/// (`emit_comptime_replace_body_directive`) at handler-compile. The index refers
/// to the CURRENT handler run's store (cleared at that run's entry), so pre-pass
/// and pass-2 each index a fresh store and never read a stale body.
pub(crate) fn push_comptime_replace_body(body: Vec<shape_ast::ast::Statement>) -> usize {
    COMPTIME_REPLACE_BODIES.with(|bodies| {
        let mut bodies = bodies.borrow_mut();
        bodies.push(body);
        bodies.len() - 1
    })
}

/// Resolve a `__emit_replace_body_checked` index back to its stashed body
/// (cloned; the store stays intact until the next per-run clear).
fn comptime_replace_body_at(index: usize) -> Option<Vec<shape_ast::ast::Statement>> {
    COMPTIME_REPLACE_BODIES.with(|bodies| bodies.borrow().get(index).cloned())
}

/// ADR-009 E1 #17 (slice 3): clear the U01 literal-type carrier store at handler
/// run ENTRY — same compile-populated lifecycle as
/// [`clear_comptime_replace_bodies`] (cleared BEFORE the inner compile that
/// stashes here, NOT at the pre-execute clear point).
pub(crate) fn clear_comptime_directive_types() {
    COMPTIME_DIRECTIVE_TYPES.with(|types| types.borrow_mut().clear());
}

/// Stash a literal directive type and return the index the
/// `__emit_set_param_type` / `__emit_set_return_type` call carries. Called from
/// the emit side at handler-compile; the index refers to the CURRENT handler
/// run's store (cleared at that run's entry).
pub(crate) fn push_comptime_directive_type(annotation: shape_ast::ast::TypeAnnotation) -> usize {
    COMPTIME_DIRECTIVE_TYPES.with(|types| {
        let mut types = types.borrow_mut();
        types.push(annotation);
        types.len() - 1
    })
}

/// Resolve a literal-type directive index back to its stashed `TypeAnnotation`
/// (cloned; the store stays intact until the next per-run clear).
fn comptime_directive_type_at(index: usize) -> Option<shape_ast::ast::TypeAnnotation> {
    COMPTIME_DIRECTIVE_TYPES.with(|types| types.borrow().get(index).cloned())
}

/// ADR-009 E1 #17 (slice 4): clear the direct-block extend carrier store at
/// handler run ENTRY — same compile-populated lifecycle as
/// [`clear_comptime_replace_bodies`] / [`clear_comptime_directive_types`].
pub(crate) fn clear_comptime_extend_statements() {
    COMPTIME_EXTEND_STATEMENTS.with(|extends| extends.borrow_mut().clear());
}

/// Stash a direct-block `extend Type { … }` statement and return the index the
/// `__emit_extend_checked(index)` call carries. Called from the emit side at
/// handler-compile; the index refers to the CURRENT handler run's store.
pub(crate) fn push_comptime_extend_statement(extend: shape_ast::ast::ExtendStatement) -> usize {
    COMPTIME_EXTEND_STATEMENTS.with(|extends| {
        let mut extends = extends.borrow_mut();
        extends.push(extend);
        extends.len() - 1
    })
}

/// Resolve an extend-carrier index back to its stashed `ExtendStatement`
/// (cloned; the store stays intact until the next per-run clear).
fn comptime_extend_statement_at(index: usize) -> Option<shape_ast::ast::ExtendStatement> {
    COMPTIME_EXTEND_STATEMENTS.with(|extends| extends.borrow().get(index).cloned())
}

/// ADR-009 C3 #14 (slice 2): a constructed hook template BOUND to its lifted
/// capture values — `CheckedTemplate` (the ONE S1 carrier, produced through
/// its typestate chokepoint) + the `(name, LiftedConst)` values `capture()`
/// lifted through the ConstLift seam (the full C3-G5 compositional domain
/// since S3b). Lives beside the template store (`COMPTIME_HOOK_TEMPLATES`);
/// the S2b install consumer resolves a `__CheckedTemplate` handle's index
/// back to this pair and feeds the values into `specialize_template` — the
/// rule-6 identity + the const_lift BAKE (S3b; no call-site delivery).
#[derive(Debug, Clone)]
pub(in crate::compiler) struct BoundTemplate {
    /// The checked template (construction-validated: classification, capture
    /// bijection, pseudo-tuple uses).
    pub(in crate::compiler) template: CheckedTemplate,
    /// The lifted capture values, validated against the template's declared
    /// trailing capture-parameter types
    /// (`const_lift::validate_capture_value_types`).
    pub(in crate::compiler) capture_values: Vec<(String, LiftedConst)>,
}

/// ADR-009 C3 #14 (slice 2): clear the hook-template body-fn store at
/// `execute_comptime_with_annotation_handler` ENTRY — BEFORE the handler-body
/// rewrite stashes into it and thus BEFORE its VM run reads it by index. Same
/// compile-populated lifecycle as [`clear_comptime_replace_bodies`] (cleared at
/// run entry, NOT at the pre-execute clear point — a pre-execute clear would
/// wipe the compile-time stash).
pub(crate) fn clear_comptime_template_body_fns() {
    COMPTIME_TEMPLATE_BODY_FNS.with(|fns| fns.borrow_mut().clear());
}

/// Stash a resolved template body `FunctionDef` and return the index the
/// rewritten `before_hook`/`after_hook` body argument carries. Called from the
/// emit-side rewrite (`rewrite_template_hook_body_args`, comptime.rs) at
/// handler-body rewrite time; the index refers to the CURRENT handler run's
/// store (cleared at that run's entry), and the returned index is exactly the
/// just-pushed slot.
pub(crate) fn push_comptime_template_body_fn(def: shape_ast::ast::FunctionDef) -> usize {
    COMPTIME_TEMPLATE_BODY_FNS.with(|fns| {
        let mut fns = fns.borrow_mut();
        fns.push(def);
        fns.len() - 1
    })
}

/// Resolve a template-body index back to its stashed `FunctionDef` (cloned;
/// the store stays intact until the next per-run clear).
fn comptime_template_body_fn_at(index: usize) -> Option<shape_ast::ast::FunctionDef> {
    COMPTIME_TEMPLATE_BODY_FNS.with(|fns| fns.borrow().get(index).cloned())
}

/// ADR-009 C3 #14 (slice 2): clear the constructed hook-template store before
/// a comptime run (called from `comptime.rs` beside
/// `clear_comptime_checked_items` — the execute-populated clear point), so
/// each execution's `__CheckedTemplate` handles index a fresh store.
pub(crate) fn clear_comptime_hook_templates() {
    COMPTIME_HOOK_TEMPLATES.with(|templates| templates.borrow_mut().clear());
}

/// Stash a constructed [`BoundTemplate`] and return the index the
/// `__CheckedTemplate` handle carries (exactly the just-pushed slot).
fn push_comptime_hook_template(template: BoundTemplate) -> usize {
    COMPTIME_HOOK_TEMPLATES.with(|templates| {
        let mut templates = templates.borrow_mut();
        templates.push(template);
        templates.len() - 1
    })
}

/// Resolve a `__CheckedTemplate` handle's index back to its [`BoundTemplate`]
/// (cloned; the store stays intact until the next per-run clear — which can
/// arrive DURING directive processing via a nested handler run, so the S2b
/// install consumer SNAPSHOT-resolves all indices at directive-loop entry,
/// fix-round-1: `snapshot_install_hook_template_handles`).
pub(in crate::compiler) fn comptime_hook_template_at(index: usize) -> Option<BoundTemplate> {
    COMPTIME_HOOK_TEMPLATES.with(|templates| templates.borrow().get(index).cloned())
}

/// ADR-009 C3 #14 (slice 2): clear the capture-binding store before a comptime
/// run (beside [`clear_comptime_hook_templates`] at the pre-execute clear
/// point), so each execution's `__CaptureBinding` handles index a fresh store.
pub(crate) fn clear_comptime_capture_bindings() {
    COMPTIME_CAPTURE_BINDINGS.with(|bindings| bindings.borrow_mut().clear());
}

/// Stash a lifted capture binding and return the index the `__CaptureBinding`
/// handle carries (exactly the just-pushed slot).
fn push_comptime_capture_binding(binding: (String, LiftedConst)) -> usize {
    COMPTIME_CAPTURE_BINDINGS.with(|bindings| {
        let mut bindings = bindings.borrow_mut();
        bindings.push(binding);
        bindings.len() - 1
    })
}

/// Resolve a `__CaptureBinding` handle's index back to its `(name,
/// LiftedConst)` pair (cloned; the store stays intact until the next per-run
/// clear).
fn comptime_capture_binding_at(index: usize) -> Option<(String, LiftedConst)> {
    COMPTIME_CAPTURE_BINDINGS.with(|bindings| bindings.borrow().get(index).cloned())
}

fn parse_type_annotation_payload(payload: &str) -> Result<shape_ast::ast::TypeAnnotation, String> {
    // ADR-009 #88 (PRESERVED, NOT the deleted `.source` fallback): the SANCTIONED
    // parser for the comptime item-generation API — `item_fn` / `extend_method`
    // (`item_fn(name, return_type: string | TypeRef, value)`), whose `string` half
    // is documented contract and has no Int64/TypeRef alternative today. A string
    // TYPE SPELLING inherently requires this parse to become a `TypeAnnotation`
    // (there is no non-parse path from "Array<int>" to an AST). This is a LIVE,
    // sanctioned carrier reached ONLY from the bare-string arm of
    // `type_annotation_from_string_or_type_ref_slot`; it is DISTINCT from the
    // `__ComptimeTypeRef.source` reparse fallback, which was DELETED at E5 CKPT-5.
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

fn string_field_from_typed_object(
    storage: &TypedObjectStorage,
    schema: &shape_runtime::type_schema::TypeSchema,
    field_name: &str,
) -> Result<String, String> {
    let slot = field_slot_from_typed_object(storage, schema, field_name)?;
    slot.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("field '{}.{}' is not a string", schema.name, field_name))
}

fn field_slot_from_typed_object(
    storage: &TypedObjectStorage,
    schema: &shape_runtime::type_schema::TypeSchema,
    field_name: &str,
) -> Result<KindedSlot, String> {
    let idx = schema
        .fields
        .iter()
        .find(|field| field.name == field_name)
        .map(|field| field.index as usize)
        .ok_or_else(|| {
            format!(
                "schema '{}' does not expose expected string field '{}'",
                schema.name, field_name
            )
        })?;
    storage
        .clone_field_kinded(idx)
        .ok_or_else(|| format!("field '{}' is missing from '{}'", field_name, schema.name))
}

fn type_annotation_from_string_or_type_ref_slot(
    slot: &KindedSlot,
    builtin_name: &str,
    overlay: &FreezeOverlay,
) -> Result<shape_ast::ast::TypeAnnotation, String> {
    // ADR-009 E1 #17 (slice 3, U01): a literal type is carried as an INDEX into
    // the per-run typed store (`COMPTIME_DIRECTIVE_TYPES`) — no JSON, no reparse.
    // Its `Int64` kind is disjoint from the legacy JSON-string payload (`String`)
    // and the U02 `__ComptimeTypeRef` object (`Ptr(TypedObject)`), so this branch
    // never shadows those paths.
    if slot.kind() == NativeKind::Int64 {
        let index = slot.as_i64().ok_or_else(|| {
            format!("{builtin_name} expects a non-null literal-type carrier index")
        })?;
        return comptime_directive_type_at(index as usize).ok_or_else(|| {
            format!("{builtin_name}: no stored literal directive type at index {index}")
        });
    }
    if let Some(payload) = slot.as_str() {
        // ADR-009 E5 CKPT-4 — design §2 class E ("reject the bare-string arm
        // loud") is BLOCKED and SURFACED, NOT applied. This arm is a SANCTIONED,
        // documented carrier for `item_fn` / `extend_method` (`item_fn(name,
        // return_type: string | TypeRef, value)`, comptime_builtins.rs — the
        // `string` half is contract). Those item-generation builtins have NO
        // sanctioned Int64/TypeRef alternative today, and a string TYPE SPELLING
        // inherently requires `parse_type_annotation_payload` to become a
        // `TypeAnnotation` (there is no non-parse path from "Array<int>" to an
        // AST). So the design's class-E reject would break item_fn/extend's
        // documented contract (~19 tests) with no additive migration — the exact
        // "migrating a class needs more than threading overlay+ASTs → SURFACE it,
        // don't force it" case. The string arm therefore SURVIVES CKPT-4 as a live
        // reader; closing it (the full exit criterion + the CKPT-5 string-arm
        // deletion) is BLOCKED on an item_fn/extend typed-carrier migration
        // decision. See e5-decisions.md CKPT-4 §"class-E blocker".
        return parse_type_annotation_payload(payload);
    }
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err(format!(
            "{builtin_name} expects a string type payload or __ComptimeTypeRef value, got {:?}",
            slot.kind()
        ));
    }
    let bits = slot.raw();
    if bits == 0 {
        return Err(format!(
            "{builtin_name} expects a non-null __ComptimeTypeRef value"
        ));
    }
    // SAFETY: the NativeKind witness proves the slot bits are a live
    // TypedObjectStorage pointer for the duration of this builtin invocation.
    let storage = unsafe { &*(bits as *const TypedObjectStorage) };
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "{builtin_name} could not resolve typed-object schema id {}",
            storage.schema_id
        )
    })?;
    if schema.name != "__ComptimeTypeRef" {
        return Err(format!(
            "{builtin_name} expects __ComptimeTypeRef, got '{}'",
            schema.name
        ));
    }
    // ADR-009 E1 #17 (slice 5, A-FULL) — E1-D7(a) STAMPED->IDENTITY-ONLY, E5
    // CKPT-5 FALLBACK DELETED: a type_ref carrying a real frozen identity resolves
    // via the ONE inverse of the semantic-freeze composite algebra
    // (`reconstruct_type_annotation`) — the ONLY resolution route. There is NO
    // `.source` field and NO reparse fallback arm (both DELETED at E5 CKPT-5): a
    // stamped-but-unresolvable identity is a NAMED `ShapeError::SemanticError`
    // (surfaced here as its `String` at the consumer ABI boundary), and an
    // UNSTAMPED ref (identity == INVALID) rejects LOUD via the INVALID arm below.
    // Neither can silently reparse a spelling, because the reparse arm no longer
    // exists — the canonical stamped->reparse walk-back is structurally
    // impossible. The identity halves are read with the same `get_field` ->
    // `clone_field_kinded` -> `as_i64` shape as `frozen_identity_from_ref`
    // (type_reflection.rs) — an existing sanctioned read of the sibling schema,
    // not a new decode path.
    let identity_field = |name: &str| -> Result<i64, String> {
        let field = schema
            .get_field(name)
            .ok_or_else(|| format!("__ComptimeTypeRef schema has no {name} field"))?;
        storage
            .clone_field_kinded(field.index as usize)
            .and_then(|value| value.as_i64())
            .ok_or_else(|| format!("__ComptimeTypeRef {name} is not an integer"))
    };
    let identity = FrozenTypeIdentity {
        high: identity_field("identity_high")?,
        low: identity_field("identity_low")?,
    };
    // ADR-009 E5 CKPT-4 (design §2 class C + A5 — RULED NAMED SURFACE-AND-STOP),
    // E5 CKPT-5 (the `.source` reparse fallback DELETED). An INVALID identity means
    // the producer could NOT stamp a reconstructable identity for this type_ref:
    // the type is genuinely not reconstructable — a function with no declared
    // return (`kind: "Unresolved"`, the design-named class-C discriminator), a
    // synthetic module member, a scoped generic parameter, or an un-applied generic
    // head. There is NO concrete type to emit. Reject LOUD; there is NO `.source`
    // field and NO reparse arm to fall back to (both deleted at CKPT-5), so the
    // Forbidden-Patterns dynamic-reparse walk-back is structurally impossible.
    if identity == FrozenTypeIdentity::INVALID {
        let kind = string_field_from_typed_object(storage, &schema, "kind").unwrap_or_default();
        let name = string_field_from_typed_object(storage, &schema, "name").unwrap_or_default();
        return Err(format!(
            "{builtin_name}: type reference '{name}' (kind='{kind}') carries no \
             reconstructable semantic identity — there is no concrete type to emit. A \
             comptime handler cannot resolve an unstamped/unresolvable type_ref (an \
             unresolved return, a synthetic member, a scoped generic parameter, or an \
             un-applied generic head); provide a concrete, reconstructable type."
        ));
    }
    // ADR-009 E5 CKPT-5: identity is guaranteed != INVALID here (the INVALID arm
    // above returned). A stamped `__ComptimeTypeRef` resolves ONLY through the ONE
    // inverse of the semantic-freeze composite algebra — there is NO `.source`
    // field to read and NO reparse fallback (both DELETED this checkpoint), so the
    // stamped->reparse walk-back cannot be written.
    reconstruct_type_annotation(overlay, identity).map_err(|e| e.to_string())
}

/// ADR-009 E1 #17 (slice 5, A-FULL): the ONE total inverse of the semantic
/// freeze's composite algebra — a frozen `FrozenTypeIdentity` back to the AST
/// `TypeAnnotation` it canonicalized from. This is the SOLE resolution route for a
/// stamped `__ComptimeTypeRef`; the `.source` reparse fallback was DELETED at E5
/// CKPT-5, so there is no spelling to reparse. This is the E1-D7(c) totality
/// obligation: every
/// one of the 10 `FrozenPayloadDescriptor` variants either reconstructs
/// structurally or returns a NAMED `ShapeError::SemanticError` — there is no
/// catch-all silent arm, and an unresolvable sub-identity surfaces the freeze
/// query's own named rejection through `?`.
///
/// - Primitive spellings invert the ONE `PRIMITIVE_SYNONYM_FAMILIES` table via
///   [`type_reflection::canonical_primitive_spelling`] (names[0] canonical) — no
///   second name table (E1-D7(c)).
/// - Reference is the structural inverse of the canonicalizer's `Borrow` arm
///   (`&T` / `&mut T`, type_reflection.rs); Callable re-applies each parameter's
///   `PassingMode` borrow around its reconstructed VALUE type, the exact inverse
///   of the canonicalizer's `Function` arm (the mode axis the identity hash
///   factors out).
/// - Applied nominals (`Array<int>`, `Option<T>`, `HashMap<K,V>`, `Result<T,E>`,
///   applied user structs/enums) and bare user nominals SPELL directly off the
///   frozen memo (ADR-009 E5 CKPT-1, design §1a — the early arm below): an
///   applied form recurses its ordered argument identities under the
///   identity-indirected recursion invariant (A2, no eager nested expansion), a
///   bare user nominal spells as its `Basic(name)`. Un-applied generic HEADS
///   (bare `Array`) stay the `payload_of` un-applied-head NAMED rejection (A3);
///   Records (field names one-way-hashed into hygienic member identities) and
///   scoped generic Parameters stay their distinct NAMED rejections — record
///   field-name preservation is CKPT-3, out of CKPT-1.
///
/// STAGE 4 (LIVE) + E5 CKPT-5 (fallback deleted): the consumer flip wired this
/// into the live directive resolver — `type_annotation_from_string_or_type_ref_slot`
/// resolves a stamped `__ComptimeTypeRef` through THIS inverse, identity-only. The
/// `.source` field + reparse arm are DELETED, so this is the ONLY route; the
/// producer stamp-gate (stage 3) admits an identity iff
/// `reconstruct_type_annotation(...).is_ok()`, so producer and consumer share ONE
/// code path (E1-D7(b)).
pub(crate) fn reconstruct_type_annotation(
    overlay: &FreezeOverlay,
    identity: FrozenTypeIdentity,
) -> Result<shape_ast::ast::TypeAnnotation, shape_ast::error::ShapeError> {
    use shape_ast::ast::{FunctionParam, ObjectTypeField, TypeAnnotation, TypePath};
    use shape_runtime::comptime_reflection::PassingMode;
    use type_reflection::payloads::FrozenPayloadDescriptor;

    fn named(message: String) -> shape_ast::error::ShapeError {
        shape_ast::error::ShapeError::SemanticError {
            message,
            location: None,
        }
    }

    // ADR-009 E5 CKPT-1 (design §1a) — SPELLING reconstruction. BEFORE the
    // descriptor-driven arms below, spell an APPLIED nominal or a bare user
    // nominal DIRECTLY from the frozen identities the semantic-freeze memo
    // already derived (`applied_nominal_of` / `bare_nominal_name_of`) — a read
    // of DERIVED facts, NEVER a `.source` reparse and never a fabricated
    // identity. This is the additive step that AUTO-WIDENS the shared stamp-gate:
    // `stamp_for` (comptime_target.rs) admits an identity iff
    // `reconstruct_type_annotation(...).is_ok()`, so the moment this arm
    // reconstructs `Array<int>` the SAME predicate stamps it — producers stop
    // emitting INVALID and the consumer stops hitting `.source` (E1-D7(b),
    // one code path; no `stamp_for` edit).
    //
    // A2 INVARIANT — identity-indirected recursion, NO eager nested expansion:
    // an applied form spells its HEAD name then RECURSES on its ordered
    // `arg_identities`; a bare-nominal argument is a LEAF spelled by name (never
    // field-expanded). So a recursive nominal — `type Tree { kids: Array<Tree> }`
    // spelled as `Array<Tree>` — spells its head + `Tree` (a bare name) and
    // TERMINATES; it never descends into `Tree`'s fields. Nested APPLIED args
    // (`Array<Option<int>>`) terminate because `arg_identities` is the finite,
    // content-derived decomposition the freeze memo interned for every
    // sub-expression (projection.rs CKPT-1 recursive memoization), not an
    // unbounded re-derivation.
    if let Some(applied) = overlay.applied_nominal_of(identity) {
        let head = overlay
            .type_names_for_identity(applied.head_identity)
            .first()
            .map(|name| (*name).to_string())
            .ok_or_else(|| {
                named(
                    "reconstruct_type_annotation: an applied nominal's head identity has no \
                     frozen spellable name"
                        .to_string(),
                )
            })?;
        let mut args = Vec::with_capacity(applied.arg_identities.len());
        for arg in applied.arg_identities {
            args.push(reconstruct_type_annotation(overlay, arg)?);
        }
        return Ok(TypeAnnotation::Generic {
            name: TypePath::simple(head),
            args,
        });
    }
    if let Some(name) = overlay.bare_nominal_name_of(identity) {
        return Ok(TypeAnnotation::Basic(name));
    }

    // The `?` here converts the freeze query's OWN named rejection (unknown
    // identity, applied-nominal-pending, un-applied-head, bounded-erased, …)
    // into a named SemanticError — this is the total-coverage boundary for
    // every identity that is NOT reconstructable (E1-D7(c)).
    let payload = overlay.payload_of(identity).map_err(named)?;

    match payload {
        FrozenPayloadDescriptor::Primitive(primitive) => {
            type_reflection::canonical_primitive_spelling(primitive)
                .map(|spelling| TypeAnnotation::Basic(spelling.to_string()))
                .ok_or_else(|| {
                    named(format!(
                        "reconstruct_type_annotation: no canonical spelling for frozen \
                         primitive {primitive:?}"
                    ))
                })
        }
        FrozenPayloadDescriptor::Never => Ok(TypeAnnotation::Never),
        FrozenPayloadDescriptor::Erased { bounds } => {
            if bounds.is_empty() {
                Ok(TypeAnnotation::Basic("any".to_string()))
            } else {
                Err(named(
                    type_reflection::payloads::bounded_erased_payload_rejection(),
                ))
            }
        }
        FrozenPayloadDescriptor::Tuple(descriptor) => {
            let mut elements = Vec::with_capacity(descriptor.elements.len());
            for element in descriptor.elements {
                elements.push(reconstruct_type_annotation(overlay, element)?);
            }
            Ok(TypeAnnotation::Tuple(elements))
        }
        FrozenPayloadDescriptor::Reference(descriptor) => {
            let inner = reconstruct_type_annotation(overlay, descriptor.referent)?;
            Ok(TypeAnnotation::Borrow {
                mutable: descriptor.mutable,
                inner: Box::new(inner),
            })
        }
        FrozenPayloadDescriptor::Union(descriptor) => {
            let mut members = Vec::with_capacity(descriptor.members.len());
            for member in descriptor.members {
                members.push(reconstruct_type_annotation(overlay, member)?);
            }
            Ok(TypeAnnotation::Union(members))
        }
        FrozenPayloadDescriptor::Callable(descriptor) => {
            let mut params = Vec::with_capacity(descriptor.params.len());
            for param in descriptor.params {
                // The canonicalizer factored the ADR mode axis OUT of the
                // borrow wrapper (PassingMode) and recorded the VALUE type; the
                // structural inverse re-applies the borrow around it.
                let value = reconstruct_type_annotation(overlay, param.type_identity)?;
                let type_annotation = match param.mode {
                    PassingMode::Move => value,
                    PassingMode::SharedBorrow => TypeAnnotation::Borrow {
                        mutable: false,
                        inner: Box::new(value),
                    },
                    PassingMode::ExclusiveBorrow => TypeAnnotation::Borrow {
                        mutable: true,
                        inner: Box::new(value),
                    },
                };
                params.push(FunctionParam {
                    name: param.name,
                    optional: param.optional,
                    type_annotation,
                });
            }
            let returns = reconstruct_type_annotation(overlay, descriptor.returns)?;
            Ok(TypeAnnotation::Function {
                params,
                returns: Box::new(returns),
                effects: None,
            })
        }
        // Post-CKPT-1 this arm is the residual DEFENSIVE case only: an applied
        // nominal is spelled by the early `applied_nominal_of` arm above, and a
        // resolved bare user nominal by the `bare_nominal_name_of` arm — both
        // return before `payload_of` is consulted. A `Nominal` descriptor that
        // reaches here is a nominal whose head has no frozen spellable name
        // (would only occur if `type_names_for_identity` were empty) — a NAMED
        // rejection, never a `.source` fallback.
        FrozenPayloadDescriptor::Nominal(_) => Err(named(
            "reconstruct_type_annotation: a nominal declaration shape reached the \
             descriptor arm without a frozen spellable head name and carries no \
             spellable type_ref target"
                .to_string(),
        )),
        // ADR-009 E5 CKPT-3 (B2 in-scope): SPELL the structural record back to
        // `{name: T, …}` from the field NAMES preserved as the spell/reflect-only
        // `RecordFieldDescriptor.name` freeze fact (the record IDENTITY + member
        // strings stay byte-identical — CKPT-0 binding invariant). Optionality is
        // record-identity-significant (`{x?:int} != {x:int}`), so it is preserved
        // per field. A2 identity-indirected: each field type RECURSES on its own
        // frozen `type_identity` (a bare/applied nominal arg is spelled by name,
        // never field-expanded), so a record with a record/applied field type
        // TERMINATES on the finite type expression — no eager expansion. This
        // AUTO-WIDENS the shared stamp-gate for records (the SAME
        // `reconstruct(...).is_ok()` predicate stamps them, so producers stop
        // emitting INVALID + the consumer stops hitting `.source`). The named
        // rejection below stays the LOUD fallback for a record that genuinely
        // cannot spell (never a silent gap).
        FrozenPayloadDescriptor::Record(descriptor) => {
            let mut fields = Vec::with_capacity(descriptor.fields.len());
            for field in descriptor.fields {
                let type_annotation = reconstruct_type_annotation(overlay, field.type_identity)?;
                fields.push(ObjectTypeField {
                    name: field.name,
                    optional: field.optional,
                    type_annotation,
                    annotations: Vec::new(),
                });
            }
            Ok(TypeAnnotation::Object(fields))
        }
        FrozenPayloadDescriptor::Parameter(_) => Err(named(
            "reconstruct_type_annotation: a scoped generic parameter is not a spellable \
             type_ref target this slice; the consumer rejects an unstamped/unresolvable \
             ref LOUD (no `.source` reparse — the fallback field + arm are deleted, E5 CKPT-5)"
                .to_string(),
        )),
    }
}

fn heap_value_from_typed_object_slot(kinded: KindedSlot) -> HeapValue {
    let ptr = kinded.raw() as *const shape_value::heap_value::TypedObjectStorage;
    // SAFETY: `typed_object_for_named_schema` returns a live TypedObjectStorage
    // pointer with at least one refcount share owned by `kinded`.
    unsafe {
        shape_value::v2::refcount::v2_retain(&(*ptr).header);
    }
    drop(kinded);
    HeapValue::TypedObject(shape_value::heap_value::TypedObjectPtr::new(ptr))
}

fn is_valid_generated_function_name(name: &str) -> bool {
    const SHAPE_IDENT_KEYWORDS: &[&str] = &[
        "pub",
        "import",
        "from",
        "use",
        "as",
        "builtin",
        "let",
        "var",
        "const",
        "mut",
        "function",
        "async",
        "await",
        "if",
        "else",
        "for",
        "while",
        "match",
        "return",
        "break",
        "continue",
        "true",
        "false",
        "null",
        "None",
        "Some",
        "and",
        "or",
        "type",
        "trait",
        "interface",
        "impl",
        "enum",
        "extend",
        "method",
        "in",
        "comptime",
        "datasource",
    ];
    if SHAPE_IDENT_KEYWORDS.contains(&name) {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// ADR-009 E2 #18 (slice 2): a literal value slot -> an `Expr::Literal`,
/// DIRECTLY. No `literal_kind` discriminator, no parallel sentinel fields: the
/// slot's runtime kind selects the literal. (Slice 5 deleted the
/// `__ComptimeItemFragment` sentinel encode/decode this superseded.)
fn literal_expr_from_slot(slot: &KindedSlot, builtin_name: &str) -> Result<shape_ast::ast::Expr, String> {
    use shape_ast::ast::{Expr, Literal, Span};

    let literal = if let Some(value) = slot.as_str() {
        Literal::String(value.to_string())
    } else if let Some(value) = slot.as_i64() {
        Literal::Int(value)
    } else if let Some(value) = slot.as_f64() {
        if !value.is_finite() {
            return Err(format!(
                "{builtin_name} only supports finite numeric literal return values"
            ));
        }
        Literal::Number(value)
    } else if let Some(value) = slot.as_bool() {
        Literal::Bool(value)
    } else {
        return Err(format!(
            "{builtin_name} only supports string, int, number, or bool literal return values; got {:?}",
            slot.kind()
        ));
    };
    Ok(Expr::Literal(literal, Span::default()))
}

/// ADR-009 E2 #18 (slice 2): build the generated free-function `Item` DIRECTLY
/// from `item_fn`'s raw args (E2-D10). The typed return comes from the raw
/// return-type slot and the body from the value slot; no sentinel fields and no
/// source/JSON string participate. (Slice 5 deleted the
/// `build_function_item_fragment` -> `function_item_from_fragment` sentinel
/// round-trip this replaced.) Spans are `Span::default()` scaffolding — the
/// directive consumer's shared check sequence (`check_generated_function_item`)
/// re-bases them to the real application anchor before the decl is reserved.
fn build_function_item(
    name: &str,
    return_type_slot: &KindedSlot,
    value_slot: &KindedSlot,
    overlay: &FreezeOverlay,
) -> Result<shape_ast::ast::Item, String> {
    use shape_ast::ast::{FunctionDef, Item, Span, Statement};

    if !is_valid_generated_function_name(name) {
        return Err(format!(
            "item_fn expected a valid generated free-function name, got '{}'",
            name
        ));
    }
    let return_type =
        type_annotation_from_string_or_type_ref_slot(return_type_slot, "item_fn", overlay)?;
    let body_expr = literal_expr_from_slot(value_slot, "item_fn")?;

    Ok(Item::Function(
        FunctionDef {
            name: name.to_string(),
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: Vec::new(),
            return_type: Some(return_type),
            where_clause: None,
            body: vec![Statement::Expression(body_expr, Span::default())],
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            effect_row: None,
        },
        Span::default(),
    ))
}

/// ADR-009 E2 #18 (slice 4.5): read an `Array<string>` comptime-builtin argument
/// into `Vec<String>`. Mirrors the established v2-raw string-array read
/// (`comptime_target.rs`): kind witness + non-null + element-type stamp guard,
/// then `TypedArray::as_slice` over `*const StringObj`. Used by `extend_method`
/// to read the template's literal segments and self-field splices.
fn read_comptime_string_array_slot(slot: &KindedSlot, arg_name: &str) -> Result<Vec<String>, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(format!(
            "extend_method expects {arg_name} as an Array<string>, got {:?}",
            slot.kind()
        ));
    }
    let ptr = slot.raw() as *const TypedArray<*const StringObj>;
    if ptr.is_null() {
        return Err(format!("extend_method received a null {arg_name} array"));
    }
    // SAFETY: kind witness (Ptr(TypedArray)) + non-null + ELEM_TYPE_STRING stamp
    // prove this is a live `Array<string>`; each element is a borrowed
    // `*const StringObj` (no ownership taken); we copy each into an owned String.
    unsafe {
        if read_elem_type(ptr as *const u8) != ELEM_TYPE_STRING {
            return Err(format!(
                "extend_method expects {arg_name} to be an Array<string> (element type mismatch)"
            ));
        }
        let slice = TypedArray::<*const StringObj>::as_slice(ptr);
        let mut out = Vec::with_capacity(slice.len());
        for &elem in slice {
            if elem.is_null() {
                return Err(format!("extend_method received a null string in {arg_name}"));
            }
            out.push(StringObj::as_str(elem).to_string());
        }
        Ok(out)
    }
}

/// ADR-009 C3 #14 (slice 2): read an `Array<__CaptureBinding>` comptime-builtin
/// argument into the resolved `(name, LiftedConst)` bindings. Sibling of
/// [`read_comptime_string_array_slot`]: kind witness + non-null + element-type
/// stamp guard, then `TypedArray::as_slice` over `*const TypedObjectStorage`;
/// each element is schema-name-checked (`__CaptureBinding`) and its opaque
/// `index` handle resolved against the per-run capture-binding store.
fn read_capture_binding_array_slot(
    slot: &KindedSlot,
    builtin_name: &str,
) -> Result<Vec<(String, LiftedConst)>, String> {
    use shape_value::v2::typed_array::ELEM_TYPE_TYPED_OBJECT;

    if slot.kind() != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(format!(
            "{builtin_name} expects captures as an Array<__CaptureBinding> built from \
             capture(name, value) calls; got {:?}",
            slot.kind()
        ));
    }
    let ptr = slot.raw() as *const TypedArray<*const TypedObjectStorage>;
    if ptr.is_null() {
        return Err(format!("{builtin_name} received a null captures array"));
    }
    // SAFETY: kind witness (Ptr(TypedArray)) + non-null prove a live
    // TypedArray header; `len` sits at a T-independent offset (repr(C),
    // pointer-sized `data`), so the empty-array read is layout-safe under any
    // element stamp. Non-empty arrays additionally require the
    // ELEM_TYPE_TYPED_OBJECT stamp before the element slice is read; each
    // element is a borrowed `*const TypedObjectStorage` (no ownership taken).
    unsafe {
        if (*ptr).len == 0 {
            return Ok(Vec::new());
        }
        if read_elem_type(ptr as *const u8) != ELEM_TYPE_TYPED_OBJECT {
            return Err(format!(
                "{builtin_name} expects captures as an Array<__CaptureBinding> built from \
                 capture(name, value) calls (element type mismatch)"
            ));
        }
        let slice = TypedArray::<*const TypedObjectStorage>::as_slice(ptr);
        let mut out = Vec::with_capacity(slice.len());
        for (i, &elem) in slice.iter().enumerate() {
            if elem.is_null() {
                return Err(format!(
                    "{builtin_name} received a null element at captures[{i}]"
                ));
            }
            let storage = &*elem;
            let schema =
                shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
                    .ok_or_else(|| {
                        format!(
                            "{builtin_name}: captures[{i}] is not a __CaptureBinding handle \
                             (unknown schema); build each element with capture(name, value)"
                        )
                    })?;
            if schema.name != "__CaptureBinding" {
                return Err(format!(
                    "{builtin_name}: captures[{i}] is not a __CaptureBinding handle (got \
                     schema '{}'); build each element with capture(name, value)",
                    schema.name
                ));
            }
            let index = field_slot_from_typed_object(storage, &schema, "index")?
                .as_i64()
                .ok_or_else(|| "__CaptureBinding.index is not an int".to_string())?;
            let binding = comptime_capture_binding_at(index as usize).ok_or_else(|| {
                format!(
                    "internal error: {builtin_name} capture-binding index {index} is not live \
                     in this comptime execution"
                )
            })?;
            out.push(binding);
        }
        Ok(out)
    }
}

/// ADR-009 C3 #14 (slice 2, S2b): resolve a single `__CheckedTemplate` handle
/// argument to its store index. Sibling of the `__CheckedItem` handle read
/// (`parse_extend_items_slot`): schema-name-checked TypedObject + opaque
/// `index` field; a non-handle slot (any other value or typed object) is a
/// NAMED rejection with the positive twin naming the two producers. The index
/// is NOT resolved against the store here — the pass-2 driver resolves it
/// after `vm.execute` returns (the store stays intact until the next per-run
/// clear), and a dead index there is internal-error-shaped.
fn checked_template_index_from_slot(
    slot: &KindedSlot,
    builtin_name: &str,
) -> Result<usize, String> {
    let not_a_handle = |detail: &str| {
        format!(
            "{builtin_name} expects a __CheckedTemplate handle ({detail}); construct one with \
             before_hook(body_fn, captures) or after_hook(body_fn, captures) and pass it \
             directly"
        )
    };
    let Some(storage) = slot.as_typed_object_storage() else {
        return Err(not_a_handle(&format!("got kind {:?}", slot.kind())));
    };
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| not_a_handle("got a typed object with an unknown schema"))?;
    if schema.name != "__CheckedTemplate" {
        return Err(not_a_handle(&format!("got schema '{}'", schema.name)));
    }
    let index = field_slot_from_typed_object(storage, &schema, "index")?
        .as_i64()
        .ok_or_else(|| "__CheckedTemplate.index is not an int".to_string())?;
    Ok(index as usize)
}

/// ADR-009 C3 #14 (slice 2): the shared `before_hook`/`after_hook` body — the
/// EAGER construction path through the S1 typestate chokepoint. The API is a
/// PRODUCER of the ONE `CheckedTemplate` carrier, never a second carrier:
///
/// 1. the body argument arrives as a template-body INDEX (an `Int64` literal
///    minted by the emit-side rewrite; a bare fn identifier is the only
///    user-spellable form — C3-G3, code is code);
/// 2. the captures array resolves to `(name, LiftedConst)` bindings;
/// 3. `CheckedTemplateBuilder::new(kind).body_fn(&def)?.captures(clause)
///    .finish()?` runs the FULL construction validation (classification, the
///    capture-tail bijection, [C0902]/[C0907] via the reused validator,
///    pseudo-tuple uses);
/// 4. the lifted values are validated against the declared trailing
///    capture-parameter types (`const_lift::validate_capture_value_types`);
/// 5. the [`BoundTemplate`] is stashed and the opaque-index
///    `__CheckedTemplate` handle returned (the E2 `item_fn` pattern).
///
/// A constructed-never-installed template is a no-op; install is the S2b
/// `install` builtin + directive consumer.
///
/// Errors are C3-G13 string-tag message-text (the ruled E1 precedent for
/// comptime-builtin-layer diagnostics): every comptime builtin surfaces
/// failures as `Err(String)` — there is no coded-diagnostic path for a
/// comptime builtin today. Routing this family through a coded path is the
/// named pre-existing follow-up on record (#60); revisit the tags when #60's
/// coded path lands. S5 owns C09xx minting from C0931+; S2 mints NO codes.
fn build_hook_template(
    hook_kind: TemplateHookKind,
    body_slot: &KindedSlot,
    bindings: Vec<(String, LiftedConst)>,
    builtin_name: &str,
) -> Result<TypedReturn, String> {
    let Some(body_index) = body_slot.as_i64() else {
        return Err(format!(
            "{builtin_name} expects its body argument as a bare module-scope fn identifier \
             (transported by the compile-time rewrite; got kind {:?}); declare \
             `fn my_hook(...)` at module scope and pass `my_hook` — a string, closure, call, \
             or computed value is not a template body (code is code)",
            body_slot.kind()
        ));
    };
    let def = comptime_template_body_fn_at(body_index as usize).ok_or_else(|| {
        format!(
            "internal error: {builtin_name} body-template index {body_index} is not live in \
             this handler run"
        )
    })?;

    // The declared capture set: one C1 value-snapshot (`move`) entry per
    // binding, in ARRAY order. Borrow modes are structurally unconstructible
    // through `capture()` — [C0902] stays reachable only as defense via the
    // reused `validate_capture_clause`; duplicates reject there as [C0907].
    let clause = shape_ast::ast::CaptureClause {
        entries: bindings
            .iter()
            .map(|(name, _)| shape_ast::ast::CaptureEntry {
                mode: shape_ast::ast::CaptureMode::Move,
                name: name.clone(),
                span: shape_ast::ast::Span::default(),
                name_span: shape_ast::ast::Span::default(),
            })
            .collect(),
        span: shape_ast::ast::Span::default(),
    };

    let template = CheckedTemplateBuilder::new(hook_kind)
        .body_fn(&def)
        .map_err(|e| e.to_string())?
        .captures(clause)
        .finish()
        .map_err(|e| e.to_string())?;

    const_lift::validate_capture_value_types(&def.name, template.capture_params(), &bindings)?;

    let index = push_comptime_hook_template(BoundTemplate {
        template,
        capture_values: bindings,
    });
    let handle = typed_object_for_named_schema(
        "__CheckedTemplate",
        &[("index", KindedSlot::from_int(index as i64))],
    );
    Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
        Arc::new(heap_value_from_typed_object_slot(handle)),
    )))
}

/// ADR-009 E2 #18 (slice 4.5, condition 1 — BOUNDED HOLE GRAMMAR): a self-field
/// splice must be a bare identifier, so the assembled `{self.<ident>}` hole is
/// structurally incapable of carrying an arbitrary handler expression. Anything
/// else is rejected at the builtin boundary with the named `[C0927]` diagnostic
/// (E2-D5: E2's diagnostic block is C0927+). This is the injection guard the
/// negative pin exercises (`a} + evil() + {b` etc. are rejected, never assembled).
///
/// HONEST-NAMING caveat (E2-Q2/B condition 2, review F3): `[C0927]` here is an
/// UNCODED STRING TAG prefixed into the builtin's `Err(String)` message — NOT a
/// registered diagnostic code. This is a PRE-EXISTING infra gap: every comptime
/// builtin surfaces failures as `Err(String)` (there is no path for a comptime
/// builtin to emit a coded diagnostic), so no E2 builtin can mint a real code
/// today. Minting `C0927` as a genuine registered diagnostic is a follow-up on
/// record; the pin asserts `contains("C0927")`, which passes on the substring.
fn is_valid_self_field_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// ADR-009 E2 #18 (slice 4.5): escape a literal template segment for the
/// `Literal::FormattedString` value form. Only braces need escaping so the
/// interpolation parser (`parse_interpolation_with_mode`) reads them as LITERAL
/// braces (`\{`/`\}`), not interpolation delimiters; quotes and other bytes are
/// already in post-string-unescape form in a FormattedString value and pass
/// through literally. This reproduces byte-for-byte the value the retired
/// `extend (f"…")` source route produced after string-literal parsing (verified
/// against showcases `TO_JSON_EXPECTED`).
fn escape_fstring_literal_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for ch in segment.chars() {
        match ch {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            other => out.push(other),
        }
    }
    out
}

/// ADR-009 E2 #18 (slice 4.5, E2-Q2/B): build the generated `Item::Extend { Type
/// { method <name>() -> <ret> { <f-string> } } }` DIRECTLY from a typed template
/// (E2-D8/§4.5 typed producer; the serialize-shaped minimal subset). The body is
/// a NATIVE f-string literal the builtin assembles from the template:
///
/// - `segments` are literal text (ConstLift'd JSON punctuation / field names),
///   escaped for the FormattedString value (`escape_fstring_literal_segment`);
/// - `field_splices` are field-name identifiers, ASSERTED (condition 1) and
///   emitted ONLY as producer-generated `{self.<ident>}` holes.
///
/// Invariant: `segments.len() == field_splices.len() + 1` (strict interleave
/// `seg[0] {self.f[0]} seg[1] … {self.f[n-1]} seg[n]`). The producer-generated
/// holes resolve through the language's native f-string compile parse
/// (`parse_expression_str`) — the SAME path as every user-authored f-string;
/// this is NOT the U03 directive transport and NOT body-as-text (E2-Q2/B ruling,
/// 2026-07-18). No source text, no `parse_program`, no directive payload.
fn build_extend_method_item(
    type_name: &str,
    method_name: &str,
    return_type_slot: &KindedSlot,
    segments: &[String],
    field_splices: &[String],
    overlay: &FreezeOverlay,
) -> Result<shape_ast::ast::Item, String> {
    use shape_ast::ast::{Expr, InterpolationMode, Literal, Span};

    if segments.len() != field_splices.len() + 1 {
        return Err(format!(
            "extend_method template mismatch: {} segments require {} field splices, got {}",
            segments.len(),
            segments.len().saturating_sub(1),
            field_splices.len()
        ));
    }

    // Assemble the native f-string VALUE: escaped literal segments interleaved
    // with producer-generated `{self.<ident>}` holes (each field asserted).
    let mut value = String::new();
    for (i, field) in field_splices.iter().enumerate() {
        if !is_valid_self_field_identifier(field) {
            return Err(format!(
                "[C0927] extend_method: field splice '{field}' is not a valid identifier; the \
                 template channel only carries `self.<identifier>` holes, never an expression"
            ));
        }
        value.push_str(&escape_fstring_literal_segment(&segments[i]));
        value.push_str("{self.");
        value.push_str(field);
        value.push('}');
    }
    value.push_str(&escape_fstring_literal_segment(&segments[field_splices.len()]));

    let body_expr = Expr::Literal(
        Literal::FormattedString {
            value,
            mode: InterpolationMode::Braces,
        },
        Span::default(),
    );

    build_extend_item_with_method_body(
        "extend_method",
        type_name,
        method_name,
        return_type_slot,
        body_expr,
        overlay,
    )
}

/// ADR-009 E2 #18 (slice 5b-1): build the generated `Item::Extend { Type { method
/// <name>() -> <ret> { <literal> } } }` whose method body is a typed LITERAL value
/// (int / number / bool / string), DIRECTLY from the value slot. The literal is
/// decoded by the SAME `literal_expr_from_slot` authority `item_fn` uses (slice 2)
/// — no second literal decoder, no source text, no f-string template. This is the
/// literal-body sibling of `build_extend_method_item`: it closes the migration gap
/// for the retired `extend (f"…{ <literal> }…")` fixtures that generate a CONSTANT
/// method body (e.g. `method answer() -> int { 42 }`), which the template producer
/// (self-field interpolation only) cannot express. Both producers share the method
/// + `Item::Extend` assembly (`build_extend_item_with_method_body`) and both flow
/// to the same generic `Item::Extend` materialization; the body expr is the only
/// axis of difference.
fn build_extend_method_literal_item(
    type_name: &str,
    method_name: &str,
    return_type_slot: &KindedSlot,
    value_slot: &KindedSlot,
    overlay: &FreezeOverlay,
) -> Result<shape_ast::ast::Item, String> {
    let body_expr = literal_expr_from_slot(value_slot, "extend_method_literal")?;
    build_extend_item_with_method_body(
        "extend_method_literal",
        type_name,
        method_name,
        return_type_slot,
        body_expr,
        overlay,
    )
}

/// ADR-009 E2 #18 (slice 5b-1): the SHARED method + `Item::Extend` assembly for
/// both `extend_method` (computed self-field f-string body) and
/// `extend_method_literal` (typed literal body). Validates the type/method
/// identifiers, resolves the typed return annotation, and wraps the caller-built
/// `body_expr` in a single-method `Item::Extend`. The `body_expr` is the ONLY axis
/// of difference between the two producers — neither materializes source text.
/// `builtin_name` labels the identifier / return diagnostics with the calling
/// producer.
fn build_extend_item_with_method_body(
    builtin_name: &str,
    type_name: &str,
    method_name: &str,
    return_type_slot: &KindedSlot,
    body_expr: shape_ast::ast::Expr,
    overlay: &FreezeOverlay,
) -> Result<shape_ast::ast::Item, String> {
    use shape_ast::ast::{ExtendStatement, Item, MethodDef, Span, Statement, TypeName};

    if !is_valid_self_field_identifier(method_name) {
        return Err(format!(
            "{builtin_name} expected a valid method name, got '{method_name}'"
        ));
    }
    if !is_valid_self_field_identifier(type_name) {
        return Err(format!(
            "{builtin_name} expected a valid type name, got '{type_name}'"
        ));
    }

    let return_type =
        type_annotation_from_string_or_type_ref_slot(return_type_slot, builtin_name, overlay)?;

    let method = MethodDef {
        name: method_name.to_string(),
        span: Span::default(),
        declaring_module_path: None,
        doc_comment: None,
        annotations: Vec::new(),
        type_params: None,
        params: Vec::new(),
        when_clause: None,
        return_type: Some(return_type),
        body: vec![Statement::Expression(body_expr, Span::default())],
        is_async: false,
    };

    Ok(Item::Extend(
        ExtendStatement {
            // `TypeName::Simple` carries a `TypePath`; `.into()` builds it via
            // `TypePath::from_qualified` — byte-identical to the legacy extend
            // parse route (`TypeName::Simple(name.into())`, parser/extensions.rs),
            // so the generated extend keys the target type exactly as a parsed
            // `extend Type { … }` would.
            type_name: TypeName::Simple(type_name.into()),
            methods: vec![method],
        },
        Span::default(),
    ))
}

/// ADR-009 E2 #18 (slice 5): resolve a typed generation carrier to AST items —
/// `__CheckedItem`-ONLY. The source-string `extend`/`replace module` generation
/// route (U03) was deleted this slice; a non-carrier slot (a source string, any
/// other typed object, or a non-object) is rejected with the named `[C0929]`
/// diagnostic naming the typed alternatives.
///
/// HONEST-NAMING caveat (same pre-existing infra gap as C0927/C0928): `[C0929]`
/// is an UNCODED STRING TAG prefixed into the `Err(String)` message, NOT a
/// registered diagnostic code — a comptime builtin has no path to emit a coded
/// diagnostic. Minting it as a real code is the shared C092x follow-up on record.
fn parse_extend_items_slot(slot: &KindedSlot) -> Result<Vec<shape_ast::ast::Item>, String> {
    const C0929_REJECTION: &str = "[C0929] the source-string `extend`/`replace module` generation route has been removed; pass a typed generation carrier — item_fn(name, ret, value) for a free function, extend_method(...)/extend_method_literal(...) for a method, or use the direct `extend target { … }` statement";

    let Some(storage) = slot.as_typed_object_storage() else {
        return Err(C0929_REJECTION.to_string());
    };
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| C0929_REJECTION.to_string())?;
    if schema.name != "__CheckedItem" {
        return Err(C0929_REJECTION.to_string());
    }
    // The TYPED route — a `__CheckedItem` handle `item_fn` / `extend_method*`
    // produced. Resolve its index back to the driver-side `CheckedItem` built
    // during THIS comptime run, with no sentinel decode and no source/JSON string.
    let index = field_slot_from_typed_object(storage, &schema, "index")?
        .as_i64()
        .ok_or_else(|| "__CheckedItem.index is not an int".to_string())?;
    let checked = comptime_checked_item_at(index as usize).ok_or_else(|| {
        format!("__CheckedItem index {index} is not live in this comptime execution")
    })?;
    Ok(vec![checked.into_item()])
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
/// ADR-009 §4.1 (slice S2): the reflection builtins (`type_ref` /
/// `type_category` / `reflect`) consume the per-compilation-unit semantic
/// freeze through the shared `Arc<FreezeOverlay>` handle — the intrinsic
/// closures clone the `Arc`, never snapshot data. The deleted per-site
/// `build_type_reflection_snapshot` rebuild has no successor here.
///
/// `site_time_impl_keys` (slice S5) is the superset key snapshot visible at
/// the comptime site (live keys + J-CT.2 `comptime impl` pairs); it feeds
/// ONLY `find_impl`'s named Dec 52 post-barrier ordering diagnostic — never
/// evidence. Supported key forms: "TraitName::TypeName" and
/// "TraitName::TypeName::ImplNameOrDefault".
pub(crate) fn create_comptime_builtins_module(
    site_time_impl_keys: HashSet<String>,
    freeze: Arc<FreezeOverlay>,
) -> ModuleExports {
    let mut module = comptime_builtins_module_base(Arc::clone(&freeze));
    // ADR-009 B2 (slice S4): `trait_ref` / `find_impl` consume the SAME
    // freeze handle — implementation evidence comes ONLY from the frozen
    // barrier truth (freeze inputs 4/5).
    trait_evidence::register_trait_evidence_builtins(
        &mut module,
        Arc::clone(&freeze),
        site_time_impl_keys,
    );
    register_frozen_reflection_builtins(&mut module, freeze);
    module
}

// ADR-009 A1 slice S3: the S2-era speculative pre-pass module
// (`create_annotation_prepass_builtins_module` + the named
// `PREPASS_REFLECTION_UNAVAILABLE` call-time rejections) is DELETED — the
// semantic-freeze barrier now runs before the annotation pre-passes in
// `compile()`, so every handler execution (speculative or authoritative)
// consumes the real freeze handle via `create_comptime_builtins_module`.

/// All comptime builtins EXCEPT the freeze-consuming reflection trio.
///
/// ADR-009 E1 #17 (slice 5, A-FULL): the typed-generation producers
/// (`item_fn` / `extend_method` / `extend_method_literal`) and the type-ref
/// directive emitters (`__emit_set_param_type` / `__emit_set_return_type`) now
/// resolve a stamped `__ComptimeTypeRef` via its frozen identity, so they need
/// the SAME `Arc<FreezeOverlay>` the producer stamped with (stage 3
/// shared-overlay plumbing). Each such closure move-captures its own
/// `Arc::clone(&freeze)` and threads `&freeze` into
/// `type_annotation_from_string_or_type_ref_slot`, whose `overlay.payload_of`
/// then finds any composite identity interned at produce time. The overlay
/// reaching the reflection trio (`create_comptime_builtins_module`) is the same
/// Arc, so the whole comptime module shares one freeze memo.
fn comptime_builtins_module_base(freeze: Arc<FreezeOverlay>) -> ModuleExports {
    let mut module = ModuleExports::new("__comptime__");

    // warning(msg: string) -> Unit
    // Collects a compile-time warning. The message flows out on the
    // thread-local diagnostics buffer; the compiler re-emits it as a
    // spanned, LSDS-routed warning anchored at the comptime construct
    // (comptime-excellence §4.4). The old bare `eprintln!` (span-less,
    // not routed) is deleted.
    register_typed_function(
        &mut module,
        "warning",
        "Emit a compile-time warning",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            if let Some(msg) = nb_args.first().and_then(|nb| nb.as_str()) {
                push_comptime_diagnostic(ComptimeDiagnostic {
                    message: msg.to_string(),
                });
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
    //
    // S2 (comptime-excellence §4.3): constructed via
    // `typed_object_for_named_schema("__ComptimeBuildConfig", ...)`, which
    // resolves the reserved, concrete named schema (`builtin_schemas.rs` —
    // `bool debug` + `string version/target_os/target_arch`) BY NAME. Every
    // field carries a statically-sourceable NativeKind (no `FieldType::Any`,
    // so no `MakeFieldRef ... FIELD_TAG_ANY` — the R2 hazard). Named
    // resolution supersedes the earlier "rely on order-insensitive field-set
    // match" posture: `__ComptimeBuildConfig` is now `reserved` and is
    // therefore SKIPPED by field-set inference, so it could no longer be
    // reached that way regardless.
    register_typed_function(
        &mut module,
        "build_config",
        "Return build-time configuration",
        vec![],
        ConcreteType::Object,
        |_args, _ctx| {
            // ADR-006 §2.7.6 / Q8 (Wave 5a Substep 3): build the typed
            // object via `typed_object_for_named_schema` (which takes
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
            let kinded = typed_object_for_named_schema(
                "__ComptimeBuildConfig",
                &[
                    ("debug", KindedSlot::from_bool(cfg!(debug_assertions))),
                    ("version", nb_str(env!("CARGO_PKG_VERSION"))),
                    ("target_os", nb_str(std::env::consts::OS)),
                    ("target_arch", nb_str(std::env::consts::ARCH)),
                    // Frozen introspection-contract version marker
                    // (comptime-excellence §4.1.4).
                    ("comptime_api", KindedSlot::from_int(1)),
                ],
            );
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
            // SAFETY: per `typed_object_for_named_schema`'s construction-
            // side contract, `ptr` points to a live TypedObjectStorage with
            // refcount ≥ 1.
            unsafe {
                shape_value::v2::refcount::v2_retain(&(*ptr).header);
            }
            drop(kinded);
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(HeapValue::TypedObject(
                    shape_value::heap_value::TypedObjectPtr::new(ptr),
                )),
            )))
        },
    );

    // The freeze-consuming reflection builtins (`type_ref` / `type_category` /
    // `reflect` / `find_impl`) are NOT part of the base:
    // `create_comptime_builtins_module` registers them with the shared
    // `Arc<FreezeOverlay>` handle (the barrier runs before every comptime site — S3).

    // item_fn(name: string, return_type: string | TypeRef, value: literal) -> ItemFragment
    //
    // First typed additive-generation slice: construct a zero-arg free
    // function fragment without requiring the comptime handler to assemble
    // `fn ...` source text. The fragment is still converted to an AST item and
    // compiled by the same strict registration/type/body pipeline as the
    // source-string `extend (expr)` path.
    let freeze_for_item_fn = Arc::clone(&freeze);
    register_typed_function(
        &mut module,
        "item_fn",
        // E2-D10: this SURFACE (name + signature) survives E2 as the CheckedItem
        // constructor; its INTERNALS (the __ComptimeItemFragment schema + sentinel
        // machinery) were removed by the slice-5 U07 deletion.
        "Build a typed CheckedItem for a zero-arg generated free function",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "return_type".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "value".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        // ADR-009 E2 #18 (slice 2, E2-D10): `item_fn` yields the typed
        // `__CheckedItem` carrier — a handle to a driver-side `CheckedItem`. The
        // builtin builds the AST `Item` directly (no sentinel fields, no
        // source/JSON string), stashes it in the per-run `CheckedItem` store, and
        // returns a handle carrying its index. (Slice 5 deleted the legacy
        // `__ComptimeItemFragment` sentinel map + its fragment builders.)
        ConcreteType::OpaqueTypedObject("__CheckedItem".to_string()),
        move |slots, _ctx| {
            if slots.len() != 3 {
                return Err(format!("item_fn expects 3 arguments, got {}", slots.len()));
            }
            let name = slots[0]
                .as_str()
                .ok_or_else(|| "item_fn expects a string function name".to_string())?;
            let item = build_function_item(name, &slots[1], &slots[2], &freeze_for_item_fn)?;
            let index = push_comptime_checked_item(CheckedItem::new(item));
            let handle = typed_object_for_named_schema(
                "__CheckedItem",
                &[("index", KindedSlot::from_int(index as i64))],
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(heap_value_from_typed_object_slot(handle)),
            )))
        },
    );

    // ADR-009 E2 #18 (slice 4.5, E2-Q2/B USER-RATIFIED 2026-07-18): the typed
    // producer for a SINGLE generated extend-method with a computed body — the
    // serialize-shaped minimal subset that closes the `@to_json` gap (item_fn is
    // free-function + literal-body only). The body is a NATIVE f-string assembled
    // by the builtin from a typed template: literal `segments` (ConstLift'd
    // punctuation) interleaved with `field_splices` (field-name identifiers,
    // asserted — condition 1 BOUNDED HOLE GRAMMAR). The producer-generated
    // `{self.<ident>}` holes resolve through the language's OWN native f-string
    // compile parse (`parse_expression_str`, `string_interpolation.rs`), the same
    // path as every user-authored f-string — this is NOT the U03 directive
    // transport and NOT body-as-text (E2-Q2/B ruling). No source string, no
    // `parse_program`. Yields the `__CheckedItem` carrier (Item-general, slice 2)
    // wrapping an `Item::Extend`, which flows through `parse_extend_items_slot` ->
    // ExtendItems -> the existing generic Item::Extend materialization (stamp /
    // reserve / register), so there is zero consumer change. `extend_method` is
    // an INTERNAL builtin (stdlib-consumed like item_fn), not a public
    // quote/builder surface (that stays E1's per the C2 D1 amendment).
    let freeze_for_extend_method = Arc::clone(&freeze);
    register_typed_function(
        &mut module,
        "extend_method",
        "Build a typed CheckedItem for one generated extend-method whose body is a computed self-field f-string template",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "type_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "method_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "return_type".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                // Parameterized `Array<string>` — a bare `Array` is an invalid
                // unparameterized generic at a strict-checked direct call site
                // (the intrinsics' bare `"Array"` works only because they are
                // SOH-gated / method-rewrite-called, not directly). A user-called
                // comptime builtin (like this one and item_fn) must declare a
                // valid param type, or the handler's `extend_method(...)` call
                // fails to type-check.
                name: "segments".to_string(),
                type_name: "Array<string>".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "field_splices".to_string(),
                type_name: "Array<string>".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject("__CheckedItem".to_string()),
        move |slots, _ctx| {
            if slots.len() != 5 {
                return Err(format!(
                    "extend_method expects 5 arguments, got {}",
                    slots.len()
                ));
            }
            let type_name = slots[0]
                .as_str()
                .ok_or_else(|| "extend_method expects a string type name".to_string())?;
            let method_name = slots[1]
                .as_str()
                .ok_or_else(|| "extend_method expects a string method name".to_string())?;
            let segments = read_comptime_string_array_slot(&slots[3], "segments")?;
            let field_splices = read_comptime_string_array_slot(&slots[4], "field_splices")?;
            let item = build_extend_method_item(
                type_name,
                method_name,
                &slots[2],
                &segments,
                &field_splices,
                &freeze_for_extend_method,
            )?;
            let index = push_comptime_checked_item(CheckedItem::new(item));
            let handle = typed_object_for_named_schema(
                "__CheckedItem",
                &[("index", KindedSlot::from_int(index as i64))],
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(heap_value_from_typed_object_slot(handle)),
            )))
        },
    );

    // ADR-009 E2 #18 (slice 5b-1): the typed producer for a SINGLE generated
    // extend-method whose body is a typed LITERAL value (int / number / bool /
    // string) — the literal-body sibling of `extend_method` (which carries a
    // computed self-field f-string template only). It closes the migration gap for
    // the retired `extend (f"…{ <literal> }…")` fixtures that generate a CONSTANT
    // method body (e.g. `method answer() -> int { 42 }`). Same carrier shape as
    // item_fn / extend_method (returns the `__CheckedItem` OpaqueTypedObject
    // accepted by `extend (expr)`); the `value` literal is decoded by the SAME
    // `literal_expr_from_slot` authority item_fn uses (single literal decoder). Like
    // extend_method, handler-scope resolution requires the paired forwarder row in
    // `COMPTIME_BUILTIN_FORWARDERS` IN ADDITION to this registration, or
    // `extend_method_literal(...)` is `[C0001] Undefined function` in a handler.
    let freeze_for_extend_method_literal = Arc::clone(&freeze);
    register_typed_function(
        &mut module,
        "extend_method_literal",
        "Build a typed CheckedItem for one generated extend-method whose body is a typed literal value",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "type_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "method_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "return_type".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                // The literal method body; the slot's runtime kind selects the
                // literal (string / int / number / bool) — inferred from the caller.
                name: "value".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject("__CheckedItem".to_string()),
        move |slots, _ctx| {
            if slots.len() != 4 {
                return Err(format!(
                    "extend_method_literal expects 4 arguments, got {}",
                    slots.len()
                ));
            }
            let type_name = slots[0]
                .as_str()
                .ok_or_else(|| "extend_method_literal expects a string type name".to_string())?;
            let method_name = slots[1]
                .as_str()
                .ok_or_else(|| "extend_method_literal expects a string method name".to_string())?;
            let item = build_extend_method_literal_item(
                type_name,
                method_name,
                &slots[2],
                &slots[3],
                &freeze_for_extend_method_literal,
            )?;
            let index = push_comptime_checked_item(CheckedItem::new(item));
            let handle = typed_object_for_named_schema(
                "__CheckedItem",
                &[("index", KindedSlot::from_int(index as i64))],
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(heap_value_from_typed_object_slot(handle)),
            )))
        },
    );

    // ADR-009 C3 #14 (slice 2): the PUBLIC hook-template comptime API — the
    // C3-G1 metaprogramming primitive for annotation runtime hooks.
    // `before_hook`/`after_hook` reference an ordinary typed Shape fn as a
    // hook-template BODY (C3-G3) with DECLARED captures and construct the S1
    // `CheckedTemplate` EAGERLY through its typestate chokepoint
    // (`build_hook_template` — the API is a PRODUCER of the one carrier,
    // never a second carrier). The `body` argument is a bare fn IDENTIFIER
    // transported by the emit-side rewrite (`rewrite_template_hook_body_args`,
    // comptime.rs — identity-literal transport; never a string). Like every
    // comptime builtin, handler-scope resolution requires the paired
    // forwarder rows in `COMPTIME_BUILTIN_FORWARDERS` IN ADDITION to these
    // registrations, or the names are `[C0001] Undefined function` in
    // handlers (the extend_method_literal lesson).
    //
    // C3-G13: failures are string-tag message-text (`Err(String)`) — see the
    // #60 routing note on `build_hook_template`.
    register_typed_function(
        &mut module,
        "before_hook",
        "Construct a checked before-hook template from a module-scope body fn and its declared captures",
        vec![
            shape_runtime::module_exports::ModuleParam {
                // Post-rewrite this is the template-body index literal; the
                // user-facing form is a bare module-scope fn identifier.
                name: "body".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "captures".to_string(),
                type_name: "Array<__CaptureBinding>".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject("__CheckedTemplate".to_string()),
        move |slots, _ctx| {
            if slots.len() != 2 {
                return Err(format!(
                    "before_hook expects 2 arguments, got {}",
                    slots.len()
                ));
            }
            let bindings = read_capture_binding_array_slot(&slots[1], "before_hook")?;
            build_hook_template(TemplateHookKind::Before, &slots[0], bindings, "before_hook")
        },
    );

    register_typed_function(
        &mut module,
        "after_hook",
        "Construct a checked after-hook template from a module-scope body fn and its declared captures",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "body".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "captures".to_string(),
                type_name: "Array<__CaptureBinding>".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject("__CheckedTemplate".to_string()),
        move |slots, _ctx| {
            if slots.len() != 2 {
                return Err(format!(
                    "after_hook expects 2 arguments, got {}",
                    slots.len()
                ));
            }
            let bindings = read_capture_binding_array_slot(&slots[1], "after_hook")?;
            build_hook_template(TemplateHookKind::After, &slots[0], bindings, "after_hook")
        },
    );

    // The ZERO-CAPTURE variants — reached ONLY through the emit-side
    // rewrite's empty-array lowering (`rewrite_template_hook_body_args`,
    // comptime.rs: `before_hook(f, [])` rewrites the call to the unspellable
    // arity-1 nocapture forwarder). An empty array literal at a
    // call-argument position has no element to prove a type from — there is
    // no untyped runtime array — so the empty-captures spelling is
    // transported structurally instead.
    register_typed_function(
        &mut module,
        "before_hook_nocapture",
        "Construct a checked before-hook template with an empty capture set (the empty-captures-array lowering)",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "body".to_string(),
            type_name: "unknown".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::OpaqueTypedObject("__CheckedTemplate".to_string()),
        move |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "before_hook_nocapture expects 1 argument, got {}",
                    slots.len()
                ));
            }
            build_hook_template(
                TemplateHookKind::Before,
                &slots[0],
                Vec::new(),
                "before_hook",
            )
        },
    );

    register_typed_function(
        &mut module,
        "after_hook_nocapture",
        "Construct a checked after-hook template with an empty capture set (the empty-captures-array lowering)",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "body".to_string(),
            type_name: "unknown".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::OpaqueTypedObject("__CheckedTemplate".to_string()),
        move |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "after_hook_nocapture expects 1 argument, got {}",
                    slots.len()
                ));
            }
            build_hook_template(TemplateHookKind::After, &slots[0], Vec::new(), "after_hook")
        },
    );

    // `capture(name, value)` — one declared capture binding (the C1
    // value-snapshot mode implicitly; borrow modes are structurally
    // unconstructible through this builtin). The value rides the KindedSlot
    // substrate and is lifted EAGERLY through the ConstLift seam
    // (`const_lift::lift_capture_value` — S3b: the full C3-G5 compositional
    // domain, recursively; never-liftables and out-of-domain kinds reject
    // at the capture() call with the named class arms, not later.
    register_typed_function(
        &mut module,
        "capture",
        "Declare one named capture binding (value snapshot) for a hook template",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "value".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject("__CaptureBinding".to_string()),
        move |slots, _ctx| {
            if slots.len() != 2 {
                return Err(format!("capture expects 2 arguments, got {}", slots.len()));
            }
            let name = slots[0].as_str().ok_or_else(|| {
                format!(
                    "capture expects a string capture name (got kind {:?}); spell the binding \
                     capture(\"name\", value)",
                    slots[0].kind()
                )
            })?;
            let lifted = const_lift::lift_capture_value(name, &slots[1])?;
            let index = push_comptime_capture_binding((name.to_string(), lifted));
            let handle = typed_object_for_named_schema(
                "__CaptureBinding",
                &[("index", KindedSlot::from_int(index as i64))],
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(heap_value_from_typed_object_slot(handle)),
            )))
        },
    );

    // ADR-009 C3 #14 (slice 2, S2b): `install(template)` — attach a
    // constructed hook template to the annotation's target function. The
    // target is IMPLICIT (the annotation's target), matching every existing
    // directive; the builtin resolves the opaque `__CheckedTemplate` handle
    // to its store index and pushes `ComptimeDirective::InstallHookTemplate`.
    // Directive consumption (specialize_template composition, the G8/driver
    // rejections, registry + staging) happens at the authoritative pass-2
    // function-target consumer — the store stays intact until the next
    // per-run clear, so the post-execute index resolution is safe (store
    // lifecycle doc above). C3-G13 string-tag errors (#60 routing note).
    register_typed_function(
        &mut module,
        "install",
        "Install a checked before/after hook template onto the annotation's target function",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "template".to_string(),
            type_name: "__CheckedTemplate".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        move |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!("install expects 1 argument, got {}", slots.len()));
            }
            let template_index = checked_template_index_from_slot(&slots[0], "install")?;
            push_comptime_directive(ComptimeDirective::InstallHookTemplate { template_index })?;
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
    // __emit_set_param_type(param_name: string, type_payload: string | TypeRef)
    let freeze_for_set_param_type = Arc::clone(&freeze);
    register_typed_function(
        &mut module,
        "__emit_set_param_type",
        "Internal: set a parameter type by name",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "param_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "type_payload".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        move |slots, _ctx| {
            if slots.len() != 2 {
                return Err(format!(
                    "__emit_set_param_type expects 2 arguments, got {}",
                    slots.len()
                ));
            }
            let param_name = slots[0]
                .as_str()
                .ok_or_else(|| "__emit_set_param_type expects a string param name".to_string())?
                .to_string();
            let type_annotation = type_annotation_from_string_or_type_ref_slot(
                &slots[1],
                "__emit_set_param_type",
                &freeze_for_set_param_type,
            )?;
            push_comptime_directive(ComptimeDirective::SetParamType {
                param_name,
                type_annotation,
            })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set a scalar parameter default value.
    // __emit_set_param_value(param_name: string, value: scalar)
    register_typed_function(
        &mut module,
        "__emit_set_param_value",
        "Internal: set a parameter default value by name",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "param_name".to_string(),
                type_name: "string".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "value".to_string(),
                type_name: "unknown".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        |slots, _ctx| {
            if slots.len() != 2 {
                return Err(format!(
                    "__emit_set_param_value expects 2 arguments, got {}",
                    slots.len()
                ));
            }
            let param_name = slots[0]
                .as_str()
                .ok_or_else(|| "__emit_set_param_value expects a string param name".to_string())?
                .to_string();
            let value = slots[1].clone();
            push_comptime_directive(ComptimeDirective::SetParamValue { param_name, value })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set function return type.
    // __emit_set_return_type(type_payload: string | TypeRef)
    let freeze_for_set_return_type = Arc::clone(&freeze);
    register_typed_function(
        &mut module,
        "__emit_set_return_type",
        "Internal: set the function return type",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "type_payload".to_string(),
            type_name: "unknown".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        move |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_set_return_type expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let type_annotation = type_annotation_from_string_or_type_ref_slot(
                &slots[0],
                "__emit_set_return_type",
                &freeze_for_set_return_type,
            )?;
            push_comptime_directive(ComptimeDirective::SetReturnType { type_annotation })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // ADR-009 E2 #18: the TYPED block-form `replace body { ... }` carrier — the
    // ONLY replace-body transport. The block-form emit stashes the Vec<Statement>
    // at handler-COMPILE (COMPTIME_REPLACE_BODIES) and passes its INDEX here — no
    // source/JSON string, no reparse. (Slice 5 deleted the source-string
    // `__emit_replace_body` builtin + `parse_function_body_payload`; the expr form
    // `replace body (expr)` is rejected at compile with [C0928].)
    // __emit_replace_body_checked(index: int)
    register_typed_function(
        &mut module,
        "__emit_replace_body_checked",
        "Internal: replace function body from a typed block-form carrier index",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "index".to_string(),
            type_name: "int".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_replace_body_checked expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let index = slots[0]
                .as_i64()
                .ok_or_else(|| "__emit_replace_body_checked expects an int index".to_string())?;
            let body = comptime_replace_body_at(index as usize).ok_or_else(|| {
                format!("__emit_replace_body_checked index {index} is not live in this comptime execution")
            })?;
            push_comptime_directive(ComptimeDirective::ReplaceBody { body })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // ADR-009 E1 #17 (slice 4): the direct-block `extend Type { … }` typed
    // carrier. The emit side stashes the literal `ExtendStatement` at
    // handler-COMPILE (`COMPTIME_EXTEND_STATEMENTS`) and passes its INDEX here —
    // no `serialize_directive_payload` JSON, no `serde_json` reparse. The legacy
    // string `__emit_extend` + `serialize_directive_payload` were DELETED whole
    // in slice 6 (07638332); the E2 COMPUTED-extend path (`__emit_extend_items`
    // ← `__CheckedItem`) is a distinct, execute-populated carrier and is untouched.
    // __emit_extend_checked(index: int)
    register_typed_function(
        &mut module,
        "__emit_extend_checked",
        "Internal: emit an extend directive from a typed block-form carrier index",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "index".to_string(),
            type_name: "int".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_extend_checked expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let index = slots[0]
                .as_i64()
                .ok_or_else(|| "__emit_extend_checked expects an int index".to_string())?;
            let extend = comptime_extend_statement_at(index as usize).ok_or_else(|| {
                format!(
                    "__emit_extend_checked index {index} is not live in this comptime execution"
                )
            })?;
            push_comptime_directive(ComptimeDirective::Extend(extend))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace module items from a typed generation
    // carrier (`item_fn(...)` -> `__CheckedItem`). `parse_extend_items_slot`
    // resolves it to AST items with no source/JSON string ever materializing, and
    // the module-target consumer builds a provenance-stamped `CheckedModule`.
    // (Slice 5 deleted the source-string arm + `parse_module_items_payload`; a
    // source-string payload now rejects with the named [C0929] diagnostic.)
    // __emit_replace_module(module_payload)
    register_typed_function(
        &mut module,
        "__emit_replace_module",
        "Internal: replace module items from a typed generation carrier",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "module_payload".to_string(),
            type_name: "unknown".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_replace_module expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let items = parse_extend_items_slot(&slots[0])?;
            push_comptime_directive(ComptimeDirective::ReplaceModuleChecked { items })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: ADD generated items from a typed generation
    // carrier (§4.5.7 `extend (expr)`; slice 5 deleted the source-string arm).
    register_typed_function(
        &mut module,
        "__emit_extend_items",
        "Internal: add generated module items from a typed generation carrier",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "items_payload".to_string(),
            type_name: "unknown".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::Unit,
        |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_extend_items expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let items = parse_extend_items_slot(&slots[0])?;
            push_comptime_directive(ComptimeDirective::ExtendItems { items })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // §4.5.7.4: render a string as a valid Shape string literal (surrounding
    // quotes + escaped quotes/backslashes/newlines/braces) for embedding a
    // computed string into generated source. Comptime-callable helper backing
    // `std::comptime::string_lit`.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "string_lit",
        "Render a string as a Shape string literal for embedding in generated source",
        "value",
        "string",
        ConcreteType::String,
        |value, _ctx| {
            let rendered = render_shape_string_literal(value.as_str());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(rendered)))
        },
    );

    module
}

/// Register the freeze-consuming reflection builtins (`type_ref` /
/// `type_category` / `reflect`) against the shared
/// per-compilation-unit freeze handle (ADR-009 §4.1, slice S2; `reflect`
/// added in B1 S3). Each closure clones the `Arc<FreezeOverlay>` — the
/// base index is shared, never rebuilt and never copied.
fn register_frozen_reflection_builtins(module: &mut ModuleExports, freeze: Arc<FreezeOverlay>) {
    let freeze_for_type_ref = Arc::clone(&freeze);
    register_typed_function(
        module,
        TYPE_REF_INTRINSIC,
        "Create an opaque TypeRef from compiler-resolved type syntax",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "identity_high".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "identity_low".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                .to_string(),
        ),
        move |slots, _ctx| {
            let identity_part = |index: usize| {
                slots
                    .get(index)
                    .and_then(KindedSlot::as_i64)
                    .ok_or_else(|| "internal type_ref identity transport is invalid".to_string())
            };
            let identity = FrozenTypeIdentity {
                high: identity_part(0)?,
                low: identity_part(1)?,
            };
            let type_ref = build_frozen_type_ref_heap_value(identity, &freeze_for_type_ref)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(type_ref),
            )))
        },
    );

    let freeze_for_type_category = Arc::clone(&freeze);
    register_typed_function(
        module,
        TYPE_CATEGORY_INTRINSIC,
        "Return the exhaustive semantic category of an opaque TypeRef",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "type_ref".to_string(),
            type_name: shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                .to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::OpaqueTypedObject("FrozenTypeCategory".to_string()),
        move |slots, _ctx| {
            let type_ref = slots
                .first()
                .ok_or_else(|| "type_category expects one TypeRef value".to_string())?;
            let category = frozen_type_category_from_ref(type_ref, &freeze_for_type_category)?;
            let category = build_frozen_type_category_heap_value(category)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(category),
            )))
        },
    );

    // ADR-009 B1 S3 — `reflect(TypeRef<T>) -> FrozenType<T>`: the FOURTH
    // freeze-consuming builtin against the same Arc'd handle. The TypeRef
    // argument goes through the same reader as `type_category`
    // (reflect-named R4 diagnostics); the payload comes from the ONE freeze
    // query API (`payload_of`); the value is the sealed `FrozenType` sum
    // carrier (unspellable descriptor schema, catalog-ordinal variant ids).
    // Reflecting a category whose payload ticket has not landed is the
    // named R1 per-category rejection, surfaced as a compile error from
    // the comptime run — never a partial descriptor.
    let freeze_for_reflect = Arc::clone(&freeze);
    register_typed_function(
        module,
        REFLECT_INTRINSIC,
        "Reflect an opaque TypeRef into the sealed FrozenType payload sum",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "type_ref".to_string(),
            type_name: shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                .to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::comptime_reflection::FROZEN_TYPE_PAYLOAD_ENUM_NAME.to_string(),
        ),
        move |slots, _ctx| {
            let type_ref = slots
                .first()
                .ok_or_else(|| "reflect expects exactly one TypeRef argument".to_string())?;
            let frozen = frozen_type_from_ref(type_ref, &freeze_for_reflect)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(frozen),
            )))
        },
    );

    // ADR-009 B5 (Stage 2, Dec 56) — `reflect_repr(TypeRef<T>,
    // RepresentationAccess<T>) -> FrozenType<T>`: the authority-gated complete
    // reflection. The FIRST argument is the same TypeRef reader as `reflect`;
    // the SECOND is decoded through the schema-name-checked
    // `representation_access_identity_from_ref` (a forged or non-authority slot
    // is the named R6 rejection). The authority must be bound to the SAME frozen
    // identity being reflected (a User authority cannot reflect Order's
    // representation). Only then does the SAME payload builder as `reflect`
    // answer the complete `FrozenType` sum — never a partial descriptor.
    let freeze_for_reflect_repr = Arc::clone(&freeze);
    register_typed_function(
        module,
        REFLECT_REPR_INTRINSIC,
        "Reflect the complete nominal representation of a TypeRef under a RepresentationAccess authority",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "type_ref".to_string(),
                type_name:
                    shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                        .to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "access".to_string(),
                type_name:
                    shape_runtime::type_schema::builtin_schemas::COMPTIME_REPRESENTATION_ACCESS_SCHEMA
                        .to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::comptime_reflection::FROZEN_TYPE_PAYLOAD_ENUM_NAME.to_string(),
        ),
        move |slots, _ctx| {
            let type_ref = slots
                .first()
                .ok_or_else(|| "reflect_repr expects a TypeRef and a RepresentationAccess".to_string())?;
            let access = slots
                .get(1)
                .ok_or_else(|| "reflect_repr expects a TypeRef and a RepresentationAccess".to_string())?;
            let frozen =
                type_reflection::frozen_type_from_repr_ref(type_ref, access, &freeze_for_reflect_repr)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(frozen),
            )))
        },
    );

    // ADR-009 B5 (Stage 2, Dec 56) — the compiler-only RepresentationAccess
    // mint. NOT registered as a spellable forwarder: the compiler injects a call
    // to it ONLY into an annotation expand-hook scope
    // (`functions_annotations.rs`), so user code can never obtain a capability.
    // The identity halves are identity-literal transport (like
    // `type_constructor`); the carrier is the schema-name-checked builder.
    let freeze_for_mint_access = Arc::clone(&freeze);
    register_typed_function(
        module,
        MINT_REPRESENTATION_ACCESS_INTRINSIC,
        "Mint a compiler-issued RepresentationAccess authority bound to a frozen type identity",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "identity_high".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "identity_low".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_REPRESENTATION_ACCESS_SCHEMA
                .to_string(),
        ),
        move |slots, _ctx| {
            let identity_part = |index: usize| {
                slots.get(index).and_then(KindedSlot::as_i64).ok_or_else(|| {
                    "internal RepresentationAccess mint identity transport is invalid".to_string()
                })
            };
            let identity = FrozenTypeIdentity {
                high: identity_part(0)?,
                low: identity_part(1)?,
            };
            let carrier = type_reflection::build_representation_access_heap_value(
                identity,
                &freeze_for_mint_access,
            )?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(carrier),
            )))
        },
    );

    // ADR-009 B4 (Stage 2, Dec 54) — uniform nominal-application intrinsics,
    // registered against the SAME per-compilation-unit freeze handle. Each
    // needing the freeze clones the `Arc`; carriers/decoders are the
    // schema-name-checked, forgery-blocking builders in `type_reflection.rs`.
    let type_constructor_ref_schema =
        shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA;
    let applied_type_schema =
        shape_runtime::type_schema::builtin_schemas::COMPTIME_APPLIED_TYPE_SCHEMA;

    // `type_constructor(C)` — head identity halves (identity-literal transport
    // from the site rewrite) → opaque TypeConstructorRef.
    let freeze_for_constructor = Arc::clone(&freeze);
    register_typed_function(
        module,
        TYPE_CONSTRUCTOR_INTRINSIC,
        "Create an opaque TypeConstructorRef from a compiler-issued frozen nominal head identity",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "identity_high".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "identity_low".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(type_constructor_ref_schema.to_string()),
        move |slots, _ctx| {
            let identity_part = |index: usize| {
                slots.get(index).and_then(KindedSlot::as_i64).ok_or_else(|| {
                    "internal type_constructor identity transport is invalid".to_string()
                })
            };
            let identity = FrozenTypeIdentity {
                high: identity_part(0)?,
                low: identity_part(1)?,
            };
            let carrier = type_reflection::build_type_constructor_ref_heap_value(
                identity,
                &freeze_for_constructor,
            )?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(carrier),
            )))
        },
    );

    // `const_arg(N)` — a checked const argument carrier (identity transport for
    // a const-generic application argument). No freeze needed.
    register_typed_function(
        module,
        CONST_ARG_INTRINSIC,
        "Create a checked const argument for a const-generic type application",
        vec![shape_runtime::module_exports::ModuleParam {
            name: "value".to_string(),
            type_name: "int".to_string(),
            required: true,
            ..Default::default()
        }],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA.to_string(),
        ),
        move |slots, _ctx| {
            let value = slots
                .first()
                .and_then(KindedSlot::as_i64)
                .ok_or_else(|| "const_arg expects an integer value".to_string())?;
            let carrier = type_reflection::build_const_arg_ref_heap_value(value)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(carrier),
            )))
        },
    );

    // `.apply(constructor, args)` — the receiver + a checked array of
    // type_ref/const_arg carriers (method-call site rewrite). Produces an
    // AppliedType whose identity EQUALS the A2 `type_ref(Head<Args>)` spelling.
    let freeze_for_apply = Arc::clone(&freeze);
    register_typed_function(
        module,
        APPLY_INTRINSIC,
        "Apply checked type/const arguments to a TypeConstructorRef, producing an AppliedType",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "constructor".to_string(),
                type_name: type_constructor_ref_schema.to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "args".to_string(),
                type_name: "Array".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(applied_type_schema.to_string()),
        move |slots, _ctx| {
            let receiver = slots
                .first()
                .ok_or_else(|| "apply expects a TypeConstructorRef receiver".to_string())?;
            let args = slots
                .get(1)
                .ok_or_else(|| "apply expects an argument array".to_string())?;
            let carrier =
                type_reflection::apply_to_constructor(receiver, args, &freeze_for_apply)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(carrier),
            )))
        },
    );

    // `.refine(applied, constructor)` — Some(AppliedType) on a head match, else
    // None (round-trips only over genuine applications). No freeze needed.
    register_typed_function(
        module,
        REFINE_INTRINSIC,
        "Refine an AppliedType against a TypeConstructorRef, recovering the application or None",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "applied".to_string(),
                type_name: applied_type_schema.to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "constructor".to_string(),
                type_name: type_constructor_ref_schema.to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::Option(Box::new(ConcreteType::OpaqueTypedObject(
            applied_type_schema.to_string(),
        ))),
        move |slots, _ctx| {
            let applied = slots
                .first()
                .ok_or_else(|| "refine expects an AppliedType receiver".to_string())?;
            let constructor = slots
                .get(1)
                .ok_or_else(|| "refine expects a TypeConstructorRef argument".to_string())?;
            match type_reflection::refine_application(applied, constructor)? {
                Some(carrier) => Ok(TypedReturn::Some(ConcreteReturn::OpaqueTypedObject(Arc::new(
                    carrier,
                )))),
                None => Ok(TypedReturn::None),
            }
        },
    );

    // `.type_argument(applied, index)` — the index-th type argument re-issued as
    // a TypeRef. Needs the freeze to validate the recovered identity.
    let freeze_for_type_argument = Arc::clone(&freeze);
    register_typed_function(
        module,
        TYPE_ARGUMENT_INTRINSIC,
        "Return the index-th type argument of an AppliedType as a TypeRef",
        vec![
            shape_runtime::module_exports::ModuleParam {
                name: "applied".to_string(),
                type_name: applied_type_schema.to_string(),
                required: true,
                ..Default::default()
            },
            shape_runtime::module_exports::ModuleParam {
                name: "index".to_string(),
                type_name: "int".to_string(),
                required: true,
                ..Default::default()
            },
        ],
        ConcreteType::OpaqueTypedObject(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA.to_string(),
        ),
        move |slots, _ctx| {
            let applied = slots
                .first()
                .ok_or_else(|| "type_argument expects an AppliedType receiver".to_string())?;
            let index = slots
                .get(1)
                .and_then(KindedSlot::as_i64)
                .ok_or_else(|| "type_argument expects an integer index".to_string())?;
            let carrier = type_reflection::applied_type_argument(
                applied,
                index,
                &freeze_for_type_argument,
            )?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(carrier),
            )))
        },
    );
}

/// Render `s` as a valid Shape string literal: wrap in double quotes and escape
/// the characters the Shape lexer treats specially inside a `"..."` literal.
/// Shape's escape set (string_literals.rs) is `\n \t \r \\ \" \' \0 \{ \} \$ \#`;
/// braces and `$`/`#` are f-string metacharacters, so escaping them keeps the
/// rendered literal safe to embed inside generated f-strings too.
fn render_shape_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '$' => out.push_str("\\$"),
            '#' => out.push_str("\\#"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// Tests gated `deep-tests` post-W11: bodies invoke
// `ModuleExports::invoke_export` which is part of the deleted comptime
// dispatch ABI; restoration requires the kinded comptime invocation
// surface (Phase-2c reentry per ADR-006 §2.7.4).
/// ADR-009 E2-D9 flip-condition tripwire (accepted addition to the slice-2
/// round). The typed MODULE-replacement producer is `item_fn` — the sole typed
/// producer today — and it yields a CLOSURE-FREE function: its body is a bare
/// literal, never an `Expr::FunctionExpr`. So a `replace module (item_fn(...))`
/// replacement carries no closure, and [C0911] (a generated closure whose
/// inference fact was never published) CANNOT fire for module scope — the
/// E2-D9 mootness, pinned on the real producer path.
///
/// When a closure-capable producer (`quote module`) lands, the typed module
/// route's item will be able to carry a closure and this assertion FLIPS. The
/// flip is the signal to write the module closure-capture C0911 pin FIRST and
/// decide the discovery-worklist question per E2-D9's flip condition — NOT to
/// relax this pin. Runs in the default tier (not deep-tests) so the gate trips.
#[cfg(test)]
mod e2_d9_closure_free_tripwire {
    use super::*;
    use shape_ast::ast::{Expr, Item, Statement};

    #[test]
    fn typed_module_producer_item_fn_is_closure_free() {
        let item = build_function_item(
            "answer",
            &nb_str("int"),
            &KindedSlot::from_int(42),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("item_fn's typed producer builds a literal function");
        let Item::Function(func_def, _) = item else {
            panic!("item_fn must produce a function item");
        };
        assert_eq!(
            func_def.body.len(),
            1,
            "a closure-free literal producer emits a single-statement body"
        );
        assert!(
            matches!(&func_def.body[0], Statement::Expression(Expr::Literal(..), _)),
            "the typed module producer's body is a bare literal — NO closure. If this flips, \
             a closure-capable producer landed: write the module C0911 closure-capture pin \
             FIRST (E2-D9 flip condition), do not relax this pin. Got: {:?}",
            func_def.body[0]
        );
    }
}

// ADR-009 E2 #18 (slice 4.5, E2-Q2/B) — producer-tier pins for `extend_method`'s
// body assembly + the condition-1 BOUNDED HOLE GRAMMAR boundary. Plain
// `#[cfg(test)]` (NOT deep-tests-gated) so they run in the standard gate — the
// first version of these lived in the deep-tests `mod tests` below and never ran
// (disclosed defect, slice 4.5 fix round). They exercise `build_extend_method_item`
// directly (no VM / no array-slot reading), pinning the AST shape, the byte-exact
// FormattedString value, and the identifier assertion in isolation; the
// end-to-end byte-parity arbiter is the shape-test
// `to_json_serializes_via_stdlib_import_*` rows.
#[cfg(test)]
mod extend_method_producer_tests {
    use super::*;

    fn extract_extend_method_body_value(item: &shape_ast::ast::Item) -> String {
        use shape_ast::ast::{Expr, Item, Literal, Statement};
        let Item::Extend(extend, _) = item else {
            panic!("expected Item::Extend, got a different item kind");
        };
        assert_eq!(extend.methods.len(), 1, "exactly one generated method");
        let method = &extend.methods[0];
        assert_eq!(method.body.len(), 1, "single-statement generated body");
        let Statement::Expression(Expr::Literal(Literal::FormattedString { value, .. }, _), _) =
            &method.body[0]
        else {
            panic!("generated method body must be a single FormattedString expression");
        };
        value.clone()
    }

    #[test]
    fn extend_method_assembles_byte_exact_fstring_body() {
        // The serialize `type User { id: int, name: string }` template.
        let segments = vec![
            "{ \"id\": ".to_string(),
            ", \"name\": \"".to_string(),
            "\" }".to_string(),
        ];
        let field_splices = vec!["id".to_string(), "name".to_string()];
        let item = build_extend_method_item(
            "User",
            "to_json",
            &KindedSlot::from_string("string"),
            &segments,
            &field_splices,
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid template assembles");

        // Braces in segments escape to `\{`/`\}`; splices are producer-generated
        // `{self.<ident>}` — byte-for-byte the value the retired source route
        // produced, so interpolation yields `{ "id": 1, "name": "Ada" }`.
        assert_eq!(
            extract_extend_method_body_value(&item),
            "\\{ \"id\": {self.id}, \"name\": \"{self.name}\" \\}"
        );
        // Confirms the outer AST too: an extend on `User` with method `to_json`.
        let shape_ast::ast::Item::Extend(extend, _) = &item else {
            unreachable!("checked above");
        };
        assert!(
            matches!(&extend.type_name, shape_ast::ast::TypeName::Simple(n) if n == "User")
        );
        assert_eq!(extend.methods[0].name, "to_json");
    }

    #[test]
    fn extend_method_assembles_number_and_bool_fields_byte_exact() {
        // Design §4 scalar coverage (review F1): int+string are pinned above;
        // number+bool here. `number`/`bool` fields are NON-string like `int` — the
        // handler encodes their BARE (unquoted) shape into the segments, so the
        // producer emits a bare `{self.<f>}` hole (no surrounding quote segment).
        // A `type Metric { ratio: number, ok: bool }` template.
        let segments = vec![
            "{ \"ratio\": ".to_string(),
            ", \"ok\": ".to_string(),
            " }".to_string(),
        ];
        let field_splices = vec!["ratio".to_string(), "ok".to_string()];
        let item = build_extend_method_item(
            "Metric",
            "to_json",
            &KindedSlot::from_string("string"),
            &segments,
            &field_splices,
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid non-string-scalar template assembles");
        assert_eq!(
            extract_extend_method_body_value(&item),
            "\\{ \"ratio\": {self.ratio}, \"ok\": {self.ok} \\}"
        );
    }

    #[test]
    fn extend_method_rejects_non_identifier_field_splice_c0927() {
        // Condition-1 / condition-3: a field-name-shaped value carrying an
        // injection payload is REJECTED at the builtin boundary, never assembled.
        let err = build_extend_method_item(
            "User",
            "to_json",
            &KindedSlot::from_string("string"),
            &["{ ".to_string(), " }".to_string()],
            &["a} + evil() + {b".to_string()],
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect_err("a non-identifier field splice must reject");
        assert!(
            err.contains("[C0927]"),
            "rejection must carry the named [C0927] diagnostic: {err}"
        );
    }

    #[test]
    fn extend_method_rejects_segment_splice_count_mismatch() {
        // Strict interleave: segments.len() must be field_splices.len() + 1.
        let err = build_extend_method_item(
            "User",
            "to_json",
            &KindedSlot::from_string("string"),
            &["only-one-segment".to_string()],
            &["id".to_string()],
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect_err("a segment/splice count mismatch must reject");
        assert!(
            err.contains("template mismatch"),
            "rejection must name the interleave-invariant violation: {err}"
        );
    }

    // ADR-009 E2 #18 (slice 5b-1) — producer-tier pins for `extend_method_literal`'s
    // literal-body assembly. They exercise `build_extend_method_literal_item`
    // directly (no VM), pinning the AST shape and the byte-exact literal for each of
    // the four literal kinds `literal_expr_from_slot` decodes (int / number / bool /
    // string). The literal decoder is the SAME authority item_fn uses (slice 2), so
    // these pin the extend-method WRAPPING around it, not a second decoder.
    fn extend_method_literal_body(item: &shape_ast::ast::Item) -> shape_ast::ast::Literal {
        use shape_ast::ast::{Expr, Item, Statement};
        let Item::Extend(extend, _) = item else {
            panic!("expected Item::Extend, got a different item kind");
        };
        assert_eq!(extend.methods.len(), 1, "exactly one generated method");
        let method = &extend.methods[0];
        assert_eq!(method.body.len(), 1, "single-statement generated body");
        let Statement::Expression(Expr::Literal(lit, _), _) = &method.body[0] else {
            panic!("generated method body must be a single literal expression");
        };
        lit.clone()
    }

    #[test]
    fn extend_method_literal_builds_int_body_byte_exact() {
        use shape_ast::ast::{Literal, TypeName};
        // The generated_method_runtime `method answer() -> int { 42 }` shape.
        let item = build_extend_method_literal_item(
            "Answer",
            "answer",
            &KindedSlot::from_string("int"),
            &KindedSlot::from_int(42),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid int literal body assembles");
        assert!(matches!(extend_method_literal_body(&item), Literal::Int(42)));
        let shape_ast::ast::Item::Extend(extend, _) = &item else {
            unreachable!("checked in helper");
        };
        assert!(matches!(&extend.type_name, TypeName::Simple(n) if n == "Answer"));
        assert_eq!(extend.methods[0].name, "answer");
    }

    #[test]
    fn extend_method_literal_builds_number_body_byte_exact() {
        use shape_ast::ast::Literal;
        let item = build_extend_method_literal_item(
            "Metric",
            "ratio",
            &KindedSlot::from_string("number"),
            &KindedSlot::from_number(3.5),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid number literal body assembles");
        assert!(matches!(extend_method_literal_body(&item), Literal::Number(n) if n == 3.5));
    }

    #[test]
    fn extend_method_literal_builds_bool_body_byte_exact() {
        use shape_ast::ast::Literal;
        let item = build_extend_method_literal_item(
            "Flag",
            "enabled",
            &KindedSlot::from_string("bool"),
            &KindedSlot::from_bool(true),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid bool literal body assembles");
        assert!(matches!(extend_method_literal_body(&item), Literal::Bool(true)));
    }

    #[test]
    fn extend_method_literal_builds_string_body_byte_exact() {
        use shape_ast::ast::Literal;
        let item = build_extend_method_literal_item(
            "Greeter",
            "greeting",
            &KindedSlot::from_string("string"),
            &KindedSlot::from_string("hello"),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("valid string literal body assembles");
        assert!(matches!(extend_method_literal_body(&item), Literal::String(s) if s == "hello"));
    }

    #[test]
    fn extend_method_literal_rejects_non_identifier_method_name() {
        // The shared assembly labels its identifier diagnostics with the calling
        // producer — a bad method name reports `extend_method_literal`, not
        // `extend_method`.
        let err = build_extend_method_literal_item(
            "Answer",
            "not a method",
            &KindedSlot::from_string("int"),
            &KindedSlot::from_int(42),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect_err("a non-identifier method name must reject");
        assert!(
            err.contains("extend_method_literal") && err.contains("valid method name"),
            "rejection must name the calling producer: {err}"
        );
    }

    #[test]
    fn extend_method_literal_rejects_non_finite_number_body() {
        // `literal_expr_from_slot` (the shared literal decoder) rejects non-finite
        // numbers; the wrapper surfaces that unchanged.
        let err = build_extend_method_literal_item(
            "Metric",
            "ratio",
            &KindedSlot::from_string("number"),
            &KindedSlot::from_number(f64::INFINITY),
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect_err("a non-finite number literal must reject");
        assert!(
            err.contains("finite"),
            "rejection must name the finiteness constraint: {err}"
        );
    }
}

// ADR-009 E2 #18 (slice 5, Part A) — the block-form replace-body carrier's
// per-run-clear no-stale-leak property, pinned DIRECTLY at the store level (the
// definitive proof the supervisor's "double-compile, no stale item across runs"
// asks for: the pre-pass/pass-2 double-compile clears the store per handler run,
// so index 0 of run N never resolves to run N-1's body). The full compile-path
// exercise is the existing lsp/vm replace-body suites, which now flow through
// this carrier (transport-parity arbiter).
#[cfg(test)]
mod replace_body_carrier_tests {
    use super::*;

    fn body_of(src: &str) -> Vec<shape_ast::ast::Statement> {
        shape_ast::parse_program(src)
            .expect("carrier fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func.body),
                _ => None,
            })
            .expect("fixture has one function")
    }

    #[test]
    fn replace_body_carrier_index_restarts_per_run_no_stale_leak() {
        let body_a = body_of("fn a() -> int { return 1 }");
        let body_b = body_of("fn b() -> int { return 2 }");
        assert_ne!(body_a, body_b, "fixtures are distinct");

        // Run 1: clear (handler entry), stash A at index 0.
        clear_comptime_replace_bodies();
        assert_eq!(push_comptime_replace_body(body_a.clone()), 0);
        assert_eq!(comptime_replace_body_at(0).as_ref(), Some(&body_a));

        // Run 2: clear (next handler entry), stash a DIFFERENT body — index
        // restarts at 0, and index 0 now resolves to B, never the stale A.
        clear_comptime_replace_bodies();
        assert_eq!(
            push_comptime_replace_body(body_b.clone()),
            0,
            "index restarts per run"
        );
        let resolved = comptime_replace_body_at(0).expect("live in this run");
        assert_eq!(resolved, body_b, "index 0 resolves to THIS run's body");
        assert_ne!(resolved, body_a, "no stale body leaks across the per-run clear");
    }
}

// ADR-009 E1 #17 (slice 3, U01): the literal-type carrier store + the consumer's
// Int64-index branch. Same lifecycle discipline as the replace-body carrier
// above; these pin the typed transport that replaces the serialize/reparse JSON
// round-trip for literal `set param x: T` / `set return T`.
#[cfg(test)]
mod e1_literal_type_carrier_tests {
    use super::*;
    use shape_ast::ast::TypeAnnotation;

    #[test]
    fn literal_type_carrier_index_restarts_per_run_no_stale_leak() {
        let a = TypeAnnotation::Basic("int".to_string());
        let b = TypeAnnotation::Basic("string".to_string());
        assert_ne!(a, b);

        clear_comptime_directive_types();
        assert_eq!(push_comptime_directive_type(a.clone()), 0);
        assert_eq!(comptime_directive_type_at(0).as_ref(), Some(&a));

        clear_comptime_directive_types();
        assert_eq!(
            push_comptime_directive_type(b.clone()),
            0,
            "index restarts per run"
        );
        let resolved = comptime_directive_type_at(0).expect("live in this run");
        assert_eq!(resolved, b, "index 0 resolves to THIS run's type");
        assert_ne!(resolved, a, "no stale type leaks across the per-run clear");
    }

    #[test]
    fn literal_type_index_resolves_through_the_consumer_without_reparse() {
        // The emit side pushes the typed annotation and bakes its index; the
        // consumer fetches it from an Int64 slot — the exact new U01 path.
        clear_comptime_directive_types();
        let ann = TypeAnnotation::Basic("int".to_string());
        let index = push_comptime_directive_type(ann.clone());
        let slot = KindedSlot::from_int(index as i64);
        let resolved = type_annotation_from_string_or_type_ref_slot(
            &slot,
            "__emit_set_param_type",
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect("Int64 index resolves to the stored typed annotation");
        assert_eq!(resolved, ann);
    }

    #[test]
    fn missing_literal_type_index_is_a_named_error_not_a_panic() {
        clear_comptime_directive_types();
        let slot = KindedSlot::from_int(99);
        let err = type_annotation_from_string_or_type_ref_slot(
            &slot,
            "__emit_set_return_type",
            &semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        )
        .expect_err("an out-of-range index is a named error");
        assert!(err.contains("index 99"), "got: {err}");
    }
}

// ADR-009 E1 #17 (slice 4): the direct-block extend carrier store. Same per-run,
// compile-populated lifecycle as the literal-type and replace-body carriers.
#[cfg(test)]
mod e1_extend_carrier_tests {
    use super::*;

    fn extend_of(src: &str) -> shape_ast::ast::ExtendStatement {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend),
                _ => None,
            })
            .expect("fixture has one extend statement")
    }

    #[test]
    fn extend_carrier_index_restarts_per_run_no_stale_leak() {
        let a = extend_of("extend A { fn m(self) -> int { 1 } }");
        let b = extend_of("extend B { fn m(self) -> int { 2 } }");
        assert_ne!(a, b);

        clear_comptime_extend_statements();
        assert_eq!(push_comptime_extend_statement(a.clone()), 0);
        assert_eq!(comptime_extend_statement_at(0).as_ref(), Some(&a));

        clear_comptime_extend_statements();
        assert_eq!(
            push_comptime_extend_statement(b.clone()),
            0,
            "index restarts per run"
        );
        let resolved = comptime_extend_statement_at(0).expect("live in this run");
        assert_eq!(resolved, b, "index 0 resolves to THIS run's extend");
        assert_ne!(resolved, a, "no stale extend leaks across the per-run clear");
    }

    #[test]
    fn missing_extend_index_resolves_to_none() {
        clear_comptime_extend_statements();
        assert!(comptime_extend_statement_at(7).is_none());
    }
}

// ADR-009 C3 #14 (slice 2): the three hook-template stores' lifecycle pins —
// the same per-run-clear no-stale-leak discipline as the E2/E1 carriers above.
// The body-fn store is COMPILE-populated (cleared at handler-run ENTRY beside
// the replace-body carrier); the template + capture-binding stores are
// EXECUTE-populated (cleared at the pre-execute point beside the checked-items
// store).
#[cfg(test)]
mod hook_template_store_tests {
    use super::*;

    fn def_of(src: &str) -> shape_ast::ast::FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn bound_template(src: &str) -> BoundTemplate {
        let def = def_of(src);
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&def)
            .expect("fixture classifies")
            .captures(shape_ast::ast::CaptureClause {
                entries: Vec::new(),
                span: shape_ast::ast::Span::default(),
            })
            .finish()
            .expect("fixture finishes");
        BoundTemplate {
            template,
            capture_values: Vec::new(),
        }
    }

    #[test]
    fn body_fn_store_index_restarts_per_run_no_stale_leak() {
        let a = def_of("fn a(x: int) -> int { return x }");
        let b = def_of("fn b(x: int) -> int { return x + 1 }");
        assert_ne!(a, b, "fixtures are distinct");

        clear_comptime_template_body_fns();
        assert_eq!(push_comptime_template_body_fn(a.clone()), 0);
        assert_eq!(comptime_template_body_fn_at(0).as_ref(), Some(&a));

        clear_comptime_template_body_fns();
        assert_eq!(
            push_comptime_template_body_fn(b.clone()),
            0,
            "index restarts per run"
        );
        let resolved = comptime_template_body_fn_at(0).expect("live in this run");
        assert_eq!(resolved, b, "index 0 resolves to THIS run's def");
        assert_ne!(resolved, a, "no stale def leaks across the per-run clear");
    }

    #[test]
    fn hook_template_store_index_restarts_per_run_and_reads_clone() {
        let a = bound_template("fn ta(x: int) -> int { return x }");
        let b = bound_template("fn tb(x: int) -> int { return x }");

        clear_comptime_hook_templates();
        assert_eq!(push_comptime_hook_template(a), 0);
        assert_eq!(
            comptime_hook_template_at(0).expect("live").template.body_fn(),
            "ta"
        );
        // Reads clone: the store stays intact until the next per-run clear —
        // the driver resolves template indices AFTER vm.execute returns.
        assert!(comptime_hook_template_at(0).is_some());

        clear_comptime_hook_templates();
        assert_eq!(push_comptime_hook_template(b), 0, "index restarts per run");
        assert_eq!(
            comptime_hook_template_at(0).expect("live").template.body_fn(),
            "tb",
            "index 0 resolves to THIS run's template"
        );
    }

    #[test]
    fn capture_binding_store_index_restarts_per_run() {
        use crate::compiler::template_specialization::const_lift::LiftedConst;

        clear_comptime_capture_bindings();
        assert_eq!(
            push_comptime_capture_binding(("a".to_string(), LiftedConst::Int(1))),
            0
        );
        assert_eq!(
            push_comptime_capture_binding(("b".to_string(), LiftedConst::Bool(true))),
            1
        );
        assert_eq!(
            comptime_capture_binding_at(0),
            Some(("a".to_string(), LiftedConst::Int(1)))
        );

        clear_comptime_capture_bindings();
        assert_eq!(
            push_comptime_capture_binding(("c".to_string(), LiftedConst::Int(2))),
            0,
            "index restarts per run"
        );
        assert_eq!(
            comptime_capture_binding_at(0),
            Some(("c".to_string(), LiftedConst::Int(2))),
            "index 0 resolves to THIS run's binding"
        );
        assert!(comptime_capture_binding_at(1).is_none());
    }
}

// ADR-009 C3 #14 (slice 2): the PUBLIC-API reachability pins — construction
// green paths and named rejections exercised THROUGH the public builtins over
// the full compile path (parse → annotation handler → emit-side rewrite →
// forwarder → mini-VM → builtin → the S1 chokepoint). S1 had no production
// caller, so representative S1 classification / pseudo-tuple sentences are
// pinned here as REACHABLE through `before_hook`/`after_hook`/`capture`.
// A constructed-never-installed template is a no-op — every green fixture
// must still compile and run its program (install is S2b).
#[cfg(test)]
mod hook_template_builtin_tests {
    use super::*;

    /// A whole program: `body_fns` at module scope, an annotation whose
    /// comptime post handler runs `handler_stmts`, applied to a victim fn.
    fn hook_program(body_fns: &str, handler_stmts: &str) -> String {
        format!(
            r#"
{body_fns}

annotation hookann() on function {{
  comptime post(target, ctx) {{
    {handler_stmts}
  }}
}}

@hookann()
fn victim(a: int) -> int {{ return a }}

victim(1)
"#
        )
    }

    fn compile(src: &str) -> shape_ast::error::Result<()> {
        let program = shape_ast::parse_program(src).expect("fixture parses");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler.compile_in_place(&program).map(|_| ())
    }

    fn expect_compile_reject(src: &str, needle: &str) {
        let err = compile(src).expect_err("fixture must reject");
        let text = err.to_string();
        assert!(
            text.contains(needle),
            "expected rejection containing {needle:?}, got: {text}"
        );
    }

    // ── GREEN construction paths ────────────────────────────────────────────

    #[test]
    fn concrete_no_capture_before_constructs_through_the_public_api() {
        compile(&hook_program(
            "fn my_before(x: int) -> int { return x + 1 }",
            "let t = before_hook(my_before, [])",
        ))
        .expect("a concrete no-capture before-template constructs and the program compiles");
    }

    #[test]
    fn concrete_with_captures_constructs_through_the_public_api() {
        compile(&hook_program(
            "fn my_before(x: int, threshold: int) -> int { return x + threshold }",
            "let t = before_hook(my_before, [capture(\"threshold\", 5)])",
        ))
        .expect("a concrete before-template with a scalar capture constructs");
    }

    #[test]
    fn polymorphic_before_with_captures_constructs_through_the_public_api() {
        compile(&hook_program(
            "fn my_before<Args>(args: Args, factor: int) -> Args { return args }",
            "let t = before_hook(my_before, [capture(\"factor\", 3)])",
        ))
        .expect("a polymorphic before-template with a scalar capture constructs");
    }

    #[test]
    fn polymorphic_after_constructs_through_the_public_api() {
        compile(&hook_program(
            "fn my_after<R>(result: R) -> R { return result }",
            "let t = after_hook(my_after, [])",
        ))
        .expect("a polymorphic after-template constructs");
    }

    // ── Named rejections through the public API ─────────────────────────────

    #[test]
    fn string_body_arg_is_rejected_code_is_code() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int) -> int { return x }",
                "let t = before_hook(\"my_before\", [])",
            ),
            "bare module-scope fn identifier",
        );
    }

    #[test]
    fn unresolvable_body_fn_is_rejected_with_module_scope_twin() {
        expect_compile_reject(
            &hook_program(
                "fn unrelated(x: int) -> int { return x }",
                "let t = before_hook(nope, [])",
            ),
            "does not resolve to a module-scope fn",
        );
    }

    // REACHABILITY PIN: the S1 classification sentences surface through the
    // public builtin (S1 had no production caller).
    #[test]
    fn s1_classification_sentence_two_type_params_reaches_through_the_api() {
        expect_compile_reject(
            &hook_program(
                "fn bad<A, B>(x: A) -> A { return x }",
                "let t = before_hook(bad, [])",
            ),
            "declares 2 type parameters",
        );
    }

    #[test]
    fn s1_classification_sentence_non_t_return_reaches_through_the_api() {
        expect_compile_reject(
            &hook_program(
                "fn bad<R>(result: R) -> int { return 0 }",
                "let t = after_hook(bad, [])",
            ),
            "returns `int`",
        );
    }

    // REACHABILITY PIN: the S1 pseudo-tuple sentences surface through the
    // public builtin.
    #[test]
    fn s1_pseudo_tuple_sentence_reaches_through_the_api() {
        expect_compile_reject(
            &hook_program(
                "fn bad<Args>(args: Args) -> Args {\n  let i = 0\n  args[i] = 1\n  return args\n}",
                "let t = before_hook(bad, [])",
            ),
            "compile-time-constant index",
        );
    }

    #[test]
    fn duplicate_capture_rejects_c0907_through_the_api() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int, cfg: int) -> int { return x }",
                "let t = before_hook(my_before, [capture(\"cfg\", 1), capture(\"cfg\", 2)])",
            ),
            "[C0907]",
        );
    }

    #[test]
    fn capture_bijection_miss_rejects_through_the_api() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int, cfg: int) -> int { return x }",
                "let t = before_hook(my_before, [capture(\"wrong\", 1)])",
            ),
            "matches none of its 1 trailing capture parameter(s) [cfg]",
        );
    }

    #[test]
    fn capture_value_type_mismatch_rejects_naming_both_sides() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int, cfg: string) -> int { return x }",
                "let t = before_hook(my_before, [capture(\"cfg\", 1)])",
            ),
            "holds a int value but the matching trailing capture parameter is annotated `string`",
        );
    }

    // S3b PIN FLIP (ordered by the slice-3 charter), RE-TARGETED in S4a: a
    // non-liftable heap value rejects through the S3 out-of-domain producer,
    // naming the kind and the closed C3-G5 domain — proven end-to-end
    // through the public API with a HashMap capture value (statically typed,
    // so the S4a generic `capture<T>` forwarder instantiates and the value
    // REACHES the execute-time lift wall). The S3-era spelling used the
    // handler's own `target` descriptor; its inline-Object annotation is
    // outside the `ConcreteType` algebra, so under per-call-site value
    // typing that spelling now rejects EARLIER at the capture call site —
    // locked by the sibling pin below. (#65 note: an INLINE array-literal
    // capture value `[capture("cfg", [1, 2])]` still trips the PRE-EXISTING
    // `pending_variable_typed_array_kind` leak — the S3b composite fixtures
    // hoist to a local first; #65 itself is not fixed in S3.)
    #[test]
    fn non_liftable_capture_value_rejects_with_the_s3_domain_sentence() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int, cfg: int) -> int { return x }",
                "let m: HashMap<string, int> = HashMap()\n    \
                 let t = before_hook(my_before, [capture(\"cfg\", m)])",
            ),
            "outside the ConstLift domain (kind Ptr(HashMap))",
        );
    }

    // S4a (#66 item 1) DISCLOSED CONSEQUENCE of per-call-site capture value
    // typing: a capture VALUE whose static type does not resolve in the
    // `ConcreteType` algebra (here the handler's `target` param — its inline
    // `TypeAnnotation::Object` annotation is the S2d-measured proof-gap (a)
    // family, S7's named follow-up) can no longer instantiate the generic
    // `capture<T>` forwarder, so the rejection fires LOUDLY at COMPILE time
    // with the established generic-inference sentence instead of reaching
    // the execute-time S3 lift wall. Still surface-and-stop — never a
    // silent narrowing; the S3 value-tier producers stay reachable for
    // resolvable-typed values (the pin above) and are unit-locked per class
    // arm in `const_lift.rs`.
    #[test]
    fn target_descriptor_capture_value_is_a_loud_compile_time_inference_rejection() {
        expect_compile_reject(
            &hook_program(
                "fn my_before(x: int, cfg: int) -> int { return x }",
                "let t = before_hook(my_before, [capture(\"cfg\", target)])",
            ),
            "cannot infer type argument(s) for generic function 'capture'",
        );
    }

    // The compile-populated body-fn store SURVIVES to execute: proven by every
    // green path above (the builtin resolves the def stashed at rewrite time
    // during vm.execute). This control additionally proves two templates in
    // ONE handler run get distinct indices.
    #[test]
    fn two_templates_in_one_handler_run_both_construct() {
        compile(&hook_program(
            "fn h1(x: int) -> int { return x }\nfn h2<R>(result: R) -> R { return result }",
            "let a = before_hook(h1, [])\n    let b = after_hook(h2, [])",
        ))
        .expect("two templates in one run construct with distinct store indices");
    }
}

// ADR-009 E1 #17 slice-5 A-FULL — STOP-shaped composite-boundary pins (E1-D7).
//
// Stage 1 locks the RULED A-FULL reconstructable frontier in-code. The
// descriptor algebra reconstructs primitive leaves + Never + base `any` +
// Tuple + Reference + Union + Callable(round-tripping); every nominal-headed
// form (applied generics `Array<int>`/`Option<T>`/`HashMap`, records, bare
// user-nominals, un-applied heads) is a NAMED rejection at `payload_of`, NOT
// reconstructable in E1. (HISTORICAL: at this stage applied generics stamp-gated
// to unstamped (identity = INVALID) → the `__ComptimeTypeRef.source` reparse arm.
// CKPT-1..3 later widened the gate so those forms STAMP, and E5 CKPT-5 DELETED the
// `.source` field + reparse arm entirely — an INVALID ref is now a NAMED consumer
// rejection.) This is exactly E1-D7(a) unstamped-fall-through + E1-D7(c) "every
// variant handled OR a named error" (the nominal-headed variants ARE handled — as
// named errors).
//
// Plain `#[cfg(test)]` (NOT deep-tests-gated) so the standard supervisor gate
// runs them (E2 finding 5: pins in a deep-tests-gated module never run). No
// production code changes this stage, so all recorded baselines are unmoved by
// construction.
#[cfg(test)]
mod e1_s5_boundary {
    use super::reconstruct_type_annotation;
    use super::semantic_freeze::overlay_for_tests;
    use super::type_reflection::payloads::{self, FrozenPayloadDescriptor};
    use super::type_reflection::type_argument;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::TypeAnnotation;

    // ADR-009 E5 CKPT-2 (A8-OUT — the POSITIVE FLIP of the former pin 3747
    // `e1_s5_applied_nominal_is_pending_rejection_not_reconstructable`): SP-1.
    // `payload_of(Array<int>)` DESCRIPTOR substitution now RESOLVES — `Array`
    // is a container, so the A8-OUT template answers `Opaque{owner: Array-head}`
    // (A7: a container has NO named-field/variant structure to state, so
    // `Opaque` is the honest "no rows to show", never a mis-stated field). The
    // element type is NOT in the descriptor — it is recovered by the orthogonal
    // `type_argument`/`arg_identities` query. This is the soundness-critical
    // surface: the descriptor never fabricates a member type; the arg lives in
    // `type_argument`. `applied_nominal_pending_rejection` now fires ONLY for a
    // head that is neither a builtin template, a user enum, nor a resolved user
    // struct (never a silent gap).
    #[test]
    fn e1_s5_applied_container_descriptor_substitutes_to_opaque_over_recoverable_arg() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        // `Array<int>` as the compiler sees it at from_function/from_type — a
        // real `TypeAnnotation`, built directly to avoid any grammar dependence.
        // `TypeAnnotation::Array(int)` routes through `canonical_applied`,
        // identical to `Generic{"Array",[int]}`.
        let array_int = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));

        let identity = overlay
            .canonicalize_type_projection(&array_int)
            .expect("Array<int> canonicalizes to a Nominal identity")
            .identity();

        let applied = overlay
            .applied_nominal_of(identity)
            .expect("Array<int> is a site-interned applied form");

        // The descriptor is `Opaque` with `owner == the Array HEAD identity`
        // (never the applied identity) — NOT pending, NOT Newtype, NOT Struct.
        match overlay.payload_of(identity) {
            Ok(FrozenPayloadDescriptor::Nominal(payloads::NominalDescriptor::Opaque { owner })) => {
                assert_eq!(
                    owner, applied.head_identity,
                    "the Opaque owner is the Array HEAD identity (owner: head), \
                     so Array<int> / Array<string> share owner"
                );
            }
            other => panic!(
                "Array<int> must descriptor-substitute to Opaque (A8-OUT container), \
                 never Newtype (would mis-imply a 1:1 wrapper over one int), never a \
                 pending rejection; got {other:?}"
            ),
        }

        // A7 recovery: the element type is recovered via the orthogonal
        // `type_argument` query, NEVER stated in the descriptor.
        assert_eq!(
            applied.arg_identities.len(),
            1,
            "Array<int> carries exactly one type argument"
        );
        let arg0 = type_argument(&applied, 0).expect("Array<int> arg 0");
        assert_eq!(
            reconstruct_type_annotation(&overlay, arg0)
                .expect("the recovered element identity spells"),
            TypeAnnotation::Basic("int".to_string()),
            "the element type is recovered as `int` via type_argument (A7), \
             not fabricated into the descriptor"
        );
    }

    // The POSITIVE boundary: a `Tuple[int, string]` reconstructs via the
    // descriptor algebra — `payload_of` returns `Tuple` with two ordered
    // element identities that each themselves `payload_of` to `Primitive`. This
    // locks the reconstructable frontier stage 2's total reconstruction fn
    // targets, and proves composites-that-round-trip (Tuple) are INSIDE A-FULL
    // while applied nominals (above) are not.
    #[test]
    fn e1_s5_tuple_int_string_reconstructs_via_descriptor() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let tuple_int_string = TypeAnnotation::Tuple(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]);

        let identity = overlay
            .canonicalize_type_projection(&tuple_int_string)
            .expect("[int, string] canonicalizes to a Tuple identity")
            .identity();

        let elements = match overlay.payload_of(identity) {
            Ok(FrozenPayloadDescriptor::Tuple(descriptor)) => descriptor.elements,
            other => panic!(
                "[int, string] must reconstruct as a Tuple descriptor (inside the \
                 A-FULL reconstructable frontier); got {other:?}"
            ),
        };
        assert_eq!(
            elements.len(),
            2,
            "the tuple carries its two ordered element identities"
        );

        for element in elements {
            assert!(
                matches!(
                    overlay.payload_of(element),
                    Ok(FrozenPayloadDescriptor::Primitive(_))
                ),
                "each tuple element is a primitive leaf that itself reconstructs \
                 off its own descriptor (int / string)"
            );
        }
    }
}

// ADR-009 E1 #17 slice-5 A-FULL — STAGE 2 reconstruction pins (E1-D7(b)/(c)).
//
// The total inverse (`reconstruct_type_annotation`) exercised directly. These
// keep the (still-unwired, stage-4-live) reconstruction fn `used` and lock:
// (1) primitive inversion returns the ONE table's canonical (names[0]) spelling
// — a synonym reconstructs to its family head, never the input spelling
// (E1-D7(c) "one name table, inverted"); (2) composites recurse the descriptor
// algebra (Tuple → element identities → primitives); (3) TOTALITY — every
// non-reconstructable identity (applied nominal, record, bare generic head) is
// a NAMED `ShapeError::SemanticError`, no panic, no catch-all silent arm. All
// three are plain `#[cfg(test)]` so the standard supervisor gate runs them.
//
// STAGE 2 was behavior-preserving at the time: the live directive consumer still
// reparsed `.source` (every producer stamp was INVALID that stage), so no baseline
// moved. (HISTORICAL: STAGE 4 flipped the consumer to identity-only and E5 CKPT-5
// DELETED the `.source` field + reparse arm; these reconstruction pins now assert
// the sole resolution route.)
#[cfg(test)]
mod e1_s5_reconstruction {
    use super::reconstruct_type_annotation;
    use super::semantic_freeze::overlay_for_tests;
    use super::type_reflection::payloads;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
    use shape_ast::error::ShapeError;

    // PIN (c). A frozen PRIMITIVE reconstructs to the ONE
    // `PRIMITIVE_SYNONYM_FAMILIES` table's canonical (`names[0]`) spelling —
    // proven by feeding a SYNONYM (`str`, `i64`, `f64`) and getting the family
    // HEAD back (`string`, `int`, `number`), never the input spelling. This is
    // the "no second name table" obligation made observable: the forward
    // classifier and the reverse speller share the ONE table.
    #[test]
    fn e1_s5_reconstruct_primitive_inverts_synonym_family_to_canonical() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        // (input spelling, expected canonical head). The four families the
        // stage instructions name — String / Bool / SignedInteger(W64) /
        // BinaryFloat(W64) — each probed via a synonym that differs from its
        // head where one exists.
        let cases = [
            ("string", "string"),
            ("str", "string"),
            ("bool", "bool"),
            ("int", "int"),
            ("i64", "int"),
            ("number", "number"),
            ("f64", "number"),
            ("float", "number"),
        ];
        for (input, canonical) in cases {
            let identity = overlay
                .canonicalize_type(&TypeAnnotation::Basic(input.to_string()))
                .unwrap_or_else(|error| panic!("leaf '{input}' canonicalizes: {error}"));
            let reconstructed = reconstruct_type_annotation(&overlay, identity)
                .unwrap_or_else(|error| panic!("leaf '{input}' reconstructs: {error:?}"));
            assert_eq!(
                reconstructed,
                TypeAnnotation::Basic(canonical.to_string()),
                "primitive '{input}' must reconstruct to its family's canonical \
                 spelling '{canonical}' (names[0]), not the input spelling"
            );
        }
    }

    // PIN (composite). `[int, string]` reconstructs by recursing the Tuple
    // descriptor's ordered element identities — each element itself
    // reconstructs off its own primitive descriptor. The composite round-trips
    // to the byte-identical annotation, proving the algebra recurses TOTALLY
    // (not just at the leaf).
    #[test]
    fn e1_s5_reconstruct_tuple_recurses_element_identities() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let tuple = TypeAnnotation::Tuple(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]);
        let identity = overlay
            .canonicalize_type(&tuple)
            .expect("[int, string] canonicalizes to a Tuple identity");
        let reconstructed =
            reconstruct_type_annotation(&overlay, identity).expect("[int, string] reconstructs");
        assert_eq!(
            reconstructed,
            TypeAnnotation::Tuple(vec![
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Basic("string".to_string()),
            ]),
            "the Tuple reconstructs by recursing its two element identities"
        );
    }

    // PIN (d) TOTALITY (ADR-009 E5 CKPT-1 case (1) FLIPPED; CKPT-3 case (2)
    // FLIPPED). The reconstruction is TOTAL — every identity either reconstructs
    // structurally or yields a NAMED `ShapeError::SemanticError`, no panic, no
    // catch-all silent arm (E1-D7(c)): (1) an APPLIED nominal (`Array<int>`) SPELLS
    // to `Generic{Array, [int]}` via the CKPT-1 applied-nominal arm; (2) a
    // structural RECORD (`{x: int, y: string}`) now SPELLS to `Object({x: int,
    // y: string})` via the CKPT-3 field-name-preservation arm — FLIPPED from the
    // pre-CKPT-3 named record rejection; (3) a BARE generic head (`Array`) STAYS
    // the un-applied-head rejection (ruling A3). Case (3) stays INVALID → a NAMED
    // consumer rejection (the `.source` reparse arm is DELETED at E5 CKPT-5); cases
    // (1)+(2) now stamp.
    #[test]
    fn e1_s5_reconstruct_covers_frozen_payload_descriptor_totally() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());

        // (1) Applied nominal — CKPT-1 FLIP: now SPELLS to `Generic{Array,[int]}`
        // (the applied-nominal arm reconstructs off the frozen memo, no reparse).
        let array_int = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        let applied_identity = overlay
            .canonicalize_type(&array_int)
            .expect("Array<int> canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, applied_identity)
                .expect("Array<int> now SPELLS via the CKPT-1 applied-nominal arm"),
            TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Basic("int".to_string())],
            },
            "Array<int> reconstructs to the applied-generic spelling `Array<int>` (CKPT-1)"
        );

        // (2) Structural record — CKPT-3 FLIP: now SPELLS to `Object({x: int,
        // y: string})` off the preserved field names (byte-sorted). Was a named
        // record rejection pre-CKPT-3.
        let record = TypeAnnotation::Object(vec![
            ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: Vec::new(),
            },
            ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("string".to_string()),
                annotations: Vec::new(),
            },
        ]);
        let record_identity = overlay
            .canonicalize_type(&record)
            .expect("{x: int, y: string} canonicalizes to a Record identity");
        assert_eq!(
            reconstruct_type_annotation(&overlay, record_identity)
                .expect("{x: int, y: string} now SPELLS via the CKPT-3 record arm"),
            TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: Vec::new(),
                },
                ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("string".to_string()),
                    annotations: Vec::new(),
                },
            ]),
            "the record reconstructs to `{{x: int, y: string}}` (names + byte-sort preserved)"
        );

        // (3) Bare generic head — Err propagated from `payload_of` via `?`.
        let bare_head = TypeAnnotation::Basic("Array".to_string());
        let head_identity = overlay
            .canonicalize_type(&bare_head)
            .expect("bare `Array` head canonicalizes to a Nominal identity");
        match reconstruct_type_annotation(&overlay, head_identity) {
            Err(ShapeError::SemanticError { message, .. }) => assert_eq!(
                message,
                payloads::unapplied_generic_head_rejection(),
                "a bare generic head reconstruction surfaces the un-applied-head named error"
            ),
            other => panic!("a bare generic head must be a named SemanticError, got {other:?}"),
        }
    }

    // ADR-009 E5 CKPT-1 (design §1a): the applied-nominal SPELLING arm. Each
    // applied builtin generic reconstructs to its `Generic{head, args}` spelling
    // DIRECTLY off the frozen memo (`applied_nominal_of` → head via
    // `type_names_for_identity`, args recursed) — never a `.source` reparse. The
    // head reverses to its canonical builtin name; each primitive arg to its
    // canonical leaf spelling. `Array<int>` is built as the `Array(_)` sugar the
    // compiler emits, and still spells to the uniform `Generic{Array,[..]}` form.
    #[test]
    fn e1_s5_reconstruct_applied_builtin_generics_spell_head_and_args() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let basic = |name: &str| TypeAnnotation::Basic(name.to_string());
        let generic = |name: &str, args: Vec<TypeAnnotation>| TypeAnnotation::Generic {
            name: TypePath::simple(name),
            args,
        };
        let cases: Vec<(TypeAnnotation, TypeAnnotation)> = vec![
            (
                TypeAnnotation::Array(Box::new(basic("int"))),
                generic("Array", vec![basic("int")]),
            ),
            (
                generic("Option", vec![basic("int")]),
                generic("Option", vec![basic("int")]),
            ),
            (
                generic("HashMap", vec![basic("string"), basic("int")]),
                generic("HashMap", vec![basic("string"), basic("int")]),
            ),
            (
                generic("Result", vec![basic("int"), basic("string")]),
                generic("Result", vec![basic("int"), basic("string")]),
            ),
        ];
        for (input, expected) in cases {
            let identity = overlay
                .canonicalize_type(&input)
                .unwrap_or_else(|e| panic!("{input:?} canonicalizes: {e}"));
            let reconstructed = reconstruct_type_annotation(&overlay, identity)
                .unwrap_or_else(|e| panic!("{input:?} spells via the applied-nominal arm: {e:?}"));
            assert_eq!(
                reconstructed, expected,
                "applied generic {input:?} reconstructs to its head+args spelling"
            );
        }
    }

    // ADR-009 E5 CKPT-1 (design §1a, A2 identity-indirected-recursion invariant):
    // a NESTED applied generic (`Array<Option<int>>`) spells to its nested
    // `Generic{Array,[Generic{Option,[int]}]}` and TERMINATES. The inner
    // `Option<int>` identity resolves off the SAME shared memo (projection.rs
    // CKPT-1 recursively memoized every composite sub-expression); the recursion
    // is identity-indirected over the finite `arg_identities`, NEVER an eager
    // field expansion.
    #[test]
    fn e1_s5_reconstruct_nested_applied_generic_terminates_and_spells() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let nested = TypeAnnotation::Array(Box::new(TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        }));
        let identity = overlay
            .canonicalize_type(&nested)
            .expect("Array<Option<int>> canonicalizes");
        let reconstructed = reconstruct_type_annotation(&overlay, identity)
            .expect("Array<Option<int>> spells + terminates via identity-indirected recursion");
        assert_eq!(
            reconstructed,
            TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Generic {
                    name: TypePath::simple("Option"),
                    args: vec![TypeAnnotation::Basic("int".to_string())],
                }],
            },
            "the nested applied generic reconstructs its head + its applied arg"
        );
    }

    // ADR-009 E5 CKPT-1 (design §1a): a bare RESOLVED user nominal spells as
    // `Basic(name)` via `bare_nominal_name_of` (a read of the frozen nominal
    // descriptor). An un-applied generic HEAD stays a named rejection (A3),
    // proven in the totality pin's case (3).
    #[test]
    fn e1_s5_reconstruct_bare_user_nominal_spells_as_basic() {
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "User".to_string(),
            (vec!["id".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "User".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [("id".to_string(), TypeAnnotation::Basic("int".to_string()))]
                    .into_iter()
                    .collect(),
            },
        );
        let overlay = overlay_for_tests(&compiler);
        let identity = overlay
            .canonicalize_type(&TypeAnnotation::Basic("User".to_string()))
            .expect("bare `User` nominal canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, identity)
                .expect("a resolved bare user nominal spells as Basic(name)"),
            TypeAnnotation::Basic("User".to_string()),
            "bare user nominal `User` reconstructs to Basic(\"User\")"
        );
    }

    // ADR-009 E5 CKPT-1: the STAMP-GATE AUTO-WIDEN pinned on the gate PREDICATE.
    // `stamp_for` (comptime_target.rs) admits an identity iff
    // `reconstruct_type_annotation(...).is_ok()`. Pre-CKPT-1 an applied-generic
    // reconstruct was `Err` → `INVALID` stamp → `.source` reparse. The moment the
    // CKPT-1 arm reconstructs it, the SAME predicate is `Ok`, so the producer
    // STAMPS it — E1-D7(b) one code path, NO `stamp_for` edit. Pinned directly on
    // the applied + nested forms.
    #[test]
    fn e1_s5_stamp_gate_predicate_auto_widens_for_applied_generics() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let b = |name: &str| TypeAnnotation::Basic(name.to_string());
        let g = |name: &str, args: Vec<TypeAnnotation>| TypeAnnotation::Generic {
            name: TypePath::simple(name),
            args,
        };
        for form in [
            TypeAnnotation::Array(Box::new(b("int"))),
            g("Option", vec![b("int")]),
            g("HashMap", vec![b("string"), b("int")]),
            g("Result", vec![b("int"), b("string")]),
            TypeAnnotation::Array(Box::new(g("Option", vec![b("int")]))),
        ] {
            let identity = overlay
                .canonicalize_type(&form)
                .unwrap_or_else(|e| panic!("{form:?} canonicalizes: {e}"));
            assert!(
                reconstruct_type_annotation(&overlay, identity).is_ok(),
                "the shared stamp-gate predicate reconstruct().is_ok() must now ADMIT \
                 {form:?} — the applied-generic auto-widen"
            );
        }
    }

    // ADR-009 E5 CKPT-3 — the NON-PERTURBATION pin (the CKPT-0 binding invariant).
    // Field-name preservation is ADDITIVE: the record's own frozen IDENTITY and
    // each field's hygienic MEMBER identity stay BYTE-IDENTICAL across CKPT-3. The
    // 128-bit identity + member halves are pinned to the CONCRETE pre-CKPT-3 values
    // captured on HEAD 1d54eb67, so any future edit that threads the field NAME into
    // the identity descriptor string or `record_member_identity` (the unsoundness
    // this invariant exists to prevent) breaks this pin LOUDLY. Optionality is
    // identity-significant: `{x?:int}` mints a DIFFERENT identity than `{x:int}`.
    #[test]
    fn e1_s5_ckpt3_record_identity_and_member_ids_are_byte_identical() {
        use super::semantic_freeze::FreezeOverlay;
        use super::type_reflection::FrozenTypeIdentity;
        use payloads::FrozenPayloadDescriptor;

        // Extract each field's (name, member.high, member.low, optional), in the
        // descriptor's byte-sorted order. `&Arc<FreezeOverlay>` deref-coerces to
        // `&FreezeOverlay` at this fn's call site.
        fn members(
            overlay: &FreezeOverlay,
            id: FrozenTypeIdentity,
        ) -> Vec<(String, i64, i64, bool)> {
            let FrozenPayloadDescriptor::Record(desc) =
                overlay.payload_of(id).expect("record payload")
            else {
                panic!("expected a Record payload");
            };
            desc.fields
                .iter()
                .map(|f| (f.name.clone(), f.member.high, f.member.low, f.optional))
                .collect()
        }

        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let field = |name: &str, optional: bool, ty: &str| ObjectTypeField {
            name: name.to_string(),
            optional,
            type_annotation: TypeAnnotation::Basic(ty.to_string()),
            annotations: Vec::new(),
        };

        // {x: int, y: string} — identity + both member ids frozen to pre-CKPT-3.
        let xy = TypeAnnotation::Object(vec![field("x", false, "int"), field("y", false, "string")]);
        let xy_id = overlay
            .canonicalize_type(&xy)
            .expect("{x:int,y:string} canonicalizes");
        assert_eq!(
            (xy_id.high, xy_id.low),
            (4972967358956473603, -5404863359470070500),
            "the record identity must be byte-identical to the pre-CKPT-3 value"
        );
        assert_eq!(
            members(&overlay, xy_id),
            vec![
                ("x".to_string(), 5117747860848310177, 1031105497090630829, false),
                ("y".to_string(), -9035473693977959263, 304561787195158326, false),
            ],
            "member identities + byte-sort order (x before y) must be byte-identical to pre-CKPT-3"
        );

        // {x?: int} — a DISTINCT identity (optionality-significant) + its member id.
        let x_opt = TypeAnnotation::Object(vec![field("x", true, "int")]);
        let x_opt_id = overlay
            .canonicalize_type(&x_opt)
            .expect("{x?:int} canonicalizes");
        assert_ne!(
            (x_opt_id.high, x_opt_id.low),
            (xy_id.high, xy_id.low),
            "optionality is identity-significant — {{x?:int}} != {{x:int,y:string}}"
        );
        assert_eq!(
            (x_opt_id.high, x_opt_id.low),
            (-1802259954908786269, -200733891727391745),
            "the optional-field record identity must be byte-identical to pre-CKPT-3"
        );
        assert_eq!(
            members(&overlay, x_opt_id),
            vec![("x".to_string(), 7472345934218968096, -929543014868829712, true)],
            "the optional field's member id must be byte-identical to pre-CKPT-3"
        );
    }

    // ADR-009 E5 CKPT-3 — record SPELLING round-trip. A structural record
    // reconstructs to `Object({name: T, …})` off the preserved field names, with
    // optionality PRESERVED per field (`{x?:int}` keeps the `?`) and byte-sorted
    // field order (x before y).
    #[test]
    fn e1_s5_ckpt3_record_spells_names_and_optionality() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let b = |name: &str| TypeAnnotation::Basic(name.to_string());
        let field = |name: &str, optional: bool, ty: TypeAnnotation| ObjectTypeField {
            name: name.to_string(),
            optional,
            type_annotation: ty,
            annotations: Vec::new(),
        };

        // {x: int, y: string} round-trips to itself (already byte-sorted).
        let xy = TypeAnnotation::Object(vec![
            field("x", false, b("int")),
            field("y", false, b("string")),
        ]);
        let xy_id = overlay.canonicalize_type(&xy).expect("{x:int,y:string} canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, xy_id).expect("record spells via CKPT-3 arm"),
            TypeAnnotation::Object(vec![
                field("x", false, b("int")),
                field("y", false, b("string")),
            ]),
            "{{x:int,y:string}} round-trips to itself with names preserved"
        );

        // {x?: int} preserves the optional `?`.
        let x_opt = TypeAnnotation::Object(vec![field("x", true, b("int"))]);
        let x_opt_id = overlay.canonicalize_type(&x_opt).expect("{x?:int} canonicalizes");
        let spelled =
            reconstruct_type_annotation(&overlay, x_opt_id).expect("optional record spells");
        assert_eq!(
            spelled,
            TypeAnnotation::Object(vec![field("x", true, b("int"))]),
            "{{x?:int}} spells with the optional flag preserved"
        );
        let TypeAnnotation::Object(fields) = &spelled else {
            panic!("expected an Object spelling");
        };
        assert!(fields[0].optional, "the `?` must survive reconstruction");
    }

    // ADR-009 E5 CKPT-3 (A2 identity-indirected): a record whose field types are
    // themselves a nested RECORD and an APPLIED generic reconstructs + TERMINATES.
    // Each field type recurses on its own finite frozen identity — the inner record
    // spells its own fields, the applied arg spells by head+args — never an eager
    // unbounded expansion.
    #[test]
    fn e1_s5_ckpt3_record_with_nested_record_and_applied_field_terminates() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let b = |name: &str| TypeAnnotation::Basic(name.to_string());
        let field = |name: &str, ty: TypeAnnotation| ObjectTypeField {
            name: name.to_string(),
            optional: false,
            type_annotation: ty,
            annotations: Vec::new(),
        };
        // { inner: {a: int}, items: Array<int> }  (byte-sort: "inner" < "items")
        let nested = TypeAnnotation::Object(vec![
            field("inner", TypeAnnotation::Object(vec![field("a", b("int"))])),
            field("items", TypeAnnotation::Array(Box::new(b("int")))),
        ]);
        let id = overlay
            .canonicalize_type(&nested)
            .expect("nested record canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, id)
                .expect("nested record spells + terminates via identity-indirected recursion"),
            TypeAnnotation::Object(vec![
                field("inner", TypeAnnotation::Object(vec![field("a", b("int"))])),
                field(
                    "items",
                    TypeAnnotation::Generic {
                        name: TypePath::simple("Array"),
                        args: vec![b("int")],
                    },
                ),
            ]),
            "the nested record spells its inner record + its applied `Array<int>` field, byte-sorted"
        );
    }

    // ADR-009 E5 CKPT-3: the stamp-gate AUTO-WIDENS for records — the SAME
    // `reconstruct(...).is_ok()` predicate `stamp_for` uses now ADMITS a structural
    // record, so producers stamp it + the consumer stops hitting `.source`. No
    // `stamp_for` edit (E1-D7(b), one code path).
    #[test]
    fn e1_s5_ckpt3_stamp_gate_predicate_auto_widens_for_records() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let field = |name: &str, optional: bool, ty: &str| ObjectTypeField {
            name: name.to_string(),
            optional,
            type_annotation: TypeAnnotation::Basic(ty.to_string()),
            annotations: Vec::new(),
        };
        for form in [
            TypeAnnotation::Object(vec![field("x", false, "int"), field("y", false, "string")]),
            TypeAnnotation::Object(vec![field("x", true, "int")]),
        ] {
            let identity = overlay
                .canonicalize_type(&form)
                .unwrap_or_else(|e| panic!("{form:?} canonicalizes: {e}"));
            assert!(
                reconstruct_type_annotation(&overlay, identity).is_ok(),
                "the shared stamp-gate predicate reconstruct().is_ok() must now ADMIT \
                 the record {form:?} — the CKPT-3 record auto-widen"
            );
        }
    }

    // ADR-009 E5 CKPT-4 (the deferred CKPT-3 termination pin, folded in). A
    // recursive NAMED record `type Tree { kids: Array<Tree> }` reconstructs +
    // TERMINATES: the nominal self-reference `Tree` inside `Array<Tree>` resolves
    // to the BARE-NAME leaf `Basic("Tree")` (via `bare_nominal_name_of`), never
    // field-expanding `Tree` — the A2 identity-indirected recursion invariant
    // applied to a NOMINAL self-ref (distinct from the anonymous-record nesting
    // pinned by `e1_s5_ckpt3_record_with_nested_record_and_applied_field_terminates`).
    // So reconstructing the field type `Array<Tree>` spells `Array<Tree>` (head +
    // bare-name arg) and STOPS — it does not descend into `Tree` forever.
    #[test]
    fn e1_s5_ckpt4_recursive_named_record_reconstructs_and_terminates() {
        use shape_ast::ast::TypePath;
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "Tree".to_string(),
            (vec!["kids".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Tree".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "kids".to_string(),
                    TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("Tree".to_string()))),
                )]
                .into_iter()
                .collect(),
            },
        );
        let overlay = overlay_for_tests(&compiler);

        // The nominal self-ref `Tree` is a bare-name LEAF — it terminates.
        let tree_id = overlay
            .canonicalize_type(&TypeAnnotation::Basic("Tree".to_string()))
            .expect("bare `Tree` nominal canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, tree_id)
                .expect("the named self-ref resolves to a bare-name leaf"),
            TypeAnnotation::Basic("Tree".to_string()),
            "recursive nominal `Tree` reconstructs to Basic(\"Tree\"), never expanding its fields"
        );

        // The field type `Array<Tree>` spells head + bare-name arg, then STOPS.
        let field_ty = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("Tree".to_string())));
        let field_id = overlay
            .canonicalize_type(&field_ty)
            .expect("Array<Tree> canonicalizes");
        assert_eq!(
            reconstruct_type_annotation(&overlay, field_id)
                .expect("Array<Tree> spells + terminates (identity-indirected recursion)"),
            TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Basic("Tree".to_string())],
            },
            "Array<Tree> spells its head + the bare-name self-ref arg, terminating"
        );
    }
}

// ADR-009 E1 #17 slice-5 A-FULL — STAGE 5 route-proof pins (E1-D7(a)/(b)/(c)),
// E5 CKPT-5 FALLBACK DELETED.
//
// A STAMPED `__ComptimeTypeRef` resolves identity-only via
// `reconstruct_type_annotation` — the SOLE route. Post-E5 CKPT-5 there is NO
// `.source` field and NO reparse arm at all, so the "resolves past a garbage
// source" pins below are STRONGER than before: the first arg
// ("###unparseable###") is now merely an unspellable NAME/KIND spelling (it feeds
// `name`/`kind` only, never a stored reparse field), and a green result can ONLY
// have come from the identity route — there is no fallback in existence for it to
// have come from. Plain `#[cfg(test)]` (NOT deep-tests-gated) so the standard
// supervisor gate runs them (E2 finding 5: pins in a deep-tests-gated module never
// run under the default gate).
#[cfg(test)]
mod e1_s5_route_proof {
    use super::reconstruct_type_annotation;
    use super::semantic_freeze::overlay_for_tests;
    use super::type_annotation_from_string_or_type_ref_slot;
    use super::FrozenTypeIdentity;
    use crate::compiler::comptime_target::build_type_ref_descriptor;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::error::ShapeError;

    // (a) LEAF identity route. A `__ComptimeTypeRef` stamped with `string`'s
    // frozen identity, built with an unspellable "###unparseable###" spelling
    // (post-E5 CKPT-5 that arg feeds only `name`/`kind` — there is NO `.source`
    // field), resolves to `Basic("string")`. Success PROVES the identity route
    // fired: it is the ONLY resolution route in existence (the `.source` reparse
    // arm is DELETED), so no fallback could have produced this. Unit-tier witness
    // for the byte-identical corpus (leaf `string` resolves via identity).
    #[test]
    fn e1_s5_leaf_identity_route_resolves_past_garbage_source() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let identity = overlay
            .canonicalize_type(&TypeAnnotation::Basic("string".to_string()))
            .expect("`string` canonicalizes to a leaf identity");
        let slot = build_type_ref_descriptor("###unparseable###", None, Some(identity));
        let resolved =
            type_annotation_from_string_or_type_ref_slot(&slot, "__emit_set_return_type", &overlay)
                .expect("a stamped leaf ref resolves via identity — the sole route");
        assert_eq!(
            resolved,
            TypeAnnotation::Basic("string".to_string()),
            "the identity route reconstructs the canonical leaf; no `.source` arm exists to reparse the garbage spelling"
        );
    }

    // (b) COMPOSITE identity route + shared-overlay (unit tier). A
    // `__ComptimeTypeRef` stamped with `[int, string]`'s composite identity plus
    // an unspellable spelling (post-E5 CKPT-5: feeds only `name`/`kind`; no
    // `.source` field) resolves to the byte-identical Tuple — ON THE SAME overlay
    // the identity was minted from (the composite payload lives in that Arc's
    // per-instance `composites` memo, so a DIFFERENT overlay could not answer
    // `payload_of`). Green proves BOTH the composite route AND the shared-overlay
    // requirement at the unit tier — with no fallback arm in existence to have
    // resolved it otherwise; the Tuple e2e is the integration canary for the same
    // property across the real handler path.
    #[test]
    fn e1_s5_composite_identity_route_resolves_past_garbage_source() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let tuple = TypeAnnotation::Tuple(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]);
        // `canonicalize_type_projection` (NOT bare `canonicalize_type`) is the
        // producer's own stamp-site call: it BOTH computes the identity AND
        // interns the composite payload into this overlay's `composites` memo,
        // exactly as `stamp_for` does — the same shared-overlay evidence the
        // consumer's `payload_of` then reads.
        let identity = overlay
            .canonicalize_type_projection(&tuple)
            .expect("[int, string] canonicalizes + interns its composite payload")
            .identity();
        let slot = build_type_ref_descriptor("###unparseable###", None, Some(identity));
        let resolved =
            type_annotation_from_string_or_type_ref_slot(&slot, "__emit_set_return_type", &overlay)
                .expect("a stamped composite ref resolves via identity off the SAME overlay");
        assert_eq!(
            resolved,
            TypeAnnotation::Tuple(vec![
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Basic("string".to_string()),
            ]),
            "the composite route recurses the Tuple element identities; no `.source` arm exists to reparse the garbage spelling"
        );
    }

    // (c) ANTI-WALK-BACK sentinel (E1-D7(a)). A STAMPED-but-never-frozen identity
    // is a NAMED `ShapeError::SemanticError` — NOT `Ok`, NOT a silent `.source`
    // reparse. `reconstruct_type_annotation` is called directly (the exact fn the
    // live resolver invokes on a stamped ref) with an identity the overlay never
    // issued; `payload_of` rejects it with its named unknown-identity error,
    // surfaced as a typed SemanticError. This is the canonical stamped->reparse
    // walk-back refusal made a test: a stamped identity NEVER falls back.
    #[test]
    fn e1_s5_stamped_unresolvable_identity_is_named_semantic_error_no_fallback() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        // A fabricated identity the freeze never issued (SHA-256 prefixes are not
        // 0xDEAD/0xBEEF), and NOT the INVALID sentinel — a genuinely STAMPED ref.
        let fabricated = FrozenTypeIdentity {
            high: 0xDEAD,
            low: 0xBEEF,
        };
        assert_ne!(
            fabricated,
            FrozenTypeIdentity::INVALID,
            "the sentinel must be a STAMPED identity, not INVALID"
        );
        match reconstruct_type_annotation(&overlay, fabricated) {
            Err(ShapeError::SemanticError { .. }) => {}
            other => panic!(
                "a stamped-but-unresolvable identity must be a NAMED SemanticError with NO \
                 fallback to reparse (E1-D7(a)); got {other:?}"
            ),
        }
    }

    // (d) ADR-009 E5 CKPT-4 RULING + E5 CKPT-5 DELETION — an UNSTAMPED (INVALID)
    // `__ComptimeTypeRef` is a RULED NAMED SURFACE-AND-STOP at the consumer. This is
    // the CKPT-4 successor to the former "falls through to the `.source` arm
    // bytewise" pin (design §4: "successor asserts unstamped/unresolvable = named
    // surface-and-stop"). CKPT-1..3 made every reconstructable type stamp, so the
    // ONLY refs that still carry INVALID describe NO reconstructable type (design §2
    // class C / A5 — an unresolved return, a synthetic member, a scoped generic
    // parameter, an un-applied head); the consumer rejects them LOUD. Post-E5
    // CKPT-5 the `.source` reparse net is DELETED (field + arm), so the rejection no
    // longer depends on `.source` at all AND the walk-back it guarded is
    // STRUCTURALLY IMPOSSIBLE: the first arg is now merely a `name`/`kind` spelling.
    // A perfectly-parseable spelling ("string") that WOULD once have reparsed to
    // `Ok(Basic("string"))` is the load-bearing witness — there is no arm left that
    // could ever answer `Ok`, so this pin now proves the arm cannot be re-introduced.
    #[test]
    fn e1_s5_ckpt4_unstamped_typeref_is_named_surface_and_stop_not_source_reparse() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());

        // INVALID stamp + a fully VALID spelling ("string"): if a `.source` reparse
        // arm existed it WOULD answer `Ok(Basic("string"))`. Post-CKPT-5 there is no
        // such arm — the consumer rejects LOUD (the CKPT-4 ruling; now structural).
        let valid_spelling = build_type_ref_descriptor("string", None, None);
        let err = type_annotation_from_string_or_type_ref_slot(
            &valid_spelling,
            "__emit_set_return_type",
            &overlay,
        )
        .expect_err(
            "an unstamped (INVALID) ref must be a NAMED surface-and-stop — even a \
             perfectly-parseable spelling can never reparse, the `.source` arm is DELETED (E5 CKPT-5)",
        );
        assert!(
            err.contains("reconstructable") || err.to_lowercase().contains("no concrete type"),
            "the error must be the ruled rejection (no reconstructable identity), NOT a \
             reparse of \"string\" — no `.source` arm exists; got: {err}"
        );

        // INVALID stamp + a GARBAGE spelling: the SAME ruled rejection — the outcome
        // never depended on the spelling content, and no `.source` arm exists.
        let garbage_spelling = build_type_ref_descriptor("###unparseable###", None, None);
        let err = type_annotation_from_string_or_type_ref_slot(
            &garbage_spelling,
            "__emit_set_return_type",
            &overlay,
        )
        .expect_err("an unstamped ref rejects LOUD regardless of the spelling content");
        assert!(
            err.contains("reconstructable") || err.to_lowercase().contains("no concrete type"),
            "the garbage-spelling INVALID ref rejects via the SAME ruling; there is no \
             `.source` arm to produce a parse failure either; got: {err}"
        );
    }

    // (e) ANTI-WALK-BACK sentinel THROUGH THE FULL CONSUMER (E1-D7(a)), STRENGTHENED
    // at E5 CKPT-5. The `.source` reparse-fallback FIELD + arm are DELETED, so the
    // walk-back this pin once guarded — "a stamped-but-unresolvable ref silently
    // reparses its valid spelling" — is now STRUCTURALLY IMPOSSIBLE: there is no
    // `.source` field to read and no arm in existence to reparse from. The pin
    // therefore becomes: a STAMPED-but-never-frozen ref (identity != INVALID → the
    // consumer takes the identity reconstruct branch) whose FIRST ARG is a
    // perfectly-parseable spelling ("int") STILL Errs through the WHOLE
    // `type_annotation_from_string_or_type_ref_slot` consumer with the identity
    // route's NAMED rejection — never an `Ok`. The spelling "int" now feeds ONLY
    // `name`/`kind` (no `.source`); it is the load-bearing witness that even a
    // valid spelling can NEVER become `Ok(Basic("int"))` because no fallback path
    // exists. If a future edit RE-INTRODUCED a `.source` field + reparse arm, "int"
    // would reparse to `Ok` and this `expect_err` would fire — this pin guards
    // against the re-introduction the CKPT-5 deletion made impossible.
    #[test]
    fn e1_s5_stamped_unresolvable_ref_errs_through_full_consumer_never_reparses_valid_source() {
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        // A fabricated identity the freeze never issued (SHA-256 prefixes are not
        // 0xDEAD/0xBEEF), and NOT the INVALID sentinel — a genuinely STAMPED ref.
        let fabricated = FrozenTypeIdentity {
            high: 0xDEAD,
            low: 0xBEEF,
        };
        assert_ne!(
            fabricated,
            FrozenTypeIdentity::INVALID,
            "the sentinel must be a STAMPED identity, not INVALID"
        );
        // The spelling "int" is a fully VALID, parseable type payload: if a
        // `.source` reparse arm existed it WOULD answer `Ok(Basic("int"))`. Post-E5
        // CKPT-5 there is no `.source` field and no such arm — the spelling feeds
        // only `name`/`kind` and can never be reparsed. A stamped ref resolves
        // identity-only.
        let slot = build_type_ref_descriptor("int", None, Some(fabricated));
        let err = type_annotation_from_string_or_type_ref_slot(
            &slot,
            "__emit_set_return_type",
            &overlay,
        )
        .expect_err(
            "a stamped-but-unresolvable ref must Err through the FULL consumer via the identity \
             route — no `.source` field or reparse arm exists to fall back to (E5 CKPT-5)",
        );
        // The surfaced String is reconstruct's named unknown-identity rejection
        // (`payload_of` -> `category_for_identity`), NOT a reparse of "int" — a
        // reparse would have been `Ok`, and no arm exists that could produce it.
        assert!(
            err.to_lowercase().contains("identity"),
            "the error must be the identity-route's named rejection; there is no `.source` \
             arm to reparse the valid spelling; got: {err}"
        );
    }

    // (f) ADR-009 E5 CKPT-1: APPLIED-GENERIC identity route past a garbage
    // spelling. The applied-generic analogue of pin (b): a `__ComptimeTypeRef`
    // stamped with `Array<Option<int>>`'s composite identity plus an unspellable
    // spelling (post-E5 CKPT-5 that arg feeds only `name`/`kind`; no `.source`
    // field) resolves to the nested `Generic{Array,[Generic{Option,[int]}]}`
    // spelling ON THE SAME overlay. Green proves THREE things at once — (i) the
    // CKPT-1 stamp-gate auto-widen (the applied identity is stampable), (ii) the
    // identity route fired (it is the SOLE route — no `.source` reparse arm exists),
    // and (iii) the NESTED inner identity resolves off the shared memo
    // (projection.rs recursive sub-expression memoization). This is the same route
    // the Tuple pin (b) exercises, now widened to applied generics.
    #[test]
    fn e1_s5_applied_generic_identity_route_resolves_past_garbage_source() {
        use shape_ast::ast::TypePath;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());
        let nested = TypeAnnotation::Array(Box::new(TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        }));
        // `canonicalize_type_projection` is the producer's own stamp-site call:
        // it computes the identity AND interns the composite payload for the
        // whole SUBTREE (CKPT-1), exactly as `stamp_for` does — the same shared
        // evidence the consumer's identity route then reads.
        let identity = overlay
            .canonicalize_type_projection(&nested)
            .expect("Array<Option<int>> canonicalizes + interns its composite payload subtree")
            .identity();
        let slot = build_type_ref_descriptor("###unparseable###", None, Some(identity));
        let resolved = type_annotation_from_string_or_type_ref_slot(
            &slot,
            "__emit_set_return_type",
            &overlay,
        )
        .expect("a stamped applied-generic ref resolves via identity, past the garbage source");
        assert_eq!(
            resolved,
            TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Generic {
                    name: TypePath::simple("Option"),
                    args: vec![TypeAnnotation::Basic("int".to_string())],
                }],
            },
            "the applied-generic identity route reconstructs the nested spelling; no `.source` arm exists to reparse the garbage spelling"
        );
    }

    // (g) ADR-009 E5 CKPT-4 EXIT CRITERION (charter-critical) + E5 CKPT-5 — the
    // `__ComptimeTypeRef` IDENTITY surface AND the OVER-DELETION TRIPWIRE. Every
    // `__ComptimeTypeRef` reaching the consumer either STAMPS (concrete types →
    // identity route) or is a RULED named surface-and-stop (INVALID → LOUD,
    // subsuming design class C). Proven end-to-end through the REAL `to_nanboxed`
    // producer + the live consumer:
    //   • a CONCRETE-return producer STAMPS its `return_type_ref` (identity !=
    //     INVALID) → the consumer resolves it via the IDENTITY route (the SOLE
    //     route; the `.source` reparse arm is DELETED at CKPT-5);
    //   • the class-C case (`kind:"Unresolved"`, no real type) rejects LOUD;
    //   • ANY INVALID `__ComptimeTypeRef` rejects LOUD (pin (d)).
    // The complementary producer half — concrete applied/record/nominal inputs
    // stamp — is pinned by the `_stamp_gate_predicate_auto_widens_*` pins.
    //
    // OVER-DELETION TRIPWIRE (#88 PRESERVED): the bare-STRING type-payload arm is a
    // SANCTIONED, documented carrier for `item_fn` / `extend_method`
    // (`item_fn(name, return_type: string | TypeRef, value)`), which have no
    // sanctioned Int64/TypeRef alternative today and inherently need
    // `parse_type_annotation_payload` to turn a spelling string into an AST. E5
    // CKPT-5 deleted the `.source` FALLBACK but PRESERVED this parser (split to
    // #88). The two string-arm bullets below prove the parser SURVIVES — it parses
    // a leaf ("int") AND an applied generic ("Array<int>"). If a future edit
    // over-deletes `parse_type_annotation_payload`/`__type_probe`/the bare-string
    // arm, those assertions fail — this is the two-sided precision guard (delete the
    // `.source` fallback AND keep the item_fn parser). See e5-decisions.md CKPT-5.
    #[test]
    fn e1_s5_ckpt4_typeref_producers_stamp_invalid_rejects_loud_string_arm_surfaced() {
        use crate::compiler::comptime_target::{AnnotationTargetKind, ComptimeTarget};
        use shape_value::KindedSlot;
        use std::sync::Arc;
        let overlay = overlay_for_tests(&BytecodeCompiler::new());

        // --- a concrete producer STAMPS: `fn f() -> int` through `to_nanboxed` ---
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Function,
            name: "f".to_string(),
            fields: Vec::new(),
            params: Vec::new(),
            return_type: Some("int".to_string()),
            annotations: Vec::new(),
            captures: Vec::new(),
            param_type_asts: Vec::new(),
            return_type_ast: Some(TypeAnnotation::Basic("int".to_string())),
            field_type_asts: Vec::new(),
        };
        let nb = target
            .to_nanboxed(Some(overlay.as_ref()))
            .expect("a concrete-return target builds");
        let storage = nb
            .as_typed_object_storage()
            .expect("__ComptimeTarget is a typed object");
        // __ComptimeTarget field order: kind, name, fields, params, return_type,
        // return_type_ref(5), annotations, captures.
        let ret_ref = storage
            .clone_field_kinded(5)
            .expect("return_type_ref slot present");
        let resolved = type_annotation_from_string_or_type_ref_slot(
            &ret_ref,
            "__emit_set_return_type",
            &overlay,
        )
        .expect("a concrete-return producer STAMPS + resolves via the identity route");
        assert_eq!(
            resolved,
            TypeAnnotation::Basic("int".to_string()),
            "the stamped return_type_ref resolves identity-only to `int` — the SOLE route; the `.source` arm is deleted"
        );

        // --- class C / any INVALID `__ComptimeTypeRef` rejects LOUD ---
        let class_c = build_type_ref_descriptor("unknown", Some("Unresolved"), None);
        let err = type_annotation_from_string_or_type_ref_slot(
            &class_c,
            "__emit_set_return_type",
            &overlay,
        )
        .expect_err("a class-C Unresolved type_ref has no concrete type — reject LOUD");
        assert!(
            err.contains("reconstructable") || err.to_lowercase().contains("no concrete type"),
            "class-C rejection must be the named surface-and-stop, got: {err}"
        );

        // --- OVER-DELETION TRIPWIRE: the bare-string item_fn arm STILL parses ---
        // #88 PRESERVED. The bare-string arm is the SANCTIONED carrier for
        // item_fn/extend (`return_type: string | TypeRef`); E5 CKPT-5 deleted the
        // `.source` fallback but MUST NOT touch this parser. A leaf spelling ("int")
        // parses via `parse_type_annotation_payload`/`__type_probe`.
        let string_carrier = KindedSlot::from_string_arc(Arc::new("int".to_string()));
        let resolved = type_annotation_from_string_or_type_ref_slot(
            &string_carrier,
            "item_fn",
            &overlay,
        )
        .expect("the item_fn/extend bare-string type carrier STILL parses a leaf (#88 preserved)");
        assert_eq!(
            resolved,
            TypeAnnotation::Basic("int".to_string()),
            "the item_fn bare-string arm parses `int` — the #88 parser survives the `.source` deletion"
        );

        // ...AND an APPLIED-GENERIC spelling: `item_fn(name, "Array<int>", value)`.
        // This is the load-bearing over-deletion tripwire — a non-trivial spelling
        // that ONLY `parse_type_annotation_payload` can turn into an AST (there is
        // no non-parse path from "Array<int>" to a `TypeAnnotation`). If CKPT-5
        // over-deleted the parser, this `expect` fails.
        let applied_carrier = KindedSlot::from_string_arc(Arc::new("Array<int>".to_string()));
        let resolved_applied = type_annotation_from_string_or_type_ref_slot(
            &applied_carrier,
            "item_fn",
            &overlay,
        )
        .expect("the item_fn bare-string carrier parses an applied-generic spelling (#88 parser preserved)");
        assert!(
            matches!(
                resolved_applied,
                TypeAnnotation::Array(_) | TypeAnnotation::Generic { .. }
            ),
            "item_fn parses `Array<int>` to an Array/Generic — the #88 parser must survive the \
             `.source` deletion (over-deletion tripwire); got: {resolved_applied:?}"
        );
    }
}

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
            remote_dispatch: None,
        }
    }

    #[test]
    fn test_comptime_builtins_module_created() {
        let module = create_comptime_builtins_module(
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        assert_eq!(module.name, "__comptime__");
    }

    #[test]
    fn test_comptime_warning_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let warning = module
            .typed_exports()
            .get("warning")
            .expect("warning function should exist");
        assert_eq!(warning.return_type, ConcreteType::Unit);
        let result = (warning.invoke)(&[], &ctx).expect("warning should return unit");
        assert!(matches!(
            result,
            TypedReturn::Concrete(ConcreteReturn::Unit)
        ));
    }

    #[test]
    fn test_comptime_error_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let error = module
            .typed_exports()
            .get("error")
            .expect("error function should exist");
        assert_eq!(error.return_type, ConcreteType::Unit);
        let result = (error.invoke)(&[], &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("comptime error"));
    }

    #[test]
    fn test_comptime_build_config_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let build_config = module
            .typed_exports()
            .get("build_config")
            .expect("build_config function should exist");
        assert_eq!(build_config.return_type, ConcreteType::Object);
        let result = (build_config.invoke)(&[], &ctx).expect("build_config should return object");
        assert!(matches!(
            result,
            TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(_))
        ));
    }
}

// S2/S3 (ADR-009 §4.1) — always-on unit proofs for the single builtins-module
// flavor: reflection resolves against the shared freeze handle (the S2-era
// pre-pass rejection flavor is deleted; the barrier precedes every comptime
// site). These do not depend on the deleted comptime dispatch ABI, so they
// are not `deep-tests`-gated.
#[cfg(test)]
mod freeze_handle_module_tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
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
            remote_dispatch: None,
        }
    }

    /// The authoritative module's `type_ref` intrinsic resolves a frozen
    /// identity against the shared freeze handle (Arc-cloned into the
    /// closure — no snapshot copy, no rebuild).
    #[test]
    fn authoritative_type_ref_resolves_against_the_shared_freeze() {
        let overlay = semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new());
        let int_identity = overlay
            .identity_of("int")
            .expect("int is frozen in every unit");
        let module = create_comptime_builtins_module(
            Default::default(),
            Arc::clone(&overlay),
        );
        let ctx = test_ctx();

        let type_ref = module
            .typed_exports()
            .get(TYPE_REF_INTRINSIC)
            .expect("type_ref intrinsic registered");
        let result = (type_ref.invoke)(
            &[
                KindedSlot::from_int(int_identity.high),
                KindedSlot::from_int(int_identity.low),
            ],
            &ctx,
        )
        .expect("frozen identity must resolve");
        assert!(matches!(
            result,
            TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(_))
        ));

        // An identity the freeze never issued is rejected (freeze-boundary
        // rule), through the same Arc-shared handle.
        let error = (type_ref.invoke)(&[KindedSlot::from_int(-1), KindedSlot::from_int(-1)], &ctx)
            .expect_err("unknown identity must be rejected");
        assert!(
            error.contains("unknown semantic type identity"),
            "freeze-boundary rejection missing: {error}"
        );
    }

    // ── ADR-009 B1 S3: `reflect` — the fourth freeze-consuming builtin ────

    /// Build an opaque TypeRef argument slot for a frozen identity (the
    /// same carrier `type_ref` produces).
    fn type_ref_slot(identity: FrozenTypeIdentity) -> KindedSlot {
        typed_object_for_named_schema(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA,
            &[
                ("identity_high", KindedSlot::from_int(identity.high)),
                ("identity_low", KindedSlot::from_int(identity.low)),
            ],
        )
    }

    /// Read `__variant` (field 0) and `__payload_0` (field 1) out of a
    /// descriptor object, returning the variant id and the payload slot.
    fn descriptor_variant_and_payload(
        storage: &shape_value::heap_value::TypedObjectStorage,
    ) -> (i64, Option<KindedSlot>) {
        let variant = storage
            .clone_field_kinded(0)
            .and_then(|slot| slot.as_i64())
            .expect("__variant is an int at field 0");
        (variant, storage.clone_field_kinded(1))
    }

    fn schema_name_of(storage: &shape_value::heap_value::TypedObjectStorage) -> String {
        shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
            .expect("descriptor schema resolvable")
            .name
            .clone()
    }

    /// `reflect` on a Primitive identity returns the sealed `FrozenType`
    /// carrier: the unspellable descriptor schema with the CATALOG-ORDINAL
    /// variant id (Primitive=0), wrapping the nested `FrozenPrimitive`
    /// descriptor whose family variant carries its width-domain object
    /// (int → SignedInteger(W64)).
    #[test]
    fn reflect_intrinsic_returns_the_ordinal_pinned_frozen_type_carrier() {
        use shape_runtime::type_schema::builtin_schemas::{
            COMPTIME_FROZEN_PRIMITIVE_SCHEMA, COMPTIME_FROZEN_TYPE_SCHEMA,
        };

        let overlay = semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new());
        let int_identity = overlay
            .identity_of("int")
            .expect("int is frozen in every unit");
        let module = create_comptime_builtins_module(Default::default(), Arc::clone(&overlay));
        let ctx = test_ctx();

        let reflect = module
            .typed_exports()
            .get(REFLECT_INTRINSIC)
            .expect("reflect intrinsic registered as the fourth freeze consumer");
        let result = (reflect.invoke)(&[type_ref_slot(int_identity)], &ctx)
            .expect("Primitive payload is enabled");
        let TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(value)) = result else {
            panic!("reflect must return an opaque typed-object carrier");
        };
        let HeapValue::TypedObject(ptr) = value.as_ref() else {
            panic!("reflect carrier must be a TypedObject");
        };
        // SAFETY: the carrier owns one live share for the wrapper's lifetime.
        let storage = unsafe { &*ptr.as_ptr() };
        assert_eq!(schema_name_of(storage), COMPTIME_FROZEN_TYPE_SCHEMA);
        let (variant, payload) = descriptor_variant_and_payload(storage);
        assert_eq!(
            variant, 0,
            "FrozenType::Primitive must carry the catalog ORDINAL 0"
        );

        let payload = payload.expect("Primitive payload present");
        let primitive = payload
            .as_typed_object_storage()
            .expect("payload is the nested FrozenPrimitive descriptor");
        assert_eq!(schema_name_of(primitive), COMPTIME_FROZEN_PRIMITIVE_SCHEMA);
        let (primitive_variant, width) = descriptor_variant_and_payload(primitive);
        // SignedInteger is catalog position 3 (Unit, Bool, Char, SignedInteger, …).
        assert_eq!(primitive_variant, 3, "int is a SignedInteger family member");
        let width = width.expect("family variant carries a width-domain payload");
        let width_storage = width
            .as_typed_object_storage()
            .expect("width payload is the IntegerWidth enum object");
        assert_eq!(
            schema_name_of(width_storage),
            shape_runtime::comptime_reflection::INTEGER_WIDTH_SCHEMA_NAME
        );
        let (width_variant, _) = descriptor_variant_and_payload(width_storage);
        // W64 is IntegerWidth catalog position 3 (W8, W16, W32, W64, Arbitrary).
        assert_eq!(width_variant, 3, "int carries the exact W64 width domain");
    }

    /// ADR-009 B5 (drift note R10/R11): an un-applied generic constructor head
    /// (`Array`, a builtin nominal head with declared param kinds) is
    /// `TypeConstructorRef` territory, NOT a resolved nominal shape — reflecting
    /// it through the intrinsic is the named rejection, never a partial or
    /// off-the-un-applied-form descriptor.
    #[test]
    fn reflect_intrinsic_rejects_unapplied_generic_head_with_the_named_diagnostic() {
        let overlay = semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new());
        let nominal_identity = overlay
            .identity_of("Array")
            .expect("Array is frozen as a builtin nominal");
        let module = create_comptime_builtins_module(Default::default(), Arc::clone(&overlay));
        let ctx = test_ctx();

        let reflect = module
            .typed_exports()
            .get(REFLECT_INTRINSIC)
            .expect("reflect intrinsic registered");
        let error = (reflect.invoke)(&[type_ref_slot(nominal_identity)], &ctx)
            .expect_err("an un-applied generic head must reject");
        assert!(
            error.contains("un-applied generic type constructor is not a resolved nominal shape"),
            "un-applied-head rejection must be the named diagnostic: {error}"
        );
    }

    /// R4 at the intrinsic layer: a non-TypeRef argument and an identity
    /// the freeze never issued both reject with named diagnostics.
    #[test]
    fn reflect_intrinsic_rejects_non_type_ref_args_and_unknown_identities() {
        let overlay = semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new());
        let module = create_comptime_builtins_module(Default::default(), Arc::clone(&overlay));
        let ctx = test_ctx();
        let reflect = module
            .typed_exports()
            .get(REFLECT_INTRINSIC)
            .expect("reflect intrinsic registered");

        let error = (reflect.invoke)(&[KindedSlot::from_int(42)], &ctx)
            .expect_err("an int is not a TypeRef");
        assert!(
            error.contains("reflect expects a TypeRef value"),
            "R4 rejection must be reflect-named: {error}"
        );

        let error = (reflect.invoke)(&[type_ref_slot(FrozenTypeIdentity::INVALID)], &ctx)
            .expect_err("an identity the freeze never issued must reject");
        assert!(
            error.contains("unknown semantic type identity"),
            "freeze-boundary rejection missing: {error}"
        );
    }
}
