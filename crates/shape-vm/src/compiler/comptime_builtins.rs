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
use shape_value::{KindedSlot, NativeKind};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) mod semantic_freeze;
mod type_reflection;

pub(crate) use semantic_freeze::FreezeOverlay;
pub(crate) use type_reflection::{
    FrozenTypeCategory, FrozenTypeIdentity, build_frozen_type_category_heap_value,
    build_frozen_type_ref_heap_value, frozen_type_category_from_ref,
};
// Legacy-path confinement (ADR-009 §4.1 "one kind vocabulary", ticket A1 S5):
// `type_reflection::build_type_info_heap_value` is deliberately NOT
// re-exported. The legacy `type_info` intrinsic below is its only caller
// (path-qualified); ticket E5 deletes the whole path. Sentinel:
// `type_reflection/tests.rs::legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path`.

pub(crate) const TYPE_REF_INTRINSIC: &str = "\u{1}comptime:type-ref";
pub(crate) const TYPE_CATEGORY_INTRINSIC: &str = "\u{1}comptime:type-category";

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
            "__emit_extend_items expects a source string or __ComptimeItemFragment, got {:?}",
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
    if schema.name != "__ComptimeItemFragment" {
        return Err(format!(
            "__emit_extend_items expects a source string or __ComptimeItemFragment, got '{}'",
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
pub(crate) fn create_comptime_builtins_module(
    trait_impl_keys: HashSet<String>,
    freeze: Arc<FreezeOverlay>,
) -> ModuleExports {
    let mut module = comptime_builtins_module_base(trait_impl_keys);
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
        "Build a typed ItemFragment for a zero-arg generated free function",
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
        ConcreteType::OpaqueTypedObject("__ComptimeItemFragment".to_string()),
        |slots, _ctx| {
            if slots.len() != 3 {
                return Err(format!("item_fn expects 3 arguments, got {}", slots.len()));
            }
            let name = slots[0]
                .as_str()
                .ok_or_else(|| "item_fn expects a string function name".to_string())?;
            let fragment = build_function_item_fragment(name, &slots[1], &slots[2])?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(fragment),
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

    // Internal comptime directive: replace module items from source payload.
    // __emit_replace_module(module_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_replace_module",
        "Internal: replace module items from source payload",
        "module_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let items = parse_module_items_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::ReplaceModule { items })?;
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

/// Register the freeze-consuming reflection trio against the shared
/// per-compilation-unit freeze handle (ADR-009 §4.1, slice S2). Each closure
/// clones the `Arc<FreezeOverlay>` — the base index is shared, never
/// rebuilt and never copied.
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
    fn test_comptime_implements_returns_false_when_not_registered() {
        let module = create_comptime_builtins_module(
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
        let module = create_comptime_builtins_module(Default::default(), Arc::clone(&overlay));
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
}
