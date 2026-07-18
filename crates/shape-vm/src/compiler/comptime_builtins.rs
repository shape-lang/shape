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

use shape_runtime::marshal::{register_typed_fn_1, register_typed_fn_2};
use shape_runtime::module_exports::ModuleExports;
use shape_runtime::type_schema::typed_object_for_named_schema;
use shape_runtime::type_system::BuiltinTypes;
use shape_runtime::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectStorage};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{ELEM_TYPE_STRING, TypedArray, read_elem_type};
use shape_value::{KindedSlot, NativeKind};
// ADR-009 E2 #18 (slice 2): the typed `item_fn` carrier (E2-D10).
use super::comptime_fragments::CheckedItem;
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
// Legacy-path confinement (ADR-009 §4.1 "one kind vocabulary", ticket A1 S5):
// `type_reflection::build_type_info_heap_value` is deliberately NOT
// re-exported. The legacy `type_info` intrinsic below is its only caller
// (path-qualified); ticket E5 deletes the whole path. Sentinel:
// `type_reflection/tests.rs::legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path`.

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
    ReplaceModule {
        items: Vec<shape_ast::ast::Item>,
    },
    /// ADR-009 E2 #18 (slice 1): the TYPED `replace module` route. Same effect
    /// as `ReplaceModule` (replace a module target's body), but the items
    /// arrived from a typed `__ComptimeItemFragment` — no source/JSON string
    /// ever existed — and the module-target consumer routes them through
    /// `build_checked_module` (generated-provenance stamp + hygienic export
    /// reservation), producing a `comptime_fragments::CheckedModule`. Carried as
    /// a DISTINCT variant so the legacy string route stays byte-for-byte
    /// unchanged until the slice-5 deletion removes it whole (E2-D8 staging;
    /// NOT a bridge — the two paths never convert into one another).
    ReplaceModuleChecked {
        items: Vec<shape_ast::ast::Item>,
    },
    /// §4.5.7: ADD generated items at the annotated item's module scope. Unlike
    /// `ReplaceModule` (which is only valid on a module target and replaces its
    /// body), `ExtendItems` is additive and valid on type/function/module
    /// targets: the parsed items are registered + compiled alongside the
    /// existing program.
    ExtendItems {
        items: Vec<shape_ast::ast::Item>,
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
    /// per-run clear. Replaces the U03 JSON transport (`serialize_directive_payload`
    /// -> `parse_function_body_payload`), deleted in Part B.
    static COMPTIME_REPLACE_BODIES: RefCell<Vec<Vec<shape_ast::ast::Statement>>> =
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
) -> Result<shape_ast::ast::TypeAnnotation, String> {
    if let Some(payload) = slot.as_str() {
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
    let source = string_field_from_typed_object(storage, &schema, "source")?;
    parse_type_annotation_payload(&source)
}

#[allow(dead_code)] // E2-D10 staging: dead until the slice-5 U07 deletion.
fn type_source_from_string_or_type_ref_slot(
    slot: &KindedSlot,
    builtin_name: &str,
) -> Result<String, String> {
    if let Some(payload) = slot.as_str() {
        parse_type_annotation_payload(payload)?;
        return Ok(payload.to_string());
    }
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err(format!(
            "{builtin_name} expects a string type payload or __ComptimeTypeRef value, got {:?}",
            slot.kind()
        ));
    }
    let storage = slot
        .as_typed_object_storage()
        .ok_or_else(|| format!("{builtin_name} expects a non-null __ComptimeTypeRef value"))?;
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
    let source = string_field_from_typed_object(storage, &schema, "source")?;
    parse_type_annotation_payload(&source)?;
    Ok(source)
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

#[allow(dead_code)] // E2-D10 staging: dead until the slice-5 U07 deletion.
fn literal_fragment_fields_from_slot(
    slot: &KindedSlot,
    builtin_name: &str,
) -> Result<Vec<(&'static str, KindedSlot)>, String> {
    let mut fields = vec![
        ("literal_string", nb_str("")),
        ("literal_int", KindedSlot::from_int(0)),
        ("literal_number", KindedSlot::from_number(0.0)),
        ("literal_bool", KindedSlot::from_bool(false)),
    ];
    let kind = if let Some(value) = slot.as_str() {
        fields[0] = ("literal_string", nb_str(value));
        "string"
    } else if let Some(value) = slot.as_i64() {
        fields[1] = ("literal_int", KindedSlot::from_int(value));
        "int"
    } else if let Some(value) = slot.as_f64() {
        if !value.is_finite() {
            return Err(format!(
                "{builtin_name} only supports finite numeric literals in ItemFragment values"
            ));
        }
        fields[2] = ("literal_number", KindedSlot::from_number(value));
        "number"
    } else if let Some(value) = slot.as_bool() {
        fields[3] = ("literal_bool", KindedSlot::from_bool(value));
        "bool"
    } else {
        return Err(format!(
            "{builtin_name} only supports string, int, number, or bool literal return values; got {:?}",
            slot.kind()
        ));
    };
    fields.insert(0, ("literal_kind", nb_str(kind)));
    Ok(fields)
}

fn literal_expr_from_fragment(
    storage: &TypedObjectStorage,
    schema: &shape_runtime::type_schema::TypeSchema,
) -> Result<shape_ast::ast::Expr, String> {
    use shape_ast::ast::{Expr, Literal, Span};

    let kind = string_field_from_typed_object(storage, schema, "literal_kind")?;
    let literal = match kind.as_str() {
        "string" => Literal::String(string_field_from_typed_object(
            storage,
            schema,
            "literal_string",
        )?),
        "int" => {
            let slot = field_slot_from_typed_object(storage, schema, "literal_int")?;
            Literal::Int(
                slot.as_i64()
                    .ok_or_else(|| "ItemFragment.literal_int is not an int".to_string())?,
            )
        }
        "number" => {
            let slot = field_slot_from_typed_object(storage, schema, "literal_number")?;
            let value = slot
                .as_f64()
                .ok_or_else(|| "ItemFragment.literal_number is not a number".to_string())?;
            if !value.is_finite() {
                return Err("ItemFragment.literal_number must be finite".to_string());
            }
            Literal::Number(value)
        }
        "bool" => {
            let slot = field_slot_from_typed_object(storage, schema, "literal_bool")?;
            Literal::Bool(
                slot.as_bool()
                    .ok_or_else(|| "ItemFragment.literal_bool is not a bool".to_string())?,
            )
        }
        other => {
            return Err(format!("unsupported ItemFragment literal kind '{}'", other));
        }
    };
    Ok(Expr::Literal(literal, Span::default()))
}

/// ADR-009 E2 #18 (slice 2): a literal value slot -> an `Expr::Literal`,
/// DIRECTLY — the typed replacement for the `__ComptimeItemFragment` sentinel
/// encode/decode (`literal_fragment_fields_from_slot` +
/// `literal_expr_from_fragment`). No `literal_kind` discriminator, no parallel
/// sentinel fields: the slot's runtime kind selects the literal.
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
/// from `item_fn`'s raw args (E2-D10) — the typed replacement for the
/// `build_function_item_fragment` -> `function_item_from_fragment` sentinel
/// round-trip (both retained, dead-but-present, until the slice-5 U07 deletion).
/// The typed return comes from the raw return-type slot and the body from the
/// value slot; no sentinel fields and no source/JSON string participate. Spans
/// are `Span::default()` scaffolding — the directive consumer's shared check
/// sequence (`check_generated_function_item`) re-bases them to the real
/// application anchor before the decl is reserved.
fn build_function_item(
    name: &str,
    return_type_slot: &KindedSlot,
    value_slot: &KindedSlot,
) -> Result<shape_ast::ast::Item, String> {
    use shape_ast::ast::{FunctionDef, Item, Span, Statement};

    if !is_valid_generated_function_name(name) {
        return Err(format!(
            "item_fn expected a valid generated free-function name, got '{}'",
            name
        ));
    }
    let return_type = type_annotation_from_string_or_type_ref_slot(return_type_slot, "item_fn")?;
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
    )
}

/// ADR-009 E2 #18 (slice 5b-1): build the generated `Item::Extend { Type { method
/// <name>() -> <ret> { <literal> } } }` whose method body is a typed LITERAL value
/// (int / number / bool / string), DIRECTLY from the value slot. The literal is
/// decoded by the SAME `literal_expr_from_slot` authority `item_fn` uses (slice 2)
/// — no second literal decoder, no source text, no f-string template. This is the
/// literal-body sibling of `build_extend_method_item`: it closes the migration gap
/// for the `extend (f"…{ <literal> }…")` fixtures that generate a CONSTANT method
/// body (e.g. `method answer() -> int { 42 }`), which the template producer
/// (self-field interpolation only) cannot express. Both producers share the method
/// + `Item::Extend` assembly (`build_extend_item_with_method_body`) and both flow
/// to the same generic `Item::Extend` materialization; the body expr is the only
/// axis of difference.
fn build_extend_method_literal_item(
    type_name: &str,
    method_name: &str,
    return_type_slot: &KindedSlot,
    value_slot: &KindedSlot,
) -> Result<shape_ast::ast::Item, String> {
    let body_expr = literal_expr_from_slot(value_slot, "extend_method_literal")?;
    build_extend_item_with_method_body(
        "extend_method_literal",
        type_name,
        method_name,
        return_type_slot,
        body_expr,
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

    let return_type = type_annotation_from_string_or_type_ref_slot(return_type_slot, builtin_name)?;

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

#[allow(dead_code)] // E2-D10 staging: dead until the slice-5 U07 deletion.
fn type_ref_slot_from_string_or_type_ref_slot(
    slot: &KindedSlot,
    builtin_name: &str,
) -> Result<KindedSlot, String> {
    if let Some(source) = slot.as_str() {
        parse_type_annotation_payload(source)?;
        return Ok(super::comptime_target::build_type_ref_descriptor(
            source, None,
        ));
    }
    type_annotation_from_string_or_type_ref_slot(slot, builtin_name)?;
    Ok(slot.clone())
}

// ADR-009 E2-D10 / E2-D8 staging: `item_fn` moved to the typed `CheckedItem`
// carrier (`build_function_item`), so the `__ComptimeItemFragment` sentinel
// builder + its exclusive callees below are now unreached. They are RETAINED
// byte-unchanged and marked dead until the slice-5 U07 deletion removes them
// whole; `#[allow(dead_code)]` is the staging annotation, not a rename.
#[allow(dead_code)]
fn build_function_item_fragment(
    name: &str,
    return_type_slot: &KindedSlot,
    value: &KindedSlot,
) -> Result<HeapValue, String> {
    if !is_valid_generated_function_name(name) {
        return Err(format!(
            "item_fn expected a valid generated free-function name, got '{}'",
            name
        ));
    }
    let return_type = type_source_from_string_or_type_ref_slot(return_type_slot, "item_fn")?;
    let return_type_ref = type_ref_slot_from_string_or_type_ref_slot(return_type_slot, "item_fn")?;
    let literal_fields = literal_fragment_fields_from_slot(value, "item_fn")?;

    let mut fields = vec![
        ("kind", nb_str("function")),
        ("name", nb_str(name)),
        ("return_type", nb_str(return_type.as_str())),
        ("return_type_ref", return_type_ref),
    ];
    fields.extend(literal_fields);
    let fragment = typed_object_for_named_schema("__ComptimeItemFragment", &fields);
    Ok(heap_value_from_typed_object_slot(fragment))
}

fn function_item_from_fragment(
    storage: &TypedObjectStorage,
    schema: &shape_runtime::type_schema::TypeSchema,
) -> Result<shape_ast::ast::Item, String> {
    use shape_ast::ast::{FunctionDef, Item, Span, Statement};

    let kind = string_field_from_typed_object(storage, schema, "kind")?;
    if kind != "function" {
        return Err(format!(
            "unsupported ItemFragment kind '{}'; only 'function' is supported",
            kind
        ));
    }
    let name = string_field_from_typed_object(storage, schema, "name")?;
    if !is_valid_generated_function_name(&name) {
        return Err(format!(
            "ItemFragment function name '{}' is not a valid Shape identifier",
            name
        ));
    }
    let return_type_ref = field_slot_from_typed_object(storage, schema, "return_type_ref")?;
    let return_type =
        type_annotation_from_string_or_type_ref_slot(&return_type_ref, "ItemFragment.return_type")?;
    let expr = literal_expr_from_fragment(storage, schema)?;

    // ADR-009 D1 (S3): the spans below are mini-VM scaffolding — this
    // builder runs inside comptime execution, where no application anchor
    // exists yet. The directive-consumption points
    // (`materialize_computed_comptime_extends` /
    // `apply_comptime_extend_items`) re-base every decl-level span to the
    // real application anchor via `anchor_generated_function_decl` BEFORE
    // the declaration is reserved or registered, so no Span::default()
    // survives onto a registered generated declaration (Decision 68).
    Ok(Item::Function(
        FunctionDef {
            name,
            name_span: Span::default(),
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: Vec::new(),
            return_type: Some(return_type),
            where_clause: None,
            body: vec![Statement::Expression(expr, Span::default())],
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
        },
        Span::default(),
    ))
}

fn parse_extend_items_slot(slot: &KindedSlot) -> Result<Vec<shape_ast::ast::Item>, String> {
    if let Some(payload) = slot.as_str() {
        return parse_module_items_payload(payload);
    }

    let storage = slot.as_typed_object_storage().ok_or_else(|| {
        format!(
            "__emit_extend_items expects a source string, __CheckedItem, or __ComptimeItemFragment, got {:?}",
            slot.kind()
        )
    })?;
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "__emit_extend_items could not resolve typed-object schema id {}",
            storage.schema_id
        )
    })?;
    // ADR-009 E2 #18 (slice 2): the TYPED route — a `__CheckedItem` handle
    // `item_fn` produced. Resolve its index back to the driver-side `CheckedItem`
    // built during THIS comptime run, with no sentinel decode and no source/JSON
    // string.
    if schema.name == "__CheckedItem" {
        let index = field_slot_from_typed_object(storage, &schema, "index")?
            .as_i64()
            .ok_or_else(|| "__CheckedItem.index is not an int".to_string())?;
        let checked = comptime_checked_item_at(index as usize).ok_or_else(|| {
            format!("__CheckedItem index {index} is not live in this comptime execution")
        })?;
        return Ok(vec![checked.into_item()]);
    }
    // LEGACY (U07 — dies WHOLE in slice 5): the `__ComptimeItemFragment` sentinel
    // map. Byte-unchanged and now unreached (item_fn moved to CheckedItem); it
    // survives beside the typed route per the E2-D8 staging until the deletion.
    if schema.name != "__ComptimeItemFragment" {
        return Err(format!(
            "__emit_extend_items expects a source string, __CheckedItem, or __ComptimeItemFragment, got '{}'",
            schema.name
        ));
    }
    Ok(vec![function_item_from_fragment(storage, &schema)?])
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
/// `type_category` / `type_info`) consume the per-compilation-unit semantic
/// freeze through the shared `Arc<FreezeOverlay>` handle — the intrinsic
/// closures clone the `Arc`, never snapshot data. The deleted per-site
/// `build_type_reflection_snapshot` rebuild has no successor here.
///
/// `trait_impl_keys` contains the set of registered trait implementations.
/// Supported key forms:
/// - Legacy: "TraitName::TypeName"
/// - Canonical: "TraitName::TypeName::ImplNameOrDefault"
///
/// `site_time_impl_keys` (slice S5) is the superset key snapshot visible at
/// the comptime site (live keys + J-CT.2 `comptime impl` pairs); it feeds
/// ONLY `find_impl`'s named Dec 52 post-barrier ordering diagnostic — never
/// evidence, never the legacy `implements` path.
pub(crate) fn create_comptime_builtins_module(
    trait_impl_keys: HashSet<String>,
    site_time_impl_keys: HashSet<String>,
    freeze: Arc<FreezeOverlay>,
) -> ModuleExports {
    let mut module = comptime_builtins_module_base(trait_impl_keys);
    // ADR-009 B2 (slice S4): `trait_ref` / `find_impl` consume the SAME
    // freeze handle — implementation evidence comes ONLY from the frozen
    // barrier truth (freeze inputs 4/5), never from the legacy
    // `trait_impl_keys` set above (E5 deletes that path).
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
fn comptime_builtins_module_base(trait_impl_keys: HashSet<String>) -> ModuleExports {
    let mut module = ModuleExports::new("__comptime__");

    // implements(type_name: string, trait_name: string) -> bool
    // Checks the TypeRegistry's trait impl data captured at compile time.
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "implements",
        "Check if a type implements a trait at compile time",
        [("type_name", "string"), ("trait_name", "string")],
        ConcreteType::Bool,
        move |type_name, trait_name, _ctx| {
            let has_impl = |ty: &str| {
                let legacy = format!("{}::{}", trait_name, ty);
                let canonical_prefix = format!("{}::{}::", trait_name, ty);
                trait_impl_keys.contains(&legacy)
                    || trait_impl_keys
                        .iter()
                        .any(|key| key.starts_with(&canonical_prefix))
            };

            if has_impl(type_name.as_str()) {
                return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(true)));
            }

            // Numeric widening: integer-family aliases can satisfy number-family impls.
            if BuiltinTypes::is_integer_type_name(type_name.as_str()) {
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

    // The freeze-consuming reflection trio (`type_ref` / `type_category` /
    // `type_info`) is NOT part of the base: `create_comptime_builtins_module`
    // registers it with the shared `Arc<FreezeOverlay>` handle (the barrier
    // runs before every comptime site — S3).

    // item_fn(name: string, return_type: string | TypeRef, value: literal) -> ItemFragment
    //
    // First typed additive-generation slice: construct a zero-arg free
    // function fragment without requiring the comptime handler to assemble
    // `fn ...` source text. The fragment is still converted to an AST item and
    // compiled by the same strict registration/type/body pipeline as the
    // source-string `extend (expr)` path.
    register_typed_function(
        &mut module,
        "item_fn",
        // E2-D10: this SURFACE (name + signature) survives E2 as the CheckedItem
        // constructor; its INTERNALS (the __ComptimeItemFragment schema + sentinel
        // machinery) die in the slice-5 U07 deletion.
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
        // ADR-009 E2 #18 (slice 2, E2-D10): `item_fn` now yields the typed
        // `__CheckedItem` carrier — a handle to a driver-side `CheckedItem` —
        // instead of the `__ComptimeItemFragment` sentinel map. The builtin
        // builds the AST `Item` directly (no sentinel fields, no source/JSON
        // string), stashes it in the per-run `CheckedItem` store, and returns a
        // handle carrying its index. The legacy fragment builders survive
        // (dead-but-present) until the slice-5 U07 deletion.
        ConcreteType::OpaqueTypedObject("__CheckedItem".to_string()),
        |slots, _ctx| {
            if slots.len() != 3 {
                return Err(format!("item_fn expects 3 arguments, got {}", slots.len()));
            }
            let name = slots[0]
                .as_str()
                .ok_or_else(|| "item_fn expects a string function name".to_string())?;
            let item = build_function_item(name, &slots[1], &slots[2])?;
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
        |slots, _ctx| {
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
        |slots, _ctx| {
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

    // Internal comptime directive: emit an extend statement payload (JSON AST).
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_extend",
        "Internal: emit extend directive payload",
        "payload",
        "string",
        ConcreteType::Unit,
        |json, _ctx| {
            let extend: shape_ast::ast::ExtendStatement = serde_json::from_str(json.as_str())
                .map_err(|e| format!("invalid extend payload: {}", e))?;
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
    // __emit_set_param_type(param_name: string, type_payload: string | TypeRef)
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
        |slots, _ctx| {
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
            let type_annotation =
                type_annotation_from_string_or_type_ref_slot(&slots[1], "__emit_set_param_type")?;
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
        |slots, _ctx| {
            if slots.len() != 1 {
                return Err(format!(
                    "__emit_set_return_type expects 1 argument, got {}",
                    slots.len()
                ));
            }
            let type_annotation =
                type_annotation_from_string_or_type_ref_slot(&slots[0], "__emit_set_return_type")?;
            push_comptime_directive(ComptimeDirective::SetReturnType { type_annotation })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace function body from serialized AST payload.
    // __emit_replace_body(body_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_replace_body",
        "Internal: replace function body from AST payload",
        "body_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let body = parse_function_body_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::ReplaceBody { body })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // ADR-009 E2 #18 (slice 5, Part A): the TYPED block-form `replace body { ... }`
    // carrier. The block-form emit stashes the Vec<Statement> at handler-COMPILE
    // (COMPTIME_REPLACE_BODIES) and passes its INDEX here — no source/JSON string,
    // no reparse. A SEPARATE builtin from `__emit_replace_body` (string) so a bare
    // int index cannot collide with the expr-form's arbitrary payload; the legacy
    // string builtin above + `parse_function_body_payload` die in Part B (they
    // lose all callers once the block form routes here and the expr form rejects).
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

    // Internal comptime directive: replace module items. TWO complete paths
    // (E2-D8 staging, ADR-009 E2 #18 slice 1). Slot-typed (`unknown`) so both
    // reach it; the arm is selected by the slot's RUNTIME kind, never by a
    // string round-trip:
    //   - a legacy source/JSON `string` -> `parse_module_items_payload` ->
    //     `ReplaceModule` (retained UNCHANGED until the slice-5 deletion);
    //   - a typed `__ComptimeItemFragment` (e.g. `item_fn(...)`) ->
    //     `ReplaceModuleChecked`. No source/JSON string ever materializes on
    //     this path — the typed transport this slice adds.
    // __emit_replace_module(module_payload)
    register_typed_function(
        &mut module,
        "__emit_replace_module",
        "Internal: replace module items from a source/JSON payload (legacy) or a typed ItemFragment",
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
            let slot = &slots[0];
            if let Some(payload) = slot.as_str() {
                // LEGACY (U03) source/JSON reparse — unchanged until slice 5.
                let items = parse_module_items_payload(payload)?;
                push_comptime_directive(ComptimeDirective::ReplaceModule { items })?;
            } else {
                // TYPED route: a `__ComptimeItemFragment`. `parse_extend_items_slot`
                // here takes ONLY its fragment branch (the string branch is
                // excluded above), converting the fragment directly to AST items
                // with no source/JSON string in between.
                let items = parse_extend_items_slot(slot)?;
                push_comptime_directive(ComptimeDirective::ReplaceModuleChecked { items })?;
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: ADD generated items from source or typed
    // ItemFragment payload (§4.5.7 `extend (expr)`).
    register_typed_function(
        &mut module,
        "__emit_extend_items",
        "Internal: add generated module items from source or typed ItemFragment payload",
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
/// `type_category` / `reflect` / legacy `type_info`) against the shared
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
    // S2 (comptime-excellence §4.3): the TypeInfo record is built via the
    // reserved, concrete named schema `__ComptimeTypeInfo` ({name, kind},
    // registered at init in `builtin_schemas.rs`) — see
    // `build_type_info_heap_value`. The previous
    // `register_predeclared_any_schema(&["kind", "name"])` lazily minted an
    // anonymous `{kind, name}` schema whose field ORDER was the reverse of
    // the stdlib `TypeInfo {name, kind}` the compiler uses for the
    // `OpaqueTypedObject("TypeInfo")` return type, so typed field access
    // read `name`/`kind` at swapped offsets. Named construction in
    // {name, kind} order aligns the physical layout with the consumer.
    //
    // `type_info(T).fields` returns the declared fields of a TypedObject type as
    // an `Array<FieldDescriptor>` — the same row shape as `target.fields` in an
    // annotation handler (comptime-excellence §4.1.2). Every non-TypedObject kind
    // reflects an empty array.
    //
    // This legacy path (TypeKindLabel string vocabulary) consumes the SAME
    // freeze handle as the typed reflection surface; E5 deletes it.
    let freeze_for_type_info = freeze;
    register_typed_function(
        module,
        "type_info",
        "Return the TypeInfo reflection record for the named type",
        vec![],
        ConcreteType::OpaqueTypedObject("TypeInfo".to_string()),
        move |nb_args, _ctx| {
            // The type name arrives as a string arg (bare type identifiers are
            // rewritten to string literals before the comptime block runs). If
            // it cannot be read as a string, surface a clean error rather than
            // reflecting an arbitrary name.
            let raw_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| "type_info expects a type name".to_string())?;
            let type_info_hv =
                type_reflection::build_type_info_heap_value(raw_name, &freeze_for_type_info)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(type_info_hv),
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
        let item = build_function_item("answer", &nb_str("int"), &KindedSlot::from_int(42))
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
        )
        .expect("valid int literal body assembles");
        assert!(matches!(extend_method_literal_body(&item), Literal::Int(42)));
        let Item::Extend(extend, _) = &item else {
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
    fn test_comptime_implements_returns_false_when_not_registered() {
        let module = create_comptime_builtins_module(
            Default::default(),
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_implements_returns_true_when_registered() {
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        impls.insert("Display::Currency".to_string());
        let module = create_comptime_builtins_module(
            impls,
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_implements_numeric_widening() {
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        let module = create_comptime_builtins_module(
            impls,
            Default::default(),
            semantic_freeze::overlay_for_tests(&crate::compiler::BytecodeCompiler::new()),
        );
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_build_config_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(
            Default::default(),
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
        let module = create_comptime_builtins_module(Default::default(), Default::default(), Arc::clone(&overlay));
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
        let module = create_comptime_builtins_module(Default::default(), Default::default(), Arc::clone(&overlay));
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
        let module = create_comptime_builtins_module(Default::default(), Default::default(), Arc::clone(&overlay));
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
