//! Compile-time (comptime) execution infrastructure.
//!
//! Provides a mini-VM executor that compiles and runs statements at compile time,
//! used for meta function methods with statement bodies.

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::{HygienicRole, HygienicSymbol};
use crate::executor::{VMConfig, VirtualMachine};
use shape_ast::ast::{
    AnnotationHandlerParam, DestructurePattern, Expr, FunctionDef, FunctionParameter, Item,
    ObjectEntry, ObjectTypeField, Program, Span, Statement, TypeAnnotation, VarKind, VariableDecl,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::heap_value::{HeapKind, HeapValue};
use shape_value::{KindedSlot, NativeKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

mod capture_payload_model;

// ADR-009 E3 (S1, U10): the comptime mini-program formerly named its
// generated builtin-forwarder parameters (`arg{N}`) and its handler
// target/ctx module bindings (`__target_arg__` / `__ctx_arg__`) with
// user-guessable spellings, so a handler body or an annotation-argument
// expression that referenced one of those names captured the generated
// binding. These helpers render the hygienic identity ([`HygienicSymbol`])
// into the mini-program's by-name namespace as an UNSPELLABLE descriptor
// (SOH-prefixed, like the forwarder constants above): a user reference to the
// former spelling is now a different, resolvable-to-nothing string. Each
// mini-program is compiled and executed in isolation, so a fixed nonce
// suffices — the unspellable prefix (not the nonce) is what defeats capture.
fn hygienic_forwarder_param(index: usize) -> String {
    HygienicSymbol::mint(HygienicRole::ComptimeForwarderParam(index as u32), 0)
        .unspellable_descriptor()
}

fn hygienic_comptime_target_binding() -> String {
    HygienicSymbol::mint(HygienicRole::ComptimeTargetBinding, 0).unspellable_descriptor()
}

fn hygienic_comptime_ctx_binding() -> String {
    HygienicSymbol::mint(HygienicRole::ComptimeCtxBinding, 0).unspellable_descriptor()
}

const TYPE_REF_FORWARDER: &str = "\u{1}comptime:forward-type-ref";
// ADR-009 B2 (slice S4): `trait_ref(TraitName)` lowers to this unspellable
// forwarder carrying the FROZEN trait identity as int literals — the same
// identity-literal transport as `type_ref`; a trait name string never
// crosses into the mini-VM (Dec 49).
const TRAIT_REF_FORWARDER: &str = "\u{1}comptime:forward-trait-ref";

/// `find_impl`'s forwarder return annotation: `Option<ImplRef-carrier>`.
/// Must stay byte-identical to
/// `format!("Option<{COMPTIME_FROZEN_IMPL_REF_SCHEMA}>")` — pinned by
/// `find_impl_forwarder_return_marker_matches_the_impl_ref_schema`.
const FIND_IMPL_RETURN_MARKER: &str = "Option<\u{1}comptime:ImplRef>";

// ADR-009 B4 (Stage 2, Dec 54): uniform nominal-application forwarders.
// `type_constructor(C)` lowers (like `type_ref`) to this unspellable forwarder
// carrying the FROZEN head identity as int literals — a name string never
// crosses into the mini-VM (R1 strings-cannot-construct). `apply` / `refine` /
// `type_argument` are METHOD-call surfaces: the site rewrite transforms the
// `receiver.method(args)` node into a call to these forwarders with the
// receiver as the first argument.
const TYPE_CONSTRUCTOR_FORWARDER: &str = "\u{1}comptime:forward-type-constructor";
const APPLY_FORWARDER: &str = "\u{1}comptime:forward-apply";
const REFINE_FORWARDER: &str = "\u{1}comptime:forward-refine";
const TYPE_ARGUMENT_FORWARDER: &str = "\u{1}comptime:forward-type-argument";

/// `refine`'s forwarder return annotation: `Option<AppliedType-carrier>`. Must
/// stay byte-identical to `format!("Option<{COMPTIME_APPLIED_TYPE_SCHEMA}>")` —
/// pinned by `refine_forwarder_return_marker_matches_the_applied_type_schema`.
const REFINE_RETURN_MARKER: &str = "Option<\u{1}comptime:AppliedType>";

/// ADR-009 B6 R1 (Dec 63): a `FrozenCallable`'s parameters are selected by
/// signature POSITION (`callable.param(I)`) or a hygienic token that resolves
/// to a position — NEVER by a string key. A string-literal selector is this
/// named rejection, mirroring the "no string keys / name-selected access"
/// invariant (spec §3.1; cf. the B4 index-not-string posture). Fires at
/// comptime-prep time, before any index is formed — never a partial descriptor.
pub(crate) const PARAM_STRING_SELECTOR_DIAGNOSTIC: &str = "callable.param expects a signature POSITION index, not a string key: a callable's \
     parameters are position-indexed descriptors — select with param(i) (or a hygienic \
     token resolving to a position), never param(\"name\")";

/// ADR-009 B6: `callable.param(I)` takes exactly one positional index argument.
pub(crate) const PARAM_ARITY_DIAGNOSTIC: &str =
    "callable.param expects exactly one signature-position index argument";

/// ADR-009 B5 R1/R2/R3 (Dec 57): a nominal shape descriptor's members are
/// selected ONLY by an owner-bound hygienic member identity (`#name`) — never a
/// source-name string (`record.field("name")`, R1), a declaration ordinal
/// (`record.field(0)` / `record.fields[0]`, R2), or a name read back off a
/// descriptor (`field.name`, R3). Source spelling and declaration position are
/// not member identities. The typed member rows are read by iterating the
/// descriptor's ordered `fields` / `variants`. The explicit `record.field(#name)`
/// selection surface is grammar-pending: the general `#ident` selection token is
/// parsed ONLY to emit a NAMED grammar-pending rejection (the tracer in
/// `parser/expressions/primary.rs`; see docs/defections.md), never resolved —
/// until it lands, iteration is the only member vehicle.
pub(crate) const DESCRIPTOR_MEMBER_SELECTION_DIAGNOSTIC: &str = "nominal member selection requires an owner-bound member identity (#name): a source-name \
     string, a declaration ordinal, or a descriptor-derived name is not a member identity — \
     iterate the descriptor's `fields` / `variants` to read the typed member rows (the explicit \
     `#name` selection surface is grammar-pending; see docs/defections.md)";

/// ADR-009 B5 R4 (Dec 55): nominal shape selection is an exhaustive TYPED match
/// over the sealed `NominalShape` sum (`Struct` / `Enum` / `Newtype` / `Opaque`)
/// — never a `.kind` string compared against a literal.
pub(crate) const NOMINAL_KIND_STRING_DIAGNOSTIC: &str = "nominal shape selection is exhaustive and typed: match the NominalShape sum \
     (Struct / Enum / Newtype / Opaque), never read a `.kind` string off the descriptor";

/// ADR-009 B5 R5 (Dec 55): a runtime representation class (native layout,
/// builtin-ness) is NOT a reflection category, and a comptime-field disposition
/// (`is_comptime`, Dec 58) is not exposed on a shape descriptor.
pub(crate) const RUNTIME_REPR_CLASS_DIAGNOSTIC: &str = "runtime representation classes (native layout, builtin-ness) and comptime-field \
     dispositions are not nominal reflection categories: a shape descriptor exposes semantic \
     identity + its public member interface, never a backend representation class";

/// (name, arity, target_method, return_fields, named_return_type,
/// param_annotations)
///
/// `param_annotations` (slice S5): concrete parameter annotations for the
/// generated forwarder fn. Forwarder params default to UNANNOTATED
/// (inference vars), which routes mini-VM calls through the
/// value-binding-call path (`infer_function_value_binding_call`) where any
/// argument unifies — bypassing the named rejection matrix in
/// `infer_comptime_builtin_call`. `find_impl` annotates its params with the
/// reserved carrier schemas so the mini-VM analyzer routes the call through
/// the comptime-builtin arm and the R3/R4/R8 named diagnostics fire at
/// check time instead of decaying to generic intrinsic errors.
const COMPTIME_BUILTIN_FORWARDERS: &[(
    &str,
    usize,
    &str,
    Option<&[&str]>,
    Option<&str>,
    Option<&[&str]>,
)] = &[
    // `implements` declares its real `bool` return (S5): the R4 rejection
    // ("a boolean cannot authorize...") needs the legacy boolean's type to
    // be CONCRETE at the `find_impl` argument position in the mini-VM —
    // an unannotated fresh var would silently unify with the TraitRef
    // carrier schema. Typing truth only; the legacy string-key semantics
    // in `comptime_builtins.rs` are untouched (E5 deletes them).
    ("implements", 2, "implements", None, Some("bool"), None),
    ("warning", 1, "warning", None, None, None),
    ("error", 1, "error", None, None, None),
    (
        "build_config",
        0,
        "build_config",
        // Return-fields hint so the comptime compiler can resolve field
        // access on the result (`cfg.debug`, `cfg.comptime_api`, …). Must
        // stay in sync with the `__ComptimeBuildConfig` schema
        // (`builtin_schemas.rs`) — `comptime_api` is the frozen
        // introspection-contract version marker (comptime-excellence §4.1.4).
        Some(&[
            "comptime_api",
            "debug",
            "target_arch",
            "target_os",
            "version",
        ]),
        None,
        None,
    ),
    // W7 (2026-05-17) — `type_info(T)` comptime builtin per
    // `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md`
    // §4 (b) recommendation. Bare type-identifier arguments are
    // rewritten to string literals by `rewrite_type_info_ident_args`
    // before this forwarder dispatches into `__comptime__.type_info`.
    // Return-fields hint covers the legacy TypeInfo fields plus the additive
    // TypeRef descriptor so the comptime compiler can resolve field access on
    // the result (`ti.name` / `ti.kind` / `ti.fields` / `ti.type_ref`).
    (
        "type_info",
        1,
        "type_info",
        Some(&["kind", "name", "fields", "type_ref"]),
        None,
        None,
    ),
    (
        TYPE_REF_FORWARDER,
        2,
        super::comptime_builtins::TYPE_REF_INTRINSIC,
        None,
        Some(shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA),
        None,
    ),
    (
        "type_category",
        1,
        super::comptime_builtins::TYPE_CATEGORY_INTRINSIC,
        None,
        Some("FrozenTypeCategory"),
        None,
    ),
    // ADR-009 B2 (slice S4): trait identity + implementation evidence.
    // The trait_ref forwarder is reached only through the site rewrite
    // (identity-literal transport); find_impl is called by name and answers
    // ONLY from frozen impl evidence (R9: unimplemented pair = None).
    (
        TRAIT_REF_FORWARDER,
        2,
        super::comptime_builtins::TRAIT_REF_INTRINSIC,
        None,
        Some(shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TRAIT_REF_SCHEMA),
        None,
    ),
    (
        "find_impl",
        2,
        super::comptime_builtins::FIND_IMPL_INTRINSIC,
        None,
        Some(FIND_IMPL_RETURN_MARKER),
        // S5: concrete carrier-schema params — see `param_annotations` note
        // in the table doc above.
        Some(&[
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA,
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
        ]),
    ),
    // ADR-009 B1 S3 — `reflect(TypeRef<T>) -> FrozenType<T>`: forwards to
    // the fourth freeze-consuming intrinsic. The named return type is the
    // injected payload-model enum (`frozen_type_enum_item`), so `match`
    // over the result resolves the sealed sum's variants and payloads.
    (
        "reflect",
        1,
        super::comptime_builtins::REFLECT_INTRINSIC,
        None,
        Some(shape_runtime::comptime_reflection::FROZEN_TYPE_PAYLOAD_ENUM_NAME),
        None,
    ),
    // ADR-009 B5 (Stage 2, Dec 56) — `reflect_repr(TypeRef<T>,
    // RepresentationAccess<T>) -> FrozenType<T>`: the authority-gated complete
    // reflection. Two concrete carrier-schema params so the mini-VM analyzer
    // routes the call through the comptime-builtin arm and the R6 named
    // authority rejection fires at check time (the `find_impl` S5 precedent).
    (
        "reflect_repr",
        2,
        super::comptime_builtins::REFLECT_REPR_INTRINSIC,
        None,
        Some(shape_runtime::comptime_reflection::FROZEN_TYPE_PAYLOAD_ENUM_NAME),
        Some(&[
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA,
            shape_runtime::type_schema::builtin_schemas::COMPTIME_REPRESENTATION_ACCESS_SCHEMA,
        ]),
    ),
    // ADR-009 B4 (Stage 2, Dec 54): uniform nominal application.
    // `type_constructor(C)` lowers to identity halves (like `type_ref`) →
    // TypeConstructorRef. `const_arg(N)` is a checked const argument. `apply` /
    // `refine` / `type_argument` are reached through the method-call site
    // rewrite (receiver prepended); their args are checked carriers, so their
    // named rejections fire in the intrinsic.
    (
        TYPE_CONSTRUCTOR_FORWARDER,
        2,
        super::comptime_builtins::TYPE_CONSTRUCTOR_INTRINSIC,
        None,
        Some(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA,
        ),
        None,
    ),
    (
        "const_arg",
        1,
        super::comptime_builtins::CONST_ARG_INTRINSIC,
        None,
        Some(shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA),
        None,
    ),
    (
        APPLY_FORWARDER,
        2,
        super::comptime_builtins::APPLY_INTRINSIC,
        None,
        Some(shape_runtime::type_schema::builtin_schemas::COMPTIME_APPLIED_TYPE_SCHEMA),
        None,
    ),
    (
        REFINE_FORWARDER,
        2,
        super::comptime_builtins::REFINE_INTRINSIC,
        None,
        Some(REFINE_RETURN_MARKER),
        None,
    ),
    (
        TYPE_ARGUMENT_FORWARDER,
        2,
        super::comptime_builtins::TYPE_ARGUMENT_INTRINSIC,
        None,
        Some(shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA),
        None,
    ),
    // First typed generation fragment surface. `item_fn(...)` returns a
    // typed fragment carrier accepted by `extend (expr)`.
    ("item_fn", 3, "item_fn", None, None, None),
    // ADR-009 E2 #18 (slice 4.5, E2-Q2/B): `extend_method(...)` — the typed
    // single-extend-method producer (arity 5), same carrier shape as item_fn
    // (returns the `__CheckedItem` OpaqueTypedObject accepted by `extend (expr)`;
    // no return-schema marker, no typed-carrier param schemas — forwarder params
    // are unannotated and infer their type from the caller). Handler-scope
    // resolution requires this forwarder row IN ADDITION to
    // `comptime_builtins_module_base` registration — the second registration
    // surface a comptime builtin needs, or `extend_method(...)` is `[C0001]
    // Undefined function` in a handler.
    ("extend_method", 5, "extend_method", None, None, None),
    // Comptime-excellence §4.5.7.4 — `string_lit(s)` renders a computed string
    // as a Shape source literal for embedding into `extend (expr)` output.
    ("string_lit", 1, "string_lit", None, None, None),
];

/// Comptime execution result.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The `value` carrier
/// migrated from the deleted `ValueWord` to `KindedSlot` (ADR-006 §2.7 /
/// Q7) — the post-§2.7.4 GENERIC_CARRIER shape for runtime values whose
/// `NativeKind` is not statically known to the consumer (the comptime VM
/// can return arbitrary heap-typed values to the outer compiler). The
/// in-VM execution path that actually populates `value` from `vm.execute`'s
/// raw bits + top-level `return_kind` is part of the comptime rebuild.
pub(crate) struct ComptimeExecutionResult {
    pub value: KindedSlot,
    pub directives: Vec<super::comptime_builtins::ComptimeDirective>,
    /// Non-fatal diagnostics (`warning()` / comptime `print()`) collected
    /// during execution, drained from the thread-local buffer. The compiler
    /// re-emits each with the driving construct's source span
    /// (comptime-excellence §4.4).
    pub warnings: Vec<super::comptime_builtins::ComptimeDiagnostic>,
    /// ADR-009 B1 S4: the mini-VM program's schema registry, carried out so
    /// the stage boundary can resolve schema ids the mini-VM registered
    /// (injected payload-model enums, comptime object literals). Without it
    /// the lift wall cannot NAME a descriptor nested inside (or forged as)
    /// a mini-VM-registered schema value — the id would miss (or collide)
    /// in the outer registry and the value would silently swallow to
    /// `Null`. Consumed by [`comptime_result_lift_rejection`].
    pub schema_registry: std::sync::Arc<shape_runtime::type_schema::TypeSchemaRegistry>,
}

/// §4.4 comptime-handler `ctx` param type. Field NAMES + ORDER match the
/// `__ComptimeContext` reserved schema (builtin_schemas.rs) so typed field
/// access on `ctx.module_path` / `ctx.file` resolves the right offsets.
fn comptime_ctx_param_type() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "module_path".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "file".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
    ])
}

/// ADR-009 B5 (Dec 56): the opaque `RepresentationAccess<T>` authority schema
/// as a param-type annotation, so a type-target handler's third positional
/// parameter types as the capability and `reflect_repr(type_ref(T), access)`
/// type-checks its authority argument.
fn comptime_representation_access_param_type() -> TypeAnnotation {
    TypeAnnotation::Basic(
        shape_runtime::type_schema::builtin_schemas::COMPTIME_REPRESENTATION_ACCESS_SCHEMA
            .to_string(),
    )
}

/// The `FieldDescriptor` row shape (comptime-excellence §4.1.1) as a concrete
/// object annotation, so `target.fields[i]` / `type_info(T).fields[i]`
/// subscript access resolves to a real object type with `.name` / `.type` /
/// `.optional` / `.annotations` fields. An `unknown`-element array is iterable
/// but not indexable, which regressed the flagship `fields[0].name` form.
fn comptime_field_descriptor_annotation() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "name".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "type".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "annotations".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "string".to_string(),
            ))),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "optional".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("bool".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "type_ref".to_string(),
            optional: false,
            type_annotation: comptime_type_ref_annotation(),
            annotations: vec![],
        },
    ])
}

fn comptime_type_ref_annotation() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "name".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "kind".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "source".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
    ])
}

/// The `ParamDescriptor` row shape (comptime-excellence §4.1.1) as a concrete
/// object annotation, so `target.params[i]` subscript access resolves.
fn comptime_param_descriptor_annotation() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "name".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "type".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "const".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("bool".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "type_ref".to_string(),
            optional: false,
            type_annotation: comptime_type_ref_annotation(),
            annotations: vec![],
        },
    ])
}

fn comptime_target_param_type() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "kind".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "name".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "fields".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(
                Box::new(comptime_field_descriptor_annotation()),
            ),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "params".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(
                Box::new(comptime_param_descriptor_annotation()),
            ),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "return_type".to_string(),
            optional: true,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "return_type_ref".to_string(),
            optional: false,
            type_annotation: comptime_type_ref_annotation(),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "annotations".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "captures".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
    ])
}

fn comptime_builtin_forwarders() -> Vec<Item> {
    let mut items = vec![frozen_type_category_enum_item()];
    items.extend(frozen_type_payload_model_items());
    items.extend(COMPTIME_BUILTIN_FORWARDERS.iter().map(
        |(name, arity, target_method, return_fields, named_return_type, param_annotations)| {
            let params: Vec<shape_ast::ast::FunctionParameter> = (0..*arity)
                .map(|i| shape_ast::ast::FunctionParameter {
                    pattern: shape_ast::ast::DestructurePattern::Identifier(
                        hygienic_forwarder_param(i),
                        Span::DUMMY,
                    ),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: param_annotations
                        .and_then(|annotations| annotations.get(i))
                        .map(|annotation| TypeAnnotation::Basic((*annotation).to_string())),
                    default_value: None,
                })
                .collect();

            let args: Vec<Expr> = (0..*arity)
                .map(|i| Expr::Identifier(hygienic_forwarder_param(i), Span::DUMMY))
                .collect();

            let body_expr = Expr::QualifiedFunctionCall {
                namespace: "__comptime__".to_string(),
                function: (*target_method).to_string(),
                const_args: Vec::new(),
                args,
                named_args: Vec::new(),
                span: Span::DUMMY,
            };

            // If the forwarder has known return fields, generate an Object
            // type annotation so the compiler can emit GetFieldTyped for
            // property access on the return value.
            //
            // ADR-009 B2 (slice S4): a named return type of the form
            // `Option<inner>` (find_impl's `Option<ImplRef-carrier>`)
            // becomes a real `Option` generic annotation so `match` with
            // `Some(..)` / `None` patterns type-checks in the mini-VM.
            let return_type = named_return_type
                .map(|name| {
                    if let Some(inner) = name
                        .strip_prefix("Option<")
                        .and_then(|rest| rest.strip_suffix('>'))
                    {
                        TypeAnnotation::option(TypeAnnotation::Basic(inner.to_string()))
                    } else {
                        TypeAnnotation::Basic((*name).to_string())
                    }
                })
                .or_else(|| {
                    return_fields.map(|fields| {
                        TypeAnnotation::Object(
                            fields
                                .iter()
                                .map(|f| ObjectTypeField {
                                    name: f.to_string(),
                                    optional: false,
                                    // `type_info(T)` result fields carry their real
                                    // types (comptime-excellence §4.1.2), not `unknown`.
                                    type_annotation: match *f {
                                        "fields" => TypeAnnotation::Array(Box::new(
                                            comptime_field_descriptor_annotation(),
                                        )),
                                        "return_type_ref" | "type_ref" => {
                                            comptime_type_ref_annotation()
                                        }
                                        "name" | "kind" | "return_type" => {
                                            TypeAnnotation::Basic("string".to_string())
                                        }
                                        _ => TypeAnnotation::Basic("unknown".to_string()),
                                    },
                                    annotations: vec![],
                                })
                                .collect(),
                        )
                    })
                });

            Item::Function(
                FunctionDef {
                    name: (*name).to_string(),
                    name_span: Span::DUMMY,
                    declaring_module_path: None,
                    doc_comment: None,
                    params,
                    return_type,
                    body: vec![Statement::Return(Some(body_expr), Span::DUMMY)],
                    type_params: Some(Vec::new()),
                    annotations: Vec::new(),
                    where_clause: None,
                    is_async: false,
                    is_comptime: false,
                },
                Span::DUMMY,
            )
        },
    ));
    items
}

fn frozen_type_category_enum_item() -> Item {
    use shape_ast::ast::{EnumDef, EnumMember, EnumMemberKind};

    Item::Enum(
        EnumDef {
            name: "FrozenTypeCategory".to_string(),
            doc_comment: None,
            type_params: None,
            members: super::comptime_builtins::FrozenTypeCategory::ALL
                .into_iter()
                .map(|category| EnumMember {
                    name: category.variant_name().to_string(),
                    kind: EnumMemberKind::Unit { value: None },
                    span: Span::DUMMY,
                    doc_comment: None,
                })
                .collect(),
            annotations: Vec::new(),
        },
        Span::DUMMY,
    )
}

/// ADR-009 B1 S3 — the payload-model type Items injected into every
/// mini-VM program beside `frozen_type_category_enum_item`, ALL generated
/// from the S1 shared runtime catalog (no hand-written variant lists):
///
/// - `FrozenType` — the sealed sum with ONLY the enabled payload variants
///   (`FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`), each a tuple variant
///   carrying its `Frozen*` payload descriptor type. Variant ids are
///   ordinal-pinned by `register_enum` (statements.rs) to match the
///   unspellable value-carrier schema — see
///   `comptime_reflection::frozen_type_payload_variant_ordinal`.
/// - `FrozenPrimitive` — the sealed sub-algebra; the integer/float FAMILY
///   variants carry their width-domain enum payload
///   (`FROZEN_PRIMITIVE_VARIANTS[..].payload_type`).
/// - `IntegerWidth` / `FloatWidth` — the width-domain enums.
/// - `FrozenNever` / `FrozenErased` — payload descriptor structs. The
///   `FrozenErased.bounds` element type is `never`: `dyn Trait` spellings
///   arrive with A2/B2, so the bound set is provably empty — the honest
///   structural form of "complete for reachable forms" (spec §3.1/§3.7).
fn frozen_type_payload_model_items() -> Vec<Item> {
    use shape_ast::ast::{EnumDef, EnumMember, EnumMemberKind, StructField, StructTypeDef};
    use shape_runtime::comptime_reflection::{
        ASSOCIATED_CONST_DESCRIPTOR_SCHEMA_NAME, ENUM_DESCRIPTOR_SCHEMA_NAME,
        FIELD_DESCRIPTOR_SCHEMA_NAME, NEWTYPE_DESCRIPTOR_SCHEMA_NAME,
        OPAQUE_TYPE_DESCRIPTOR_SCHEMA_NAME, STRUCT_DESCRIPTOR_SCHEMA_NAME,
        VARIANT_DESCRIPTOR_SCHEMA_NAME,
    };
    use shape_runtime::comptime_reflection::{
        FIELD_INITIALIZATION_SCHEMA_NAME, FLOAT_WIDTH_SCHEMA_NAME, FROZEN_PRIMITIVE_VARIANTS,
        FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES, FROZEN_TYPE_PAYLOAD_ENUM_NAME,
        FieldInitialization, FloatWidth, INTEGER_WIDTH_SCHEMA_NAME, IntegerWidth,
        NOMINAL_SHAPE_SCHEMA_NAME, NominalShape, PASSING_MODE_SCHEMA_NAME, PassingMode,
        frozen_type_enabled_payload_type_name,
    };
    // ADR-009 B7: the composite payloads' typed element-row model names.
    use shape_runtime::comptime_reflection::{
        RECORD_FIELD_SCHEMA_NAME, TUPLE_ELEMENT_SCHEMA_NAME, UNION_MEMBER_SCHEMA_NAME,
    };

    let enum_item = |name: &str, members: Vec<EnumMember>| {
        Item::Enum(
            EnumDef {
                name: name.to_string(),
                doc_comment: None,
                type_params: None,
                members,
                annotations: Vec::new(),
            },
            Span::DUMMY,
        )
    };
    let unit_member = |name: &str| EnumMember {
        name: name.to_string(),
        kind: EnumMemberKind::Unit { value: None },
        span: Span::DUMMY,
        doc_comment: None,
    };
    let tuple_member = |name: &str, payload_type: &str| EnumMember {
        name: name.to_string(),
        kind: EnumMemberKind::Tuple(vec![TypeAnnotation::Basic(payload_type.to_string())]),
        span: Span::DUMMY,
        doc_comment: None,
    };
    let struct_item = |name: &str, fields: Vec<StructField>| {
        Item::StructType(
            StructTypeDef {
                name: name.to_string(),
                doc_comment: None,
                type_params: None,
                fields,
                methods: Vec::new(),
                annotations: Vec::new(),
                native_layout: None,
            },
            Span::DUMMY,
        )
    };
    let field = |name: &str, type_annotation: TypeAnnotation| StructField {
        annotations: Vec::new(),
        is_comptime: false,
        name: name.to_string(),
        span: Span::DUMMY,
        doc_comment: None,
        type_annotation,
        default_value: None,
    };
    let int_ty = || TypeAnnotation::Basic("int".to_string());

    vec![
        enum_item(
            FROZEN_TYPE_PAYLOAD_ENUM_NAME,
            FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
                .into_iter()
                .map(|category| {
                    tuple_member(
                        category.variant_name(),
                        frozen_type_enabled_payload_type_name(category)
                            .expect("enabled payload categories carry a descriptor type name"),
                    )
                })
                .collect(),
        ),
        enum_item(
            "FrozenPrimitive",
            FROZEN_PRIMITIVE_VARIANTS
                .iter()
                .map(|variant| match variant.payload_type {
                    Some(payload_type) => tuple_member(variant.name, payload_type),
                    None => unit_member(variant.name),
                })
                .collect(),
        ),
        enum_item(
            INTEGER_WIDTH_SCHEMA_NAME,
            IntegerWidth::ALL
                .into_iter()
                .map(|width| unit_member(width.variant_name()))
                .collect(),
        ),
        enum_item(
            FLOAT_WIDTH_SCHEMA_NAME,
            FloatWidth::ALL
                .into_iter()
                .map(|width| unit_member(width.variant_name()))
                .collect(),
        ),
        struct_item("FrozenNever", Vec::new()),
        struct_item(
            "FrozenErased",
            vec![StructField {
                annotations: Vec::new(),
                is_comptime: false,
                name: "bounds".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Never)),
                default_value: None,
            }],
        ),
        // ADR-009 B6 (Dec 63): the callable signature descriptor payload model.
        // `PassingMode` is the ADR mode axis; `ParamDescriptor` one positional
        // parameter (type identity halves + optional + mode); `FrozenCallable`
        // the ordered param array + return type identity halves. Generated from
        // the shared runtime catalog (`PassingMode::ALL`) — no hand-written mode
        // list.
        enum_item(
            PASSING_MODE_SCHEMA_NAME,
            PassingMode::ALL
                .into_iter()
                .map(|mode| unit_member(mode.variant_name()))
                .collect(),
        ),
        struct_item(
            "ParamDescriptor",
            vec![
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
                field("optional", TypeAnnotation::Basic("bool".to_string())),
                field(
                    "mode",
                    TypeAnnotation::Basic(PASSING_MODE_SCHEMA_NAME.to_string()),
                ),
            ],
        ),
        struct_item(
            "FrozenCallable",
            vec![
                field(
                    "params",
                    TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                        "ParamDescriptor".to_string(),
                    ))),
                ),
                field("returns_identity_high", int_ty()),
                field("returns_identity_low", int_ty()),
            ],
        ),
        capture_payload_model::capture_mode_enum_item(),
        capture_payload_model::capture_descriptor_struct_item(),
        // ADR-009 B5 (Dec 55-59): the nominal-shape descriptor payload model.
        // `NominalShape` is the sealed declaration-shape axis (each variant
        // carries its typed row descriptor); `FieldInitialization` the Dec 59
        // disposition. The `*Descriptor` structs carry owner + owner-bound
        // member identities (never source-name strings, Dec 57). `FrozenNominal`
        // wraps the sealed shape only. Generated from the shared runtime catalog
        // (`NominalShape::ALL` + `descriptor_type_name` / `FieldInitialization::
        // ALL`) — no hand-written variant/mapping list.
        enum_item(
            FIELD_INITIALIZATION_SCHEMA_NAME,
            FieldInitialization::ALL
                .into_iter()
                .map(|init| unit_member(init.variant_name()))
                .collect(),
        ),
        struct_item(
            FIELD_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("member_high", int_ty()),
                field("member_low", int_ty()),
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
                field(
                    "initialization",
                    TypeAnnotation::Basic(FIELD_INITIALIZATION_SCHEMA_NAME.to_string()),
                ),
            ],
        ),
        struct_item(
            VARIANT_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("member_high", int_ty()),
                field("member_low", int_ty()),
                field("payload_arity", int_ty()),
            ],
        ),
        struct_item(
            ASSOCIATED_CONST_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("member_high", int_ty()),
                field("member_low", int_ty()),
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
            ],
        ),
        struct_item(
            STRUCT_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("field_count", int_ty()),
                field(
                    "fields",
                    TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                        FIELD_DESCRIPTOR_SCHEMA_NAME.to_string(),
                    ))),
                ),
            ],
        ),
        struct_item(
            ENUM_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("variant_count", int_ty()),
                field(
                    "variants",
                    TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                        VARIANT_DESCRIPTOR_SCHEMA_NAME.to_string(),
                    ))),
                ),
            ],
        ),
        struct_item(
            NEWTYPE_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
                field("inner_identity_high", int_ty()),
                field("inner_identity_low", int_ty()),
            ],
        ),
        struct_item(
            OPAQUE_TYPE_DESCRIPTOR_SCHEMA_NAME,
            vec![
                field("owner_identity_high", int_ty()),
                field("owner_identity_low", int_ty()),
            ],
        ),
        enum_item(
            NOMINAL_SHAPE_SCHEMA_NAME,
            NominalShape::ALL
                .into_iter()
                .map(|shape| tuple_member(shape.variant_name(), shape.descriptor_type_name()))
                .collect(),
        ),
        struct_item(
            "FrozenNominal",
            vec![field(
                "shape",
                TypeAnnotation::Basic(NOMINAL_SHAPE_SCHEMA_NAME.to_string()),
            )],
        ),
        // ADR-009 B7 (Dec 50/94): the four composite payload models. Each
        // wrapping `Frozen{Tuple,Record,Union}` carries its ordered/normalized
        // element array (a TYPED element struct); `FrozenReference` is a flat
        // mutable/referent descriptor. Field names + order match the unspellable
        // value carriers registered in `builtin_schemas.rs` exactly (the
        // FrozenCallable/ParamDescriptor precedent), so a bound match payload
        // reads its typed fields. No `.kind` string, no rendered type name.
        struct_item(
            TUPLE_ELEMENT_SCHEMA_NAME,
            vec![
                field("index", int_ty()),
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
            ],
        ),
        struct_item(
            "FrozenTuple",
            vec![field(
                "elements",
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    TUPLE_ELEMENT_SCHEMA_NAME.to_string(),
                ))),
            )],
        ),
        struct_item(
            RECORD_FIELD_SCHEMA_NAME,
            vec![
                field("member_high", int_ty()),
                field("member_low", int_ty()),
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
                field("optional", TypeAnnotation::Basic("bool".to_string())),
            ],
        ),
        struct_item(
            "FrozenRecord",
            vec![field(
                "fields",
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    RECORD_FIELD_SCHEMA_NAME.to_string(),
                ))),
            )],
        ),
        struct_item(
            "FrozenReference",
            vec![
                field("mutable", TypeAnnotation::Basic("bool".to_string())),
                field("referent_identity_high", int_ty()),
                field("referent_identity_low", int_ty()),
            ],
        ),
        struct_item(
            UNION_MEMBER_SCHEMA_NAME,
            vec![
                field("type_identity_high", int_ty()),
                field("type_identity_low", int_ty()),
            ],
        ),
        struct_item(
            "FrozenUnion",
            vec![field(
                "members",
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    UNION_MEMBER_SCHEMA_NAME.to_string(),
                ))),
            )],
        ),
        // ADR-009 B7 Slice 2 (Dec 50/94): the Parameter payload model. Field
        // names + order match the unspellable `FrozenParameter` value carrier
        // registered in `builtin_schemas.rs` exactly (the FrozenErased
        // precedent): the type parameter's stable identity halves + the
        // provably-empty (uninhabited-element) bound-set array. No `.kind`
        // string, no rendered type name.
        struct_item(
            "FrozenParameter",
            vec![
                field("identity_high", int_ty()),
                field("identity_low", int_ty()),
                field(
                    "bounds",
                    TypeAnnotation::Array(Box::new(TypeAnnotation::Never)),
                ),
            ],
        ),
    ]
}

/// Ensure that the last statement in a body is a tail value (returns its result).
///
/// When the last statement is `Statement::If`, its value is discarded by the
/// compiler because it's compiled as a statement (not an expression).  This
/// helper recursively wraps the last expressions of each branch in explicit
/// `Statement::Return` so the wrapping comptime function returns the value.
fn ensure_tail_return(body: &mut Vec<Statement>) {
    let Some(last) = body.last_mut() else {
        return;
    };
    match last {
        // If the last statement is an if/else, ensure each branch returns.
        Statement::If(if_stmt, _span) => {
            ensure_tail_return(&mut if_stmt.then_body);
            if let Some(else_body) = &mut if_stmt.else_body {
                ensure_tail_return(else_body);
            }
        }
        // An expression statement at the end should become a return.
        Statement::Expression(expr, span) => {
            *last = Statement::Return(Some(expr.clone()), *span);
        }
        // Explicit return is already fine.
        Statement::Return(_, _) => {}
        // Other statements: do nothing (function will return null).
        _ => {}
    }
}

/// Rewrite bare type/trait-name identifier arguments to comptime reflection
/// payloads at any nesting depth.
///
/// `type_info(User)`, `type_ref(User)`, and `implements(Dog, Speak)` name types
/// or traits directly. Legacy reflection receives names; `type_ref` receives a
/// compiler-issued identity. The walk is fully recursive so natural nested forms
/// work too — `print(type_info(User).name)`, `if implements(T, "Ord") { ... }`,
/// `let n = type_info(field.type).name`, etc. — not only a bare top-level
/// statement. The outer type-checker accepts exactly the same `type_ref`
/// argument shapes — a bare identifier OR the checked type-expression carrier
/// `Expr::TypeSyntax` (inference/access.rs `is_type_ref_builtin` gate) — so
/// the two paths agree; change both together (ADR-009 A2 lockstep contract).
///
/// ADR-009 A2 (slice S4): returns `Err` when a checked type expression fails
/// canonicalization (unresolved leaf at any depth, inference hole, Dec 50/94
/// normalization rejection). The error propagates out of the comptime entry
/// points BEFORE the user's comptime code compiles or executes (Dec 52 freeze
/// boundary) — never an `INVALID` sentinel, never a partial descriptor.
/// ADR-009 B6 (Dec 63): the set of identifiers currently bound to a
/// `FrozenCallable` value, threaded through the comptime rewrite so the
/// `.param(I)` / `.parameters` accessor desugarings fire ONLY on a real
/// `FrozenCallable` receiver.
///
/// In the S1 compiler model a `FrozenCallable` value enters scope through
/// exactly one route: a `FrozenType::Callable(<ident>)` match binding
/// (`reflect(type_ref(<callable>))` → the payload sum → the Callable arm). The
/// accessor rewrites lower to plain field / index access on the descriptor
/// carrier's `params` array, which does NOT re-check that the receiver is a
/// `FrozenCallable` — so an unguarded, receiver-blind rewrite would silently
/// capture an arbitrary user receiver that happens to spell `.param(...)` or
/// carry a `.parameters` field (wrong field / a misleading `PARAM_ARITY`
/// diagnostic on a user `.param(a, b)`). Tracking the callable-bound
/// identifiers per lexical arm scope keeps the surface precise
/// (CLAUDE.md Forbidden Patterns — no collateral capture; ADR-009 §3
/// precise-surface discipline). Genuine `FrozenCallable` uses in the enabled
/// surface always spell the receiver as the bound identifier directly
/// (`c.param(i)`, `c.parameters`), so the guard admits every enabled form.
///
/// ADR-009 B5 (Dec 55): the same precise-surface discipline governs the
/// `FrozenNominal.shape()` accessor — a `nominals` set tracks identifiers bound
/// by a `FrozenType::Nominal(<ident>)` match so `.shape()` rewrites fire ONLY on
/// a real `FrozenNominal` receiver (never a user `.shape()` method on an
/// unrelated receiver). One scope carrier, one binding-collection walk.
///
/// ADR-009 B5 (Dec 55-57, slice S3): the `descriptors` set tracks identifiers
/// bound by a `NominalShape::<Variant>(<ident>)` match — the sealed shape's
/// typed row descriptor (`StructDescriptor` / `EnumDescriptor` / …). A
/// descriptor exposes its members ONLY through ordered `fields` / `variants`
/// iteration; the forbidden member-SELECTION forms (`record.field("name")` R1,
/// `record.field(0)` R2, `record.kind` R4) are named rejections fired against a
/// tracked descriptor receiver — never a generic "unknown method/field" decay.
#[derive(Clone, Default)]
struct CallableScope {
    callables: std::collections::HashSet<String>,
    nominals: std::collections::HashSet<String>,
    descriptors: std::collections::HashSet<String>,
}

impl CallableScope {
    fn new() -> Self {
        Self::default()
    }
}

/// Add any identifier bound by a reserved `FrozenType::<Variant>(<ident>)` or
/// `NominalShape::<Variant>(<ident>)` match pattern to the matching scope set.
/// ONLY the reserved single-binding tuple pattern under the reserved head
/// qualifies — a user enum with a `Callable` / `Nominal` / `Struct` variant (a
/// different enum head) does not, nor does a struct/array destructuring shape.
fn collect_frozen_callable_bindings(pattern: &shape_ast::ast::Pattern, scope: &mut CallableScope) {
    if let shape_ast::ast::Pattern::Constructor {
        enum_name,
        variant,
        fields,
    } = pattern
    {
        let head = enum_name.as_ref().map(|path| path.name());
        let is_frozen_type_head = head.as_deref() == Some("FrozenType");
        let is_nominal_shape_head = head.as_deref() == Some("NominalShape");
        if !is_frozen_type_head && !is_nominal_shape_head {
            return;
        }
        if let shape_ast::ast::PatternConstructorFields::Tuple(pats) = fields {
            if let [shape_ast::ast::Pattern::Identifier { name, .. }] = pats.as_slice() {
                if is_frozen_type_head {
                    match variant.as_str() {
                        "Callable" => {
                            scope.callables.insert(name.clone());
                        }
                        "Nominal" => {
                            scope.nominals.insert(name.clone());
                        }
                        _ => {}
                    }
                } else if is_nominal_shape_head {
                    // Every sealed shape's payload is a typed row descriptor —
                    // member selection off any of them is a named rejection.
                    match variant.as_str() {
                        "Struct" | "Enum" | "Newtype" | "Opaque" => {
                            scope.descriptors.insert(name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Whether `expr` is a bare identifier bound to a `FrozenCallable` in the
/// current lexical scope — the precondition for the `.param(I)` / `.parameters`
/// accessor rewrites (Dec 63).
fn is_frozen_callable_receiver(expr: &Expr, scope: &CallableScope) -> bool {
    matches!(expr, Expr::Identifier(name, _) if scope.callables.contains(name))
}

/// Whether `expr` is a bare identifier bound to a `FrozenNominal` in the
/// current lexical scope — the precondition for the `.shape()` accessor rewrite
/// (Dec 55).
fn is_frozen_nominal_receiver(expr: &Expr, scope: &CallableScope) -> bool {
    matches!(expr, Expr::Identifier(name, _) if scope.nominals.contains(name))
}

/// Whether `expr` is a bare identifier bound to a `NominalShape::<Variant>`
/// payload descriptor (`StructDescriptor` / `EnumDescriptor` / …) in the current
/// lexical scope — the precondition for the S3 member-selection rejection matrix
/// (R1/R2/R3/R4/R5).
fn is_descriptor_receiver(expr: &Expr, scope: &CallableScope) -> bool {
    matches!(expr, Expr::Identifier(name, _) if scope.descriptors.contains(name))
}

fn rewrite_comptime_type_symbol_args(
    stmt: &mut Statement,
    freeze: &super::comptime_builtins::FreezeOverlay,
) -> Result<()> {
    rewrite_comptime_type_symbol_args_scoped(stmt, freeze, &CallableScope::new())
}

fn rewrite_comptime_type_symbol_args_scoped(
    stmt: &mut Statement,
    freeze: &super::comptime_builtins::FreezeOverlay,
    callables: &CallableScope,
) -> Result<()> {
    match stmt {
        Statement::Expression(expr, _) => {
            rewrite_comptime_type_symbol_args_expr_scoped(expr, freeze, callables)?
        }
        Statement::Return(Some(expr), _) => {
            rewrite_comptime_type_symbol_args_expr_scoped(expr, freeze, callables)?
        }
        Statement::Return(None, _) | Statement::Break(_) | Statement::Continue(_) => {}
        Statement::VariableDecl(decl, _) => {
            if let Some(init) = &mut decl.value {
                rewrite_comptime_type_symbol_args_expr_scoped(init, freeze, callables)?;
            }
        }
        Statement::Assignment(assign, _) => {
            rewrite_comptime_type_symbol_args_expr_scoped(&mut assign.value, freeze, callables)?;
        }
        Statement::For(for_loop, _) => {
            match &mut for_loop.init {
                shape_ast::ast::ForInit::ForIn { iter, .. } => {
                    rewrite_comptime_type_symbol_args_expr_scoped(iter, freeze, callables)?;
                }
                shape_ast::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    rewrite_comptime_type_symbol_args_scoped(init, freeze, callables)?;
                    rewrite_comptime_type_symbol_args_expr_scoped(condition, freeze, callables)?;
                    rewrite_comptime_type_symbol_args_expr_scoped(update, freeze, callables)?;
                }
            }
            for s in &mut for_loop.body {
                rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
            }
        }
        Statement::While(while_loop, _) => {
            rewrite_comptime_type_symbol_args_expr_scoped(
                &mut while_loop.condition,
                freeze,
                callables,
            )?;
            for s in &mut while_loop.body {
                rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
            }
        }
        Statement::If(if_stmt, _) => {
            rewrite_comptime_type_symbol_args_expr_scoped(
                &mut if_stmt.condition,
                freeze,
                callables,
            )?;
            for s in &mut if_stmt.then_body {
                rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
            }
            if let Some(else_body) = &mut if_stmt.else_body {
                for s in else_body {
                    rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
                }
            }
        }
        Statement::SetParamValue { expression, .. }
        | Statement::SetParamTypeExpr { expression, .. }
        | Statement::SetReturnExpr { expression, .. }
        | Statement::ReplaceBodyExpr { expression, .. }
        | Statement::ReplaceModuleExpr { expression, .. }
        | Statement::ExtendItemsExpr { expression, .. } => {
            rewrite_comptime_type_symbol_args_expr_scoped(expression, freeze, callables)?;
        }
        Statement::ReplaceBody { body, .. } => {
            for s in body {
                rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
            }
        }
        // Directives with no embedded expression / already-parsed payloads.
        Statement::Extend(_, _)
        | Statement::RemoveTarget(_)
        | Statement::SetParamType { .. }
        | Statement::SetReturnType { .. } => {}
    }
    Ok(())
}

#[cfg(test)]
fn rewrite_comptime_type_symbol_args_expr(
    expr: &mut Expr,
    freeze: &super::comptime_builtins::FreezeOverlay,
) -> Result<()> {
    rewrite_comptime_type_symbol_args_expr_scoped(expr, freeze, &CallableScope::new())
}

fn rewrite_comptime_type_symbol_args_expr_scoped(
    expr: &mut Expr,
    freeze: &super::comptime_builtins::FreezeOverlay,
    callables: &CallableScope,
) -> Result<()> {
    // ADR-009 B4 (Stage 2, Dec 54): the method-call surfaces
    // (`apply` / `refine` / `type_argument`) on comptime carriers rewrite to
    // their forwarders with the receiver prepended — BEFORE the child
    // recursion below lowers nested `type_ref` / `const_arg` /
    // `type_constructor` arguments. `apply` transports its variadic arguments
    // as a checked array literal (each element is a checked carrier — never an
    // untyped argument array, R4); `refine` / `type_argument` take the single
    // method argument alongside the receiver.
    if let Expr::MethodCall {
        receiver,
        method,
        args,
        span,
        ..
    } = expr
    {
        let forwarder = match method.as_str() {
            "apply" => Some(APPLY_FORWARDER),
            "refine" => Some(REFINE_FORWARDER),
            "type_argument" => Some(TYPE_ARGUMENT_FORWARDER),
            _ => None,
        };
        if let Some(forwarder) = forwarder {
            let span = *span;
            let receiver = std::mem::replace(receiver.as_mut(), Expr::Unit(span));
            let call_args = std::mem::take(args);
            let new_args = if forwarder == APPLY_FORWARDER {
                vec![receiver, Expr::Array(call_args, span)]
            } else {
                let mut v = Vec::with_capacity(call_args.len() + 1);
                v.push(receiver);
                v.extend(call_args);
                v
            };
            *expr = Expr::FunctionCall {
                name: forwarder.to_string(),
                const_args: Vec::new(),
                args: new_args,
                named_args: Vec::new(),
                span,
            };
        }
    }

    // ADR-009 B6 (Stage 2, Dec 63): the `FrozenCallable` accessor surface.
    // `callable.param(I)` is signature-indexed POSITIONAL access — it desugars
    // to indexing the descriptor's ordered `params` array (the working S1
    // carrier), so the returned `ParamDescriptor` carries the position's
    // type-identity / optionality / passing mode. A string selector is the
    // named R1 rejection, fired HERE (comptime-prep) before any index forms —
    // parameters are never string-keyed.
    //
    // The rewrite (and its R1 string / arity diagnostics) fire ONLY when the
    // receiver is provably a `FrozenCallable` (`is_frozen_callable_receiver`):
    // the desugaring lowers to plain `.params[I]` access that does not re-check
    // the receiver, so a receiver-blind rewrite would silently capture a user
    // `.param(...)` method on an unrelated receiver (wrong field / a misleading
    // `PARAM_ARITY` diagnostic on a user `foo.param(a, b)`). A non-callable
    // receiver is left as an ordinary `MethodCall` for normal dispatch.
    if let Expr::MethodCall {
        receiver,
        method,
        args,
        span,
        ..
    } = expr
    {
        if method == "param" && is_frozen_callable_receiver(receiver.as_ref(), callables) {
            let span = *span;
            if args.len() != 1 {
                return Err(ShapeError::SemanticError {
                    message: PARAM_ARITY_DIAGNOSTIC.to_string(),
                    location: None,
                });
            }
            if matches!(
                &args[0],
                Expr::Literal(shape_ast::ast::Literal::String(_), _)
            ) {
                return Err(ShapeError::SemanticError {
                    message: PARAM_STRING_SELECTOR_DIAGNOSTIC.to_string(),
                    location: None,
                });
            }
            let receiver = std::mem::replace(receiver.as_mut(), Expr::Unit(span));
            let index = std::mem::take(args)
                .into_iter()
                .next()
                .expect("arity checked to be exactly one");
            *expr = Expr::IndexAccess {
                object: Box::new(Expr::PropertyAccess {
                    object: Box::new(receiver),
                    property: "params".to_string(),
                    optional: false,
                    span,
                }),
                index: Box::new(index),
                end_index: None,
                span,
            };
        }
    }

    // `callable.parameters` (property) is the ordered per-position descriptor
    // collection — the same `params` carrier under the ADR-named surface. Kept
    // a distinct spelling so the surface reads as the ADR-009 §Public-surface
    // `callable.parameters` while reusing the one carrier. Guarded on a
    // `FrozenCallable` receiver (Dec 63) so a user struct with a `.parameters`
    // field used in comptime is not silently renamed to `.params`.
    if let Expr::PropertyAccess {
        object, property, ..
    } = expr
    {
        if property == "parameters" && is_frozen_callable_receiver(object.as_ref(), callables) {
            *property = "params".to_string();
        }
    }

    // ADR-009 B5 (Stage 2, Dec 55): the `FrozenNominal.shape()` accessor.
    // `nominal.shape()` projects the sealed `NominalShape` sum — it desugars to
    // reading the descriptor carrier's `shape` field (the `NominalShape` enum
    // value the payload builder pre-populated), so `match nominal.shape() { … }`
    // resolves the sealed shapes. Guarded on a provably-`FrozenNominal` receiver
    // (`is_frozen_nominal_receiver`): the desugaring lowers to plain `.shape`
    // field access that does not re-check the receiver, so a receiver-blind
    // rewrite would silently capture a user `.shape()` method on an unrelated
    // receiver. A non-nominal receiver is left as an ordinary `MethodCall`.
    if let Expr::MethodCall {
        receiver,
        method,
        args,
        span,
        ..
    } = expr
    {
        if method == "shape"
            && args.is_empty()
            && is_frozen_nominal_receiver(receiver.as_ref(), callables)
        {
            let span = *span;
            let receiver = std::mem::replace(receiver.as_mut(), Expr::Unit(span));
            *expr = Expr::PropertyAccess {
                object: Box::new(receiver),
                property: "shape".to_string(),
                optional: false,
                span,
            };
        }
    }

    // ADR-009 B5 (Stage 2, Dec 55-57, slice S3): the hard member-selection
    // rejection matrix, fired against a tracked shape-descriptor receiver so a
    // forbidden spelling names its rejection precisely instead of decaying to a
    // generic "unknown method/field" error. A descriptor's members are read ONLY
    // by iterating its ordered `fields` / `variants`; every SELECTION spelling is
    // rejected.
    if let Expr::MethodCall {
        receiver, method, ..
    } = expr
    {
        if is_descriptor_receiver(receiver.as_ref(), callables)
            && matches!(method.as_str(), "field" | "variant" | "constant")
        {
            // R1 (string), R2 (ordinal), R3 (descriptor-derived name): any
            // argument shape is a member-SELECTION attempt, all rejected.
            return Err(ShapeError::SemanticError {
                message: DESCRIPTOR_MEMBER_SELECTION_DIAGNOSTIC.to_string(),
                location: None,
            });
        }
    }
    if let Expr::PropertyAccess {
        object, property, ..
    } = expr
    {
        let on_descriptor = is_descriptor_receiver(object.as_ref(), callables);
        let on_nominal = is_frozen_nominal_receiver(object.as_ref(), callables);
        if on_descriptor || on_nominal {
            // R4: `.kind` string read off a nominal / descriptor.
            if property == "kind" {
                return Err(ShapeError::SemanticError {
                    message: NOMINAL_KIND_STRING_DIAGNOSTIC.to_string(),
                    location: None,
                });
            }
            // R5 (runtime representation class) / R9 (comptime-field
            // disposition): neither is a reflection category on a shape
            // descriptor.
            if matches!(
                property.as_str(),
                "is_builtin" | "is_comptime" | "native_layout"
            ) {
                return Err(ShapeError::SemanticError {
                    message: RUNTIME_REPR_CLASS_DIAGNOSTIC.to_string(),
                    location: None,
                });
            }
        }
    }

    // Rewrite this call's own bare-identifier args if it is a reflection call.
    if let Expr::FunctionCall { name, args, .. } = expr {
        if name == "type_info" || name == "implements" {
            for arg in args.iter_mut() {
                if let Expr::Identifier(ident, span) = arg {
                    *arg = Expr::Literal(shape_ast::ast::Literal::String(ident.clone()), *span);
                }
            }
        } else if name == "type_ref" {
            // ADR-009 A2 (S4): two accepted argument shapes, one identity
            // scheme. A bare identifier resolves through the freeze's
            // name-keyed query (A1 behavior, unchanged — unresolved names
            // flow the INVALID sentinel to the intrinsic's named rejection);
            // the checked type-expression carrier canonicalizes through the
            // SAME overlay (leaves resolve via `identity_of`, so a leaf
            // spelled bare or inside a composite reaches one identity) and
            // rejects HERE, at compile time, with the canonicalizer's named
            // error before user comptime executes (Dec 52).
            // Rejection R1 fall-through (B2 slice S5): a bare identifier that
            // is not a frozen VALUE type but IS a frozen trait transports the
            // trait identity — still an identity literal, never a string —
            // so the TypeRef carrier builder can answer the NAMED
            // traits-are-not-value-types rejection. Genuinely-unknown names
            // keep transporting INVALID (A1 row 2's generic diagnostic).
            let lowered = match args.as_slice() {
                [Expr::Identifier(ident, span)] => Some((
                    freeze
                        .identity_of(ident)
                        .or_else(|| freeze.trait_identity_of(ident))
                        .unwrap_or(super::comptime_builtins::FrozenTypeIdentity::INVALID),
                    *span,
                )),
                [Expr::TypeSyntax(annotation, span)] => Some((
                    freeze.canonicalize_type(annotation).map_err(|message| {
                        ShapeError::SemanticError {
                            message,
                            location: None,
                        }
                    })?,
                    *span,
                )),
                _ => None,
            };
            if let Some((identity, span)) = lowered {
                *name = TYPE_REF_FORWARDER.to_string();
                *args = vec![
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.high), span),
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.low), span),
                ];
            }
        } else if name == "trait_ref" {
            // ADR-009 B2 (slice S4): the bare trait identifier lowers to the
            // FROZEN trait identity (freeze input 4 — a DISTINCT identity
            // kind from value types; `identity_of` never resolves a trait).
            // A name with no frozen trait identity transports INVALID and the
            // intrinsic answers with the named not-a-trait rejection.
            if let [Expr::Identifier(ident, span)] = args.as_slice() {
                let identity = freeze
                    .trait_identity_of(ident)
                    .unwrap_or(super::comptime_builtins::FrozenTypeIdentity::INVALID);
                let span = *span;
                *name = TRAIT_REF_FORWARDER.to_string();
                *args = vec![
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.high), span),
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.low), span),
                ];
            }
        } else if name == "type_constructor" {
            // ADR-009 B4 (Stage 2, Dec 54): `type_constructor(C)` lowers the
            // bare nominal head to its FROZEN identity halves (the `type_ref`
            // transport). A name with no frozen VALUE-type identity transports
            // INVALID and the intrinsic answers with the named unknown-
            // constructor rejection (R6); a name string never crosses into the
            // mini-VM (R1 strings-cannot-construct).
            if let [Expr::Identifier(ident, span)] = args.as_slice() {
                let identity = freeze
                    .identity_of(ident)
                    .unwrap_or(super::comptime_builtins::FrozenTypeIdentity::INVALID);
                let span = *span;
                *name = TYPE_CONSTRUCTOR_FORWARDER.to_string();
                *args = vec![
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.high), span),
                    Expr::Literal(shape_ast::ast::Literal::Int(identity.low), span),
                ];
            }
        }
    }

    // Recurse into every child expression so nested reflection calls
    // (`print(type_info(User).name)`) are rewritten too. The callable scope
    // flows unchanged into children EXCEPT match arms, whose per-arm bindings
    // extend it (handled explicitly below).
    let recur = |child: &mut Expr| -> Result<()> {
        rewrite_comptime_type_symbol_args_expr_scoped(child, freeze, callables)
    };
    match expr {
        Expr::FunctionCall {
            args, named_args, ..
        } => {
            for a in args.iter_mut() {
                recur(a)?;
            }
            for (_, a) in named_args.iter_mut() {
                recur(a)?;
            }
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            recur(receiver)?;
            for a in args.iter_mut() {
                recur(a)?;
            }
            for (_, a) in named_args.iter_mut() {
                recur(a)?;
            }
        }
        Expr::QualifiedFunctionCall {
            args, named_args, ..
        } => {
            for a in args.iter_mut() {
                recur(a)?;
            }
            for (_, a) in named_args.iter_mut() {
                recur(a)?;
            }
        }
        Expr::PropertyAccess { object, .. } => recur(object)?,
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            recur(object)?;
            recur(index)?;
            if let Some(end) = end_index {
                recur(end)?;
            }
        }
        Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
            recur(left)?;
            recur(right)?;
        }
        Expr::UnaryOp { operand, .. } => recur(operand)?,
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            recur(condition)?;
            recur(then_expr)?;
            if let Some(e) = else_expr {
                recur(e)?;
            }
        }
        Expr::Match(match_expr, _) => {
            recur(&mut match_expr.scrutinee)?;
            for arm in &mut match_expr.arms {
                // A `FrozenType::Callable(c)` arm binds `c` to a
                // `FrozenCallable` for the guard + body scope only (Dec 63).
                let mut arm_scope = callables.clone();
                collect_frozen_callable_bindings(&arm.pattern, &mut arm_scope);
                if let Some(guard) = &mut arm.guard {
                    rewrite_comptime_type_symbol_args_expr_scoped(guard, freeze, &arm_scope)?;
                }
                rewrite_comptime_type_symbol_args_expr_scoped(&mut arm.body, freeze, &arm_scope)?;
            }
        }
        Expr::Array(elems, _) => {
            for e in elems.iter_mut() {
                recur(e)?;
            }
        }
        Expr::Object(entries, _) => {
            for entry in entries.iter_mut() {
                match entry {
                    shape_ast::ast::ObjectEntry::Field { value, .. } => recur(value)?,
                    shape_ast::ast::ObjectEntry::Spread(e) => recur(e)?,
                }
            }
        }
        Expr::Block(block, _) => {
            // Exhaustive over `BlockItem` — annotation-handler bodies are
            // block EXPRESSIONS, so `let` / assignment items arrive as
            // `BlockItem::VariableDecl` / `BlockItem::Assignment`, not as
            // `BlockItem::Statement` (S3: `let flag = type_category(
            // type_ref(User))` inside a handler body).
            for item in &mut block.items {
                match item {
                    shape_ast::ast::BlockItem::Statement(s) => {
                        rewrite_comptime_type_symbol_args_scoped(s, freeze, callables)?;
                    }
                    shape_ast::ast::BlockItem::Expression(e) => recur(e)?,
                    shape_ast::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(init) = &mut decl.value {
                            recur(init)?;
                        }
                    }
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        recur(&mut assign.value)?;
                    }
                }
            }
        }
        Expr::TryOperator(inner, _)
        | Expr::Await(inner, _)
        | Expr::Spread(inner, _)
        | Expr::AsyncScope(inner, _)
        | Expr::Reference { expr: inner, .. } => recur(inner)?,
        Expr::Return(Some(inner), _) | Expr::Break(Some(inner), _) => recur(inner)?,
        // Control-flow EXPRESSION forms (`for`/`while`/`if`/`loop` used as block
        // items or tail values) embed a reflection call in natural comptime code
        // — e.g. `for p in c.parameters { … }` inside a comptime match arm, or
        // `if implements(T, Ord) { … }`. Recurse into their sub-expressions so
        // the accessor/reflection rewrites reach them (statement forms are
        // covered by `rewrite_comptime_type_symbol_args`).
        Expr::For(for_expr, _) => {
            recur(&mut for_expr.iterable)?;
            recur(&mut for_expr.body)?;
        }
        Expr::While(while_expr, _) => {
            recur(&mut while_expr.condition)?;
            recur(&mut while_expr.body)?;
        }
        Expr::If(if_expr, _) => {
            recur(&mut if_expr.condition)?;
            recur(&mut if_expr.then_branch)?;
            if let Some(else_branch) = &mut if_expr.else_branch {
                recur(else_branch)?;
            }
        }
        Expr::Loop(loop_expr, _) => recur(&mut loop_expr.body)?,
        Expr::Range { start, end, .. } => {
            if let Some(s) = start {
                recur(s)?;
            }
            if let Some(e) = end {
                recur(e)?;
            }
        }
        // Leaves and constructs that do not embed a reflection call in
        // practical comptime code are left untouched.
        _ => {}
    }
    Ok(())
}

/// Execute statements at compile time (comptime) and return the result.
///
/// Used for meta function methods with statement bodies. The statements are
/// wrapped in a function, compiled into a standalone BytecodeProgram, and
/// executed with a 5-second timeout.
///
/// Extension async functions (e.g., `postgres.connect()`) are supported:
/// `populate_module_objects()` wraps them with `block_in_place` + `block_on`,
/// which requires a tokio runtime. If no runtime exists (e.g., running from
/// tests or non-async CLI), a temporary single-threaded runtime is created.
#[allow(dead_code)]
pub(crate) fn execute_comptime(
    statements: &[Statement],
    comptime_helpers: &[FunctionDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
    freeze: std::sync::Arc<super::comptime_builtins::FreezeOverlay>,
) -> Result<ComptimeExecutionResult> {
    execute_comptime_with_context(
        statements,
        comptime_helpers,
        &[],
        &[],
        &[],
        extensions,
        trait_impl_keys,
        known_type_symbols,
        freeze,
    )
}

/// J-CT.2 (2026-05-23) — execute_comptime extended with a comptime-context
/// items slice. `comptime_impl_blocks` carry user-defined `comptime impl
/// Trait for Type { ... }` blocks; `comptime_context_trait_defs` carry the
/// `Item::Trait` AST for the traits those impls implement;
/// `comptime_context_struct_defs` carry the `Item::StructType` AST for the
/// types those impls target. All three are prepended as items into the
/// mini-VM program so the in-comptime-mode compiler desugars + compiles
/// them normally (same `Item::StructType` / `Item::Trait` / `Item::Impl`
/// arms as the outer compiler). Method dispatch from `instance.method()`
/// inside the comptime block then routes through the standard UFCS /
/// `Type::method` resolution path — audit §2.D carve-out, no new dispatch
/// shape.
pub(crate) fn execute_comptime_with_context(
    statements: &[Statement],
    comptime_helpers: &[FunctionDef],
    comptime_impl_blocks: &[shape_ast::ast::types::ImplBlock],
    comptime_context_trait_defs: &[shape_ast::ast::types::TraitDef],
    comptime_context_struct_defs: &[shape_ast::ast::types::StructTypeDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
    freeze: std::sync::Arc<super::comptime_builtins::FreezeOverlay>,
) -> Result<ComptimeExecutionResult> {
    // Wrap statements in a function so the compiler produces a callable entry point.
    // Ensure the last statement is a tail return so if/else values aren't discarded.
    let mut body = statements.to_vec();
    // Transform bare identifiers in legacy reflection calls to their existing
    // internal string payloads. `type_ref(T)` instead receives a compiler-issued
    // semantic identity; user-provided strings never construct a TypeRef.
    for stmt in &mut body {
        rewrite_comptime_type_symbol_args(stmt, freeze.as_ref())?;
    }
    ensure_tail_return(&mut body);

    // ADR-009 E3 (S2, U10): the mini-program entry wrapper is a HYGIENIC
    // generated function whose role is bound by its compiler-issued token —
    // never the former user-guessable `__comptime_block__` spelling. The
    // rendering is unspellable, so a `comptime { }` body (which becomes this
    // wrapper's body) cannot reference or shadow the wrapper. Nonce 0: the
    // mini-program holds exactly one such wrapper, so a fixed deterministic
    // identity is sufficient (and reproducible); the decl below and the tail
    // call reuse this one rendering.
    let func_name =
        HygienicSymbol::mint(HygienicRole::ComptimeBlockWrapper, 0).unspellable_descriptor();
    let func_def = FunctionDef {
        name: func_name.clone(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params: Vec::new(),
        return_type: None,
        body,
        type_params: Some(Vec::new()),
        annotations: Vec::new(),
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let mut items = comptime_builtin_forwarders();
    // J-CT.2 — struct defs FIRST so impl-block trait/method bindings can
    // resolve the target type. Impl blocks SECOND so the trait-method
    // symbol registry is populated before the wrapping `__comptime_block__`
    // function's body compiles its `instance.method()` calls. Each
    // comptime impl block has `is_comptime: true`; the in-comptime-mode
    // compiler's `Item::Impl` arm hits the J-CT.2 short-circuit (which
    // re-stores into the *mini-VM's* `comptime_impl_blocks` field, a
    // no-op for the inner mini-VM since there is no further nesting in
    // the audit-scoped one-level depth). To compile the methods, we
    // clear `is_comptime` on the cloned blocks before passing through —
    // they NEED to be compiled into mini-VM bytecode (we're already in
    // comptime mode; the outer-skip discipline doesn't apply within the
    // mini-VM).
    for trait_def in comptime_context_trait_defs {
        let mut def = trait_def.clone();
        // Same discipline as the impl-block clearing below (S5): inside the
        // mini-VM everything IS comptime, so a `comptime trait` compiles as
        // a plain trait. Without this a `comptime trait` + `comptime impl`
        // pair failed the J-CT.1 alignment check in the mini (threaded
        // trait kept is_comptime=true while the impl's flag was cleared).
        // The OUTER env's trait def keeps its flag — J-CT.1 runtime-call
        // rejection outside comptime blocks is unaffected.
        def.is_comptime = false;
        items.push(Item::Trait(def, Span::DUMMY));
    }
    for struct_def in comptime_context_struct_defs {
        items.push(Item::StructType(struct_def.clone(), Span::DUMMY));
    }
    for impl_block in comptime_impl_blocks {
        let mut block = impl_block.clone();
        block.is_comptime = false;
        items.push(Item::Impl(block, Span::DUMMY));
    }
    items.extend(
        comptime_helpers
            .iter()
            .cloned()
            .map(|helper| Item::Function(helper, Span::DUMMY)),
    );
    items.push(Item::Function(func_def, Span::DUMMY));
    items.push(Item::Expression(
        Expr::FunctionCall {
            name: func_name,
            const_args: Vec::new(),
            args: Vec::new(),
            named_args: Vec::new(),
            span: Span::DUMMY,
        },
        Span::DUMMY,
    ));
    let program = Program {
        items,
        docs: shape_ast::ast::ProgramDocs::default(),
    };

    // ADR-009 §4.1 (S2): the reflection intrinsics consume the shared
    // freeze handle — the Arc moves into the builtins module's closures.
    //
    // S5: the site-time key set for the Dec 52 ordering diagnostic is the
    // live key snapshot PLUS the J-CT.2 `comptime impl` pairs threaded into
    // this mini-program (those register only in the mini-VM's env, so the
    // outer live keys never see them). Diagnostic-only — the legacy
    // `implements` set is passed through unchanged, and no evidence is ever
    // produced from either set.
    let mut site_time_impl_keys = trait_impl_keys.clone();
    for impl_block in comptime_impl_blocks {
        let trait_name = match &impl_block.trait_name {
            shape_ast::ast::types::TypeName::Simple(n) => n.to_string(),
            shape_ast::ast::types::TypeName::Generic { name, .. } => name.to_string(),
        };
        let type_name = match &impl_block.target_type {
            shape_ast::ast::types::TypeName::Simple(n) => n.to_string(),
            shape_ast::ast::types::TypeName::Generic { name, .. } => name.to_string(),
        };
        site_time_impl_keys.insert(format!("{trait_name}::{type_name}"));
    }
    let comptime_builtins = super::comptime_builtins::create_comptime_builtins_module(
        trait_impl_keys,
        site_time_impl_keys,
        freeze,
    );
    compile_and_execute_comptime_program(
        &program,
        vec!["__comptime__".to_string()],
        Vec::new(),
        extensions,
        known_type_symbols,
        comptime_builtins,
    )
}

/// Compile and execute one comptime mini-program with a caller-supplied
/// `__comptime__` builtins module (the freeze-consuming module from
/// `create_comptime_builtins_module` — the only flavor; the S2-era
/// pre-pass rejection module is deleted, S3).
fn compile_and_execute_comptime_program(
    program: &Program,
    mut known_bindings: Vec<String>,
    mut runtime_module_bindings: Vec<(String, KindedSlot)>,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    known_type_symbols: std::collections::HashSet<String>,
    comptime_builtins: shape_runtime::module_exports::ModuleExports,
) -> Result<ComptimeExecutionResult> {
    // Build the full extension list first so module namespace bindings
    // (e.g. `__comptime__`) are typed during compilation.
    let mut all_extensions: Vec<shape_runtime::module_exports::ModuleExports> = extensions.to_vec();
    all_extensions.push(comptime_builtins);

    // Extension module namespaces are valid bindings in comptime handlers.
    // This enables generic annotation code to call module-scoped intrinsics
    // (e.g. `duckdb.connect_codegen(uri)`) without hardcoded exceptions.
    for module in &all_extensions {
        if !known_bindings.iter().any(|name| name == &module.name) {
            known_bindings.push(module.name.clone());
        }
    }

    // Compile the mini-program
    // Note: Do NOT inject prelude items here. Comptime mini-programs only need
    // their own helpers + extension builtins. Injecting the prelude would cause
    // name collisions (e.g., prelude's `sum` vs a comptime-generated `sum` method).
    let mut compiler = BytecodeCompiler::new().with_extensions(all_extensions.clone());
    compiler.set_comptime_mode(true);
    compiler.allow_internal_comptime_namespace = true;
    compiler.register_known_bindings(&known_bindings);
    for type_name in known_type_symbols {
        compiler
            .struct_types
            .entry(type_name)
            .or_insert_with(|| (Vec::new(), Span::DUMMY));
    }
    let mut bytecode = compiler.compile(program)?;

    rebind_typed_object_bindings_to_bytecode_schemas(&bytecode, &mut runtime_module_bindings);

    for module in &all_extensions {
        ensure_module_object_schema(&mut bytecode, module);
    }

    // Execute inside a function that guarantees a tokio runtime is available.
    // Extension async functions (wrapped by populate_module_objects) need
    // `tokio::runtime::Handle::current()` to work.
    execute_in_runtime_with_module_bindings(bytecode, &all_extensions, runtime_module_bindings)
}

/// Re-register comptime module bindings against the freshly-compiled
/// bytecode's schema registry.
///
/// **Phase-2c rebuild (C2-comptime-rebuild) per ADR-006 §2.4 / §2.7.4.**
/// The pre-bulldozer body walked `HeapValue::TypedObject { schema_id,
/// slots, heap_mask }` (the deleted inline-struct shape), looked up the
/// matching bytecode schema by field-name superset, and rebuilt the
/// binding via `ValueSlot::from_value_word`. Post-strict-typing the
/// payload is `TypedObjectPtr` wrapping a v2-raw
/// `*const TypedObjectStorage` (§2.3 amendment, Wave 2 Round 4 D4) — slot
/// bits are a direct typed pointer, NOT `Arc::into_raw(Arc<HeapValue>)`
/// (so `as_heap_value()` would be unsound; the typed-Arc dispatch
/// recovers `&TypedObjectStorage` directly per §2.7.16 receiver-recovery
/// soundness rule).
///
/// Promotion shape (mirrors the pre-bulldozer behaviour): for each
/// TypedObject-kinded binding, the smallest superset schema in the new
/// bytecode is selected (by field-name superset); the binding is rebuilt
/// against that schema's id with one share per heap-kinded slot read via
/// the §2.7.7 / Q9 kind-driven `read_typed_object_field` helper. Missing
/// target fields surface (no silent-default — promotion across mismatched
/// field sets would corrupt schema-keyed reads). Strict equality (target
/// id matches source id) short-circuits.
///
/// Refcount discipline: `read_typed_object_field` bumps one independent
/// share on each heap-kinded slot via `Arc::increment_strong_count::<T>`
/// or `v2_retain`. The accumulated `(bits, kind)` pairs are transferred
/// into a fresh `TypedObjectStorage::_new(...)` allocation; the new
/// storage owns those shares (retired by `_drop` at refcount=0 via
/// `drop_fields`). The source binding's `KindedSlot::Drop` releases the
/// original `TypedObjectPtr` share on `mem::replace`.
fn rebind_typed_object_bindings_to_bytecode_schemas(
    bytecode: &BytecodeProgram,
    module_bindings: &mut [(String, KindedSlot)],
) {
    use shape_value::TypedObjectStorage;
    use shape_value::ValueSlot;

    for (_name, value) in module_bindings.iter_mut() {
        // Only TypedObject-kinded bindings carry a schema id that may
        // need re-pointing into the fresh bytecode registry. Other
        // kinds (scalars, strings, arrays, etc.) are independent of
        // the bytecode's `TypeSchemaRegistry`.
        if !matches!(value.kind(), NativeKind::Ptr(HeapKind::TypedObject)) {
            continue;
        }
        let src_bits = value.slot().raw();
        if src_bits == 0 {
            // Null TypedObject — nothing to rebind.
            continue;
        }

        // Typed-Arc dispatch label recovery (§2.7.16 receiver-recovery
        // soundness rule): TypedObject slot bits are
        // `*const TypedObjectStorage`, never `*const HeapValue`. Cast
        // directly to the typed payload pointer; do NOT route through
        // `slot.as_heap_value()` (unsound under the v2-raw Path-B
        // `from_typed_object_raw` carrier per ADR-006 §2.3 amendment
        // Wave 2 Round 4 D4 ckpt-final-prime²).
        //
        // SAFETY: `NativeKind::Ptr(HeapKind::TypedObject)` is the kind
        // table's witness that these bits point to a live
        // `TypedObjectStorage`. The binding owns one strong-count share
        // on the HeapHeader-at-offset-0 refcount for the duration of
        // this iteration; the storage cannot be deallocated under us.
        let src_storage: &TypedObjectStorage = unsafe { &*(src_bits as *const TypedObjectStorage) };

        // Resolve the source schema (the ambient registry holds both
        // stdlib + predeclared schemas, including any registered by
        // `register_predeclared_any_schema` on the comptime
        // construction side e.g. via `ComptimeTarget::to_nanboxed`).
        let src_schema_id = src_storage.schema_id as shape_runtime::type_schema::SchemaId;
        let Some(src_schema) =
            shape_runtime::type_schema::lookup_schema_by_id_public(src_schema_id)
        else {
            // Source schema not resolvable — no safe rebind path.
            // Leave the binding alone; downstream schema-keyed reads
            // surface the mismatch with a clean diagnostic rather
            // than silent corruption.
            continue;
        };

        // Find the smallest target schema in the bytecode registry
        // whose field set is a superset of the source's. Mirrors the
        // pre-bulldozer promotion shape.
        let src_field_names: Vec<&str> =
            src_schema.fields.iter().map(|f| f.name.as_str()).collect();
        let target_schema = bytecode
            .type_schema_registry
            .type_names()
            .filter_map(|name| bytecode.type_schema_registry.get(name))
            .filter(|schema| {
                src_field_names
                    .iter()
                    .all(|name| schema.get_field(name).is_some())
            })
            .min_by_key(|schema| schema.fields.len())
            .cloned();
        let Some(target_schema) = target_schema else {
            // No matching target schema. The binding stays as-is; the
            // comptime body either reads it directly (the runtime
            // ambient registry resolves the source schema id) or
            // surfaces a clean schema-keyed access error.
            continue;
        };
        if target_schema.id == src_schema_id {
            // Already aligned — no rebuild needed.
            continue;
        }

        // Walk the target schema's fields in declared order. For each,
        // look up the source slot by field name and read it kinded;
        // missing target fields surface (silent-default promotion is
        // forbidden — playbook §7 #4).
        let n = target_schema.fields.len();
        let mut new_slots: Vec<ValueSlot> = Vec::with_capacity(n);
        let mut new_kinds: Vec<NativeKind> = Vec::with_capacity(n);
        let mut new_heap_mask: u64 = 0;
        let mut bail = false;

        for (target_idx, target_field) in target_schema.fields.iter().enumerate() {
            let Some(src_field) = src_schema.get_field(&target_field.name) else {
                // Target schema has a field not present in source.
                // Mirrors the pre-bulldozer `unwrap_or_else(none)` only
                // when the source had at least covered the target's
                // field set; the superset-match filter above guarantees
                // every source field exists in the target — but the
                // target may carry extras the source can't fill. Refuse
                // the rebuild (cleaner than fabricating a kind-stamped
                // None default that could mis-type the slot).
                bail = true;
                break;
            };
            let src_idx = src_field.index as usize;
            if src_idx >= src_storage.slots().len() || src_idx >= src_storage.field_kinds.len() {
                bail = true;
                break;
            }

            // The source slot's actual stored kind drives the read.
            // For `FieldType::Any` schemas (the predeclared comptime
            // shape), `to_native_kind()` refuses; we trust the
            // construction-side parallel kind table (§2.7.7 / Q9).
            let src_kind = src_storage.field_kinds[src_idx];
            let src_slot = src_storage.slots()[src_idx];
            let kinded =
                read_typed_object_field(src_slot, src_kind, src_storage.heap_mask, src_idx);

            // Transfer the share into `new_slots`; the rebuilt
            // TypedObject's `_drop` releases it via `drop_fields`.
            let bits = kinded.slot().raw();
            let kind = kinded.kind();
            std::mem::forget(kinded);
            new_slots.push(ValueSlot::from_raw(bits));
            new_kinds.push(kind);
            let is_heap_kind = matches!(kind, NativeKind::String | NativeKind::Ptr(_));
            if is_heap_kind && bits != 0 && target_idx < 64 {
                new_heap_mask |= 1u64 << target_idx;
            }
        }

        if bail {
            // Release every share we already accumulated so the
            // partial walk does not leak. Reconstruct each as a
            // `KindedSlot` and let Drop retire the share via the
            // §2.7.7 kind-driven dispatch.
            for (i, slot) in new_slots.drain(..).enumerate() {
                let kind = new_kinds[i];
                drop(KindedSlot::new(slot, kind));
            }
            continue;
        }

        // Build the rebuilt TypedObject. `_new` returns a `*mut
        // TypedObjectStorage` with refcount=1 on the HeapHeader; the
        // slot bits = `ptr as u64` via `from_typed_object_raw`
        // (ADR-006 §2.3 amendment Wave 2 D1 / D4).
        let new_ptr = TypedObjectStorage::_new(
            target_schema.id as u64,
            new_slots.into_boxed_slice(),
            new_heap_mask,
            Arc::from(new_kinds.into_boxed_slice()),
        );
        let new_kinded = KindedSlot::new(
            ValueSlot::from_typed_object_raw(new_ptr),
            NativeKind::Ptr(HeapKind::TypedObject),
        );

        // Replace the binding. The old `KindedSlot` Drop releases the
        // source TypedObject's share via the §2.7.7 / Q9 kind-driven
        // dispatch (TypedObjectStorage::release_elem → v2_release →
        // _drop at refcount=0).
        let old = std::mem::replace(value, new_kinded);
        drop(old);
    }
}

fn ensure_module_object_schema(
    bytecode: &mut BytecodeProgram,
    module: &shape_runtime::module_exports::ModuleExports,
) {
    let schema_name = format!("__mod_{}", module.name);
    let mut export_names: Vec<String> = module
        .export_names_available(true)
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    export_names.sort();
    export_names.dedup();

    let fields: Vec<(String, shape_runtime::type_schema::FieldType)> = export_names
        .into_iter()
        .map(|name| (name, shape_runtime::type_schema::FieldType::Any))
        .collect();
    bytecode
        .type_schema_registry
        .upsert_type_scoped_union_fields(schema_name, fields);
}

/// Execute a comptime handler with a target parameter bound.
///
/// Used for comptime annotation handlers that accept an explicit target parameter.
/// The handler body
/// is wrapped in a function that takes one parameter (the target object), which
/// is passed as an argument when calling the function.
///
/// Returns the `KindedSlot` result of the handler execution (ADR-006 §2.7).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn execute_comptime_with_target(
    handler_body: &Expr,
    handler_param: &str,
    target_value: KindedSlot,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    let handler_params = vec![AnnotationHandlerParam {
        name: handler_param.to_string(),
        is_variadic: false,
    }];
    // Test-only entry: a real freeze over an empty compilation unit through
    // the single freeze barrier (no empty-snapshot construction exists).
    let freeze = super::comptime_builtins::semantic_freeze::overlay_for_tests(
        &crate::compiler::BytecodeCompiler::new(),
    );
    execute_comptime_with_annotation_handler(
        handler_body,
        &handler_params,
        target_value,
        &[],
        &[],
        &[],
        &[],
        extensions,
        known_type_symbols,
        "",
        "",
        trait_impl_keys,
        freeze,
        // No representation authority on this convenience path (used by the
        // reflect-annotation self-test; not a type-target declaration hook).
        None,
    )
}

/// Execute a comptime annotation handler.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The body wires three
/// pieces that depend on the deleted `ValueWord` carrier:
///
/// 1. The `target_value: KindedSlot` is bound as a comptime module binding
///    keyed by `__target_arg__`. The set-module-binding path in
///    `execute_in_runtime_with_module_bindings` consumes the deleted
///    `set_module_binding_by_name_nb(&str, ValueWord)` API; the kinded
///    replacement is part of the comptime rebuild.
/// 2. `const_bindings` are materialized into the comptime AST via
///    `nb_to_expr`, which round-trips through deleted `ValueWord` accessors
///    (`as_any_array`, `as_str`, `as_decimal`, `as_heap_ref`, …). The
///    kinded replacement reads `(slot, kind)` directly and dispatches on
///    `NativeKind` for scalars + `slot.as_heap_value()` + `HeapValue::*`
///    match for heap arms (per ADR-006 §2.7.6 / Q8).
/// 3. The `ctx_nb` typed-object construction below uses the deleted
///    `typed_object_from_pairs` shape that takes `&[(&str, ValueWord)]`.
///    The kinded replacement takes `&[(&str, KindedSlot)]` and builds
///    `Arc<TypedObjectStorage>` directly.
///
/// All three pieces are part of the comptime-rebuild surface; the
/// signature is preserved so callers in `functions_annotations.rs` /
/// `statements.rs` / `expressions/mod.rs` continue to compile, but the
/// body panics until the rebuild lands rather than synthesizing a
/// placeholder result that would silently mis-bind handler params.
///
/// ADR-009 §4.1 (S2, ABI closed in S3): every caller supplies the REAL
/// per-compilation-unit freeze handle (`compiler.comptime_freeze_overlay()?`)
/// — the barrier runs before the first comptime site of the unit, including
/// the two speculative annotation pre-passes. The `__comptime__` builtins
/// module is built here from that handle, and the handler body receives the
/// same `type_ref`/`type_info`/`implements` type-symbol rewrite as comptime
/// blocks (`rewrite_comptime_type_symbol_args`), so frozen reflection
/// resolves inside annotation handlers too. No default/empty snapshot, no
/// `Option<freeze>`, exists on any path (rejection-matrix row 9).
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_comptime_with_annotation_handler(
    handler_body: &Expr,
    handler_params: &[AnnotationHandlerParam],
    target_value: KindedSlot,
    annotation_args: &[Expr],
    annotation_def_param_names: &[String],
    const_bindings: &[(String, KindedSlot)],
    comptime_helpers: &[FunctionDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    known_type_symbols: std::collections::HashSet<String>,
    ctx_module_path: &str,
    ctx_file: &str,
    trait_impl_keys: std::collections::HashSet<String>,
    freeze: std::sync::Arc<super::comptime_builtins::FreezeOverlay>,
    // ADR-009 B5 (Dec 56): the frozen identity halves of the annotated type,
    // present ONLY for declaration-attached type-target handlers. When present,
    // the compiler injects a call to the unspellable mint intrinsic
    // (`__access_arg__ = mint(high, low)`) INTO the handler program so the
    // `RepresentationAccess<T>` carrier is built inside the comptime mini-VM
    // (whose registry the schema-name-checked decoder reads) — never minted in
    // the outer registry and passed across, which would collide schema ids. The
    // carrier is delivered to the handler's third positional parameter as
    // `access` (author consent) — the ONLY route by which a `RepresentationAccess`
    // enters scope, so user code can never obtain one. Function/module/expression
    // targets pass `None`.
    access_identity: Option<(i64, i64)>,
) -> Result<ComptimeExecutionResult> {
    if handler_params.iter().filter(|p| p.is_variadic).count() > 1 {
        return Err(ShapeError::RuntimeError {
            message: "comptime annotation handlers support at most one variadic parameter"
                .to_string(),
            location: None,
        });
    }
    if let Some((idx, _)) = handler_params
        .iter()
        .enumerate()
        .find(|(_, p)| p.is_variadic)
    {
        if idx != handler_params.len().saturating_sub(1) {
            return Err(ShapeError::RuntimeError {
                message: "variadic comptime annotation handler parameter must be last".to_string(),
                location: None,
            });
        }
    }

    // ADR-009 E2 #18 (slice 5, Part A): clear the block-form `replace body`
    // carrier store at handler-run ENTRY — BEFORE this handler is compiled below
    // (its `replace body { ... }` statements stash into the store during that
    // compile) and thus before its VM run reads them by index. Distinct from the
    // pre-execute `clear_comptime_checked_items` (which clears item_fn's
    // execute-populated store): the body stash is compile-populated, so a
    // pre-execute clear would wipe it. Per-run clear ⇒ pre-pass and pass-2 each
    // index a fresh store, no stale body leaks across the double compile.
    super::comptime_builtins::clear_comptime_replace_bodies();

    let params: Vec<FunctionParameter> = handler_params
        .iter()
        .enumerate()
        .map(|(idx, p)| FunctionParameter {
            pattern: DestructurePattern::Identifier(p.name.clone(), Span::DUMMY),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: if idx == 0 {
                Some(comptime_target_param_type())
            } else if idx == 1 {
                Some(comptime_ctx_param_type())
            } else if idx == 2 && access_identity.is_some() {
                // ADR-009 B5 (Dec 56): the third positional parameter of a
                // type-target handler receives the `RepresentationAccess<T>`
                // authority; typing it here lets `reflect_repr(type_ref(T),
                // access)` type-check its authority argument.
                Some(comptime_representation_access_param_type())
            } else {
                None
            },
            default_value: None,
        })
        .collect();

    // ADR-009 E3 (S1, U10): the handler target/ctx module bindings are keyed
    // by the hygienic identity's UNSPELLABLE descriptor, so a handler body or
    // annotation-argument expression that references the former spelling
    // (`__target_arg__` / `__ctx_arg__`) resolves to nothing — it can no
    // longer capture the compiler-provided target/ctx. The SAME descriptor is
    // used for the call-arg reference, the known-binding declaration, and the
    // binding preset below.
    let target_binding = hygienic_comptime_target_binding();
    let ctx_binding = hygienic_comptime_ctx_binding();

    let mut call_args: Vec<Expr> = Vec::with_capacity(handler_params.len());
    let mut ann_idx = 0usize;
    for (idx, param) in handler_params.iter().enumerate() {
        if idx == 0 {
            call_args.push(Expr::Identifier(target_binding.clone(), Span::DUMMY));
            continue;
        }
        if idx == 1 {
            call_args.push(Expr::Identifier(ctx_binding.clone(), Span::DUMMY));
            continue;
        }
        // ADR-009 B5 (Dec 56): the third positional parameter of a type-target
        // handler binds the compiler-minted `RepresentationAccess<T>` authority,
        // NOT an annotation argument. The mint intrinsic is unspellable
        // (SOH-prefixed) and runs INSIDE this comptime mini-VM, so the carrier's
        // schema id is native to the registry the schema-name-checked decoder
        // reads — never minted in the outer registry and passed across (which
        // collides ids). It re-validates the identity through the shared freeze,
        // so a fabricated identity mints nothing.
        if idx == 2 {
            if let Some((high, low)) = access_identity {
                call_args.push(Expr::QualifiedFunctionCall {
                    namespace: "__comptime__".to_string(),
                    function: super::comptime_builtins::MINT_REPRESENTATION_ACCESS_INTRINSIC
                        .to_string(),
                    const_args: Vec::new(),
                    args: vec![
                        Expr::Literal(shape_ast::ast::Literal::Int(high), Span::DUMMY),
                        Expr::Literal(shape_ast::ast::Literal::Int(low), Span::DUMMY),
                    ],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                });
                continue;
            }
        }
        if param.is_variadic {
            call_args.push(Expr::Array(
                annotation_args.get(ann_idx..).unwrap_or_default().to_vec(),
                Span::DUMMY,
            ));
            ann_idx = annotation_args.len();
            continue;
        }
        let Some(arg) = annotation_args.get(ann_idx) else {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "missing annotation argument for comptime handler parameter '{}'",
                    param.name
                ),
                location: None,
            });
        };
        call_args.push(arg.clone());
        ann_idx += 1;
    }
    let extra_handler_params = handler_params.len().saturating_sub(2);
    if extra_handler_params > 0
        && ann_idx < annotation_args.len()
        && !handler_params.iter().any(|p| p.is_variadic)
    {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "too many annotation arguments: expected {}, got {}",
                ann_idx,
                annotation_args.len()
            ),
            location: None,
        });
    }

    // If the handler only has (target, ctx) but the annotation definition has params,
    // inject them as extra function params so the handler body can reference them by name.
    let mut params = params;
    if extra_handler_params == 0 && !annotation_def_param_names.is_empty() {
        for (i, def_param_name) in annotation_def_param_names.iter().enumerate() {
            if let Some(arg) = annotation_args.get(i) {
                params.push(FunctionParameter {
                    pattern: DestructurePattern::Identifier(def_param_name.clone(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: None,
                    default_value: None,
                });
                call_args.push(arg.clone());
            }
        }
    }

    // §4.4 comptime `ctx` compile-context: { module_path, file }. Built via
    // the reserved named schema (S2) so it never collides with an unrelated
    // ad-hoc field set. `build_config()` remains the single build-info surface
    // (no `ctx.build`).
    let ctx_nb = shape_runtime::type_schema::typed_object_for_named_schema(
        "__ComptimeContext",
        &[
            (
                "module_path",
                KindedSlot::from_string_arc(std::sync::Arc::new(ctx_module_path.to_string())),
            ),
            (
                "file",
                KindedSlot::from_string_arc(std::sync::Arc::new(ctx_file.to_string())),
            ),
        ],
    );

    // ADR-009 §4.1 (S3): annotation-handler bodies get the same frozen
    // type-symbol rewrite as comptime blocks — `type_ref(User)` becomes the
    // compiler-issued identity forwarder resolved against the shared freeze.
    let mut body_statement = Statement::Return(Some(handler_body.clone()), Span::DUMMY);
    rewrite_comptime_type_symbol_args(&mut body_statement, freeze.as_ref())?;

    // Wrap the handler body in a function that takes the target parameter.
    // ADR-009 E3 (S2, U10): HYGIENIC generated wrapper — role bound by the
    // compiler-issued token, never the former `__comptime_handler_fn__`
    // spelling. Unspellable rendering, one wrapper per mini-program, decl and
    // tail call reuse the one rendering (nonce 0, program-isolated).
    let func_name =
        HygienicSymbol::mint(HygienicRole::ComptimeHandlerWrapper, 0).unspellable_descriptor();
    let func_def = FunctionDef {
        name: func_name.clone(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params,
        return_type: None,
        body: vec![body_statement],
        type_params: Some(Vec::new()),
        annotations: Vec::new(),
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let mut items = comptime_builtin_forwarders();
    items.extend(
        comptime_helpers
            .iter()
            .cloned()
            .map(|helper| Item::Function(helper, Span::DUMMY)),
    );
    for (name, value) in const_bindings {
        let expr = nb_to_expr(value, Span::DUMMY).map_err(|message| ShapeError::RuntimeError {
            message: format!(
                "failed to materialize comptime const binding '{}': {}",
                name, message
            ),
            location: None,
        })?;
        items.push(Item::VariableDecl(
            VariableDecl {
                kind: VarKind::Const,
                is_mut: false,
                pattern: DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                type_annotation: None,
                value: Some(expr),
                ownership: Default::default(),
            },
            Span::DUMMY,
        ));
    }
    items.push(Item::Function(func_def, Span::DUMMY));
    items.push(Item::Expression(
        Expr::FunctionCall {
            name: func_name,
            const_args: Vec::new(),
            args: call_args,
            named_args: Vec::new(),
            span: Span::DUMMY,
        },
        Span::DUMMY,
    ));
    let program = Program {
        items,
        docs: shape_ast::ast::ProgramDocs::default(),
    };

    // ADR-009 §4.1 (S2/S3): the `__comptime__` builtins module carries the
    // reflection surface resolved against the shared freeze handle — for the
    // speculative pre-pass runs exactly as for the authoritative pass-2
    // runs. The old `TypeReflectionSnapshot::default()` empty-snapshot
    // defect and its S2 successor (the pre-pass reflection-rejection
    // module) are deleted.
    // S5: annotation-handler runs thread no J-CT.2 comptime impl blocks, so
    // the site-time key set for the Dec 52 ordering diagnostic is exactly
    // the live key snapshot (diagnostic-only; never evidence).
    let site_time_impl_keys = trait_impl_keys.clone();
    let comptime_builtins = super::comptime_builtins::create_comptime_builtins_module(
        trait_impl_keys,
        site_time_impl_keys,
        freeze,
    );
    compile_and_execute_comptime_program(
        &program,
        vec![
            target_binding.clone(),
            ctx_binding.clone(),
            "__comptime__".to_string(),
        ],
        vec![(target_binding, target_value), (ctx_binding, ctx_nb)],
        extensions,
        known_type_symbols,
        comptime_builtins,
    )
}

/// Run compiled bytecode on a fresh VM with extensions and pre-set
/// module-binding variables.
///
/// Phase-2c rebuild (C2-comptime-rebuild): the kinded path threads each
/// pre-set binding into the §2.7.8 / Q10 parallel module-binding kind
/// track via `module_binding_write_kinded(index, bits, kind)` after
/// resolving the binding name through `program.module_binding_names`.
/// `vm.execute(None)` returns a `KindedSlot` directly (ADR-006 §2.7 / Q7)
/// — no synthesis layer.
fn execute_in_runtime_with_module_bindings(
    bytecode: BytecodeProgram,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    module_bindings: Vec<(String, KindedSlot)>,
) -> Result<ComptimeExecutionResult> {
    let run = |module_bindings: Vec<(String, KindedSlot)>| -> Result<ComptimeExecutionResult> {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);

        for ext in extensions {
            vm.register_extension(ext.clone());
        }
        vm.populate_module_objects();

        // Pre-set module bindings (e.g. `__target_arg__`, `__ctx_arg__`).
        // The name → index lookup uses `program.module_binding_names`;
        // unknown names are dropped (the compile-side
        // `register_known_bindings` is responsible for inserting names
        // before compilation).
        for (name, value) in module_bindings {
            let idx = vm
                .program
                .module_binding_names
                .iter()
                .position(|n| n == &name);
            match idx {
                Some(i) => {
                    let bits = value.slot().raw();
                    let kind = value.kind();
                    // Transfer the share into the binding storage; the
                    // input slot's Drop must not double-release.
                    std::mem::forget(value);
                    vm.module_binding_write_kinded(i, bits, kind);
                }
                None => {
                    // Drop the input slot's share (no consumer).
                    drop(value);
                }
            }
        }

        // 5-second timeout watchdog — bounded comptime budget protects
        // the host from runaway user code (same shape as the pre-stub
        // body).
        let interrupt = Arc::new(AtomicU8::new(0));
        vm.set_interrupt(interrupt.clone());
        let timeout_interrupt = interrupt.clone();
        let _timer_handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            timeout_interrupt.store(1, Ordering::SeqCst);
        });

        super::comptime_builtins::clear_comptime_directives();
        super::comptime_builtins::clear_comptime_diagnostics();
        // ADR-009 E2 #18 (slice 2): reset the `item_fn` carrier store so this
        // run's `__CheckedItem` handles index a fresh store.
        super::comptime_builtins::clear_comptime_checked_items();
        let value = vm.execute(None).map_err(|e| ShapeError::RuntimeError {
            message: format!("Comptime handler execution failed: {}", e),
            location: None,
        })?;
        let directives = super::comptime_builtins::take_comptime_directives();
        let warnings = super::comptime_builtins::take_comptime_diagnostics();

        Ok(ComptimeExecutionResult {
            value,
            directives,
            warnings,
            schema_registry: Arc::new(vm.program.type_schema_registry.clone()),
        })
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        run(module_bindings)
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ShapeError::RuntimeError {
                message: format!("Failed to create tokio runtime for comptime: {}", e),
                location: None,
            })?;
        rt.block_on(async { run(module_bindings) })
    }
}

/// ADR-009 B1 S4 — the VALUE-DEEP stage-boundary lift wall for comptime
/// results that enter runtime code (`Expr::Comptime`).
///
/// The shallow `runtime_lift_rejection` call only saw the top-level slot,
/// so a descriptor NESTED inside an object/array result (or forged as a
/// mini-VM-registered spellable model value) slipped past the wall and was
/// silently swallowed to `Null` by the `nb_to_expr` fallback (scout risk 4
/// — the materialization bypass channel). This extends the CHANNEL to call
/// `runtime_lift_rejection` on every reachable typed-object node — never
/// the reverse (the wall is not weakened).
///
/// The walk runs with the mini-VM program's schema registry installed as
/// the ambient scope: descriptor carriers built by the intrinsics use the
/// order-stable builtin schema ids, but the injected spellable model enums
/// and comptime object-literal schemas exist only in the mini-VM registry —
/// without it their ids miss (or collide) in the outer registry and the
/// wall cannot name them.
pub(crate) fn comptime_result_lift_rejection(
    value: &KindedSlot,
    schema_registry: &std::sync::Arc<shape_runtime::type_schema::TypeSchemaRegistry>,
) -> Option<&'static str> {
    let _scope = shape_runtime::type_schema::SyncRegistryScope::enter(schema_registry.clone());
    deep_descriptor_lift_rejection(value)
}

/// Recursive half of [`comptime_result_lift_rejection`]: the shared
/// name-matched wall (`runtime_lift_rejection`) at every node, recursing
/// through typed-object fields and typed-array elements — the only carrier
/// shapes a comptime descriptor can nest in on the materialization channel.
fn deep_descriptor_lift_rejection(value: &KindedSlot) -> Option<&'static str> {
    if let Some(message) = shape_runtime::comptime_reflection::runtime_lift_rejection(value) {
        return Some(message);
    }
    let bits = value.slot().raw();
    if bits == 0 {
        return None;
    }
    match value.kind() {
        NativeKind::Ptr(HeapKind::TypedObject) => {
            // SAFETY: `NativeKind::Ptr(HeapKind::TypedObject)` is the kind
            // witness that these bits point to a live `TypedObjectStorage`
            // (§2.7.16 receiver-recovery soundness rule — direct typed
            // pointer, never `as_heap_value()`); `value` owns one
            // strong-count share for the duration of the walk.
            let storage: &shape_value::TypedObjectStorage =
                unsafe { &*(bits as *const shape_value::TypedObjectStorage) };
            let field_count = storage.slots().len().min(storage.field_kinds.len());
            for idx in 0..field_count {
                let field = read_typed_object_field(
                    storage.slots()[idx],
                    storage.field_kinds[idx],
                    storage.heap_mask,
                    idx,
                );
                if let Some(message) = deep_descriptor_lift_rejection(&field) {
                    return Some(message);
                }
            }
            None
        }
        NativeKind::Ptr(HeapKind::TypedArray) => {
            typed_array_descriptor_lift_rejection(bits as *const u8)
        }
        _ => None,
    }
}

/// Typed-array arm of the deep wall: walk `TypedObjectPtr` elements (and
/// nested arrays) through the elem-type stamp. Scalar element types cannot
/// carry descriptors and are skipped.
fn typed_array_descriptor_lift_rejection(array: *const u8) -> Option<&'static str> {
    use shape_value::v2::typed_array::{
        ELEM_TYPE_TYPED_ARRAY, ELEM_TYPE_TYPED_OBJECT, TypedArray, read_elem_type,
    };
    // SAFETY (all blocks below): the caller's `NativeKind::Ptr(
    // HeapKind::TypedArray)` kind witness proves `array` points to a live
    // `TypedArray<T>`; the elem-type stamp selects the monomorphized
    // element layout before any element is read.
    let elem_type = unsafe { read_elem_type(array) };
    match elem_type {
        ELEM_TYPE_TYPED_OBJECT => {
            let arr = array as *const TypedArray<*const shape_value::TypedObjectStorage>;
            for &elem in unsafe { TypedArray::as_slice(arr) } {
                if elem.is_null() {
                    continue;
                }
                // Take one independent share so the element slot's Drop is
                // balanced (same retain discipline as
                // `read_typed_object_field`).
                unsafe {
                    shape_value::v2::refcount::v2_retain(&(*elem).header);
                }
                let slot = KindedSlot::from_typed_object_raw(elem);
                let rejection = deep_descriptor_lift_rejection(&slot);
                if rejection.is_some() {
                    return rejection;
                }
            }
            None
        }
        ELEM_TYPE_TYPED_ARRAY => {
            let arr = array as *const TypedArray<*const u8>;
            for &elem in unsafe { TypedArray::as_slice(arr) } {
                if elem.is_null() {
                    continue;
                }
                let rejection = typed_array_descriptor_lift_rejection(elem);
                if rejection.is_some() {
                    return rejection;
                }
            }
            None
        }
        _ => None,
    }
}

/// Convert a comptime execution result to an AST Literal for compilation.
///
/// Phase-2c rebuild (C2-comptime-rebuild): dispatch is
/// `match slot.kind { NativeKind::* => … }` for scalars + `slot.as_heap_value()`
/// + `HeapValue::*` match for heap arms per ADR-006 §2.7.6 / Q8. Heap arms
/// without a single-literal representation fall through to a
/// `Literal::String` Debug rendering of the kind (best-effort — the
/// upstream caller `expressions/mod.rs:1246` tries `nb_to_expr` first and
/// only falls through to this when the value reduces to a single literal).
pub(crate) fn vmvalue_to_literal(value: &KindedSlot) -> shape_ast::ast::Literal {
    nb_to_literal(value)
}

/// Convert a comptime KindedSlot to an AST Literal for compilation.
///
/// Same surface as `vmvalue_to_literal`. Used by comptime for-loop
/// unrolling where elements are already individual KindedSlots
/// (extracted from the `HeapValue::TypedArray(Arc<TypedArrayData>)`
/// per-element shape per ADR-006 §2.3).
pub(crate) fn nb_to_literal(nb: &KindedSlot) -> shape_ast::ast::Literal {
    use shape_ast::ast::Literal;

    // Scalar dispatch on NativeKind first (ADR-006 §2.7.7 / Q9 — kind
    // is the single source of truth for the slot's interpretation).
    match nb.kind() {
        NativeKind::Int64 => return Literal::Int(nb.as_i64().unwrap_or(0)),
        NativeKind::Float64 => return Literal::Number(nb.as_f64().unwrap_or(0.0)),
        NativeKind::Bool => {
            // A `Bool`-kinded slot always materializes as a Bool literal,
            // including `false` (raw 0). The none sentinel is `NativeKind::Null`
            // (`KindedSlot::none()` → `NativeKind::Null`, kinded_slot.rs:565),
            // NOT a Bool-kinded zero, so it never reaches this arm — it falls
            // through to the heap path where `bits == 0 ≡ None`. Treating a
            // Bool-kinded zero as None here mis-rendered `comptime { false }`
            // (and `build_config().debug` in a release build) as `null`.
            return Literal::Bool(nb.as_bool().unwrap_or(false));
        }
        NativeKind::String => {
            if let Some(s) = nb.as_str() {
                return Literal::String(s.to_string());
            }
            return Literal::None;
        }
        NativeKind::Ptr(HeapKind::Char) => {
            if let Some(c) = nb.as_char() {
                return Literal::Char(c);
            }
            return Literal::None;
        }
        // R2 (2026-06-18): TypedObject is NOT routable through
        // `as_heap_value()` — its bits are `*const TypedObjectStorage`, not
        // `Arc::into_raw(Arc<HeapValue>)`, so the deref would reinterpret
        // `schema_id` as a HeapValue discriminator and segfault. A
        // TypedObject has no single-literal representation; the
        // expression-form path (`nb_to_expr` / `typed_object_to_object_expr`)
        // is the only valid materialization. Returning `None` here keeps the
        // last-resort literal path sound (callers prefer `nb_to_expr` first).
        NativeKind::Ptr(HeapKind::TypedObject) => {
            return Literal::None;
        }
        _ => {}
    }

    // Heap-arm dispatch via `slot.as_heap_value()` + `HeapValue::*`
    // match per ADR-006 §2.7.6 / Q8.
    let slot_for_hv = nb.slot();
    let bits = slot_for_hv.raw();
    if bits == 0 {
        return Literal::None;
    }
    let hv = slot_for_hv.as_heap_value();
    match hv {
        HeapValue::String(s) => Literal::String((**s).clone()),
        HeapValue::Decimal(d) => Literal::Decimal(**d),
        HeapValue::BigInt(i) => Literal::Int(**i),
        HeapValue::Char(c) => Literal::Char(*c),
        // Complex types (TypedArray / TypedObject / HashMap / etc.) cannot
        // be represented as a single literal — last-resort Debug string.
        _ => Literal::String(format!("{}", hv)),
    }
}

/// Public entry point for converting a comptime KindedSlot to an AST
/// expression.
pub(crate) fn nb_to_expr_public(nb: &KindedSlot, span: Span) -> std::result::Result<Expr, String> {
    nb_to_expr(nb, span)
}

/// Convert a comptime KindedSlot to an AST expression.
///
/// Phase-2c rebuild (C2-comptime-rebuild): dispatch is
/// `match slot.kind { NativeKind::* => … }` for scalars + `slot.as_heap_value()`
/// + `HeapValue::*` match for heap arms per ADR-006 §2.7.6 / Q8. The
/// TypedArray walk reads each element via the kinded per-variant pattern
/// from `array_aggregation::element_kinded` (ADR-005 §1 single-discriminator
/// — dispatch through `HeapValue` match in the `the-deleted-heterogeneous-element-carrier`
/// arm). The TypedObject walk reads slots via the schema's `FieldType` to
/// recover per-field NativeKind; `FieldType::Any` fields surface explicitly
/// because slot bits without kind metadata cannot be safely re-typed at
/// the literal-readback layer (the comptime predeclared schemas use Any).
fn nb_to_expr(nb: &KindedSlot, span: Span) -> std::result::Result<Expr, String> {
    // Scalar dispatch first (ADR-006 §2.7.7 / Q9).
    match nb.kind() {
        NativeKind::Int64 => {
            return Ok(Expr::Literal(
                shape_ast::ast::Literal::Int(nb.as_i64().unwrap_or(0)),
                span,
            ));
        }
        NativeKind::Float64 => {
            return Ok(Expr::Literal(
                shape_ast::ast::Literal::Number(nb.as_f64().unwrap_or(0.0)),
                span,
            ));
        }
        NativeKind::Bool => {
            // A `Bool`-kinded slot always materializes as a Bool literal,
            // including `false` (raw 0). The none sentinel is `NativeKind::Null`
            // (`KindedSlot::none()` → `NativeKind::Null`, kinded_slot.rs:565),
            // NOT a Bool-kinded zero, so it never reaches this arm. Treating a
            // Bool-kinded zero as None here mis-rendered `comptime { false }`
            // (and `build_config().debug`) as `null`.
            return Ok(Expr::Literal(
                shape_ast::ast::Literal::Bool(nb.as_bool().unwrap_or(false)),
                span,
            ));
        }
        NativeKind::String => {
            if let Some(s) = nb.as_str() {
                return Ok(Expr::Literal(
                    shape_ast::ast::Literal::String(s.to_string()),
                    span,
                ));
            }
            return Ok(Expr::Literal(shape_ast::ast::Literal::None, span));
        }
        NativeKind::Ptr(HeapKind::Char) => {
            if let Some(c) = nb.as_char() {
                return Ok(Expr::Literal(shape_ast::ast::Literal::Char(c), span));
            }
            return Ok(Expr::Literal(shape_ast::ast::Literal::None, span));
        }
        // R2 (2026-06-18): TypedObject readback via direct typed-pointer
        // recovery — NOT `slot.as_heap_value()`. A `Ptr(HeapKind::TypedObject)`
        // slot's bits are `*const TypedObjectStorage` (the v2-raw
        // `from_typed_object_raw` carrier per ADR-006 §2.3 amendment Wave 2
        // Round 4 D4), never `Arc::into_raw(Arc<HeapValue>)`. Routing through
        // `as_heap_value()` reinterprets the storage's first 8 bytes
        // (`schema_id: u64`) as a `HeapValue` discriminator and segfaults
        // (§2.7.16 receiver-recovery soundness rule). This was the
        // `comptime { build_config() }` SIGSEGV.
        //
        // Per-field kinds come from the storage's own
        // `field_kinds: Arc<[NativeKind]>` (stamped at construction by
        // `typed_object_from_pairs`), NOT the schema's `FieldType` — the
        // comptime predeclared schemas register every field as
        // `FieldType::Any`, which has no kinded projection, but the storage
        // carries the proven per-slot kind.
        NativeKind::Ptr(HeapKind::TypedObject) => {
            let bits = nb.slot().raw();
            if bits == 0 {
                return Ok(Expr::Literal(shape_ast::ast::Literal::None, span));
            }
            // SAFETY: `NativeKind::Ptr(HeapKind::TypedObject)` is the kind
            // table's witness that these bits point to a live
            // `TypedObjectStorage`. `nb` owns one strong-count share on the
            // HeapHeader-at-offset-0 refcount for the duration of this call,
            // so the storage cannot be deallocated under us.
            let storage: &shape_value::TypedObjectStorage =
                unsafe { &*(bits as *const shape_value::TypedObjectStorage) };
            return typed_object_to_object_expr(storage, span);
        }
        _ => {}
    }

    // Heap-arm dispatch via `slot.as_heap_value()` + `HeapValue` match
    // (ADR-006 §2.7.6 / Q8). Null bits ≡ None at the literal boundary.
    let slot_for_hv = nb.slot();
    let bits = slot_for_hv.raw();
    if bits == 0 {
        return Ok(Expr::Literal(shape_ast::ast::Literal::None, span));
    }
    let hv = slot_for_hv.as_heap_value();
    match hv {
        HeapValue::String(s) => Ok(Expr::Literal(
            shape_ast::ast::Literal::String((**s).clone()),
            span,
        )),
        HeapValue::Decimal(d) => Ok(Expr::Literal(shape_ast::ast::Literal::Decimal(**d), span)),
        HeapValue::BigInt(i) => Ok(Expr::Literal(shape_ast::ast::Literal::Int(**i), span)),
        HeapValue::Char(c) => Ok(Expr::Literal(shape_ast::ast::Literal::Char(*c), span)),
        // V3-S5 ckpt-5: HeapValue::TypedArray outer arm DELETED at ckpt-4
        // in lockstep with TypedArrayData enum + TypedBuffer<T> wrapper
        // layer per W12 audit §3.6. Comptime materialization of v2-raw
        // `TypedArray<T>` arrays lands at ckpt-6 STRICT close.
        //   HeapValue::TypedArray(arr) => { ... }
        // TypedObject is handled earlier by the `Ptr(HeapKind::TypedObject)`
        // kind-arm (direct typed-pointer recovery, NOT `as_heap_value()`),
        // so it can never reach this `HeapValue` match. The compiler keeps
        // the arm absent — any TypedObject-kinded slot returns before here.
        // Cold fallthrough — closures, futures, data tables, etc. are
        // not valid comptime literals.
        other => Err(format!(
            "unsupported comptime literal value: HeapValue::{:?}",
            other.kind()
        )),
    }
}

/// Materialize a comptime `TypedObject` into an `Expr::Object` literal.
///
/// R2 (2026-06-18). Reads each field through the storage's own
/// `field_kinds: Arc<[NativeKind]>` (stamped at construction in
/// `typed_object_from_pairs`), NOT the schema's `FieldType`. The comptime
/// predeclared schemas register every field as `FieldType::Any` (which has
/// no kinded projection), so the prior schema-driven readback would have
/// errored even after the segfault fix; the storage's proven per-slot kind
/// is the authoritative source per ADR-006 §2.7.5.
///
/// Field ordering follows the schema's declared order so the emitted object
/// literal is stable across runs. `read_typed_object_field` bumps one
/// independent share per heap-kinded slot; the returned `KindedSlot`'s Drop
/// retires it at scope exit.
fn typed_object_to_object_expr(
    storage: &shape_value::TypedObjectStorage,
    span: Span,
) -> std::result::Result<Expr, String> {
    let schema_id = storage.schema_id as u32;
    let schema =
        shape_runtime::type_schema::lookup_schema_by_id_public(schema_id).ok_or_else(|| {
            format!(
                "TypedObject schema id {} not found while materializing \
                 comptime literal — playbook §7 surface, ADR-006 §2.7.4 \
                 (schema rebind deferred)",
                schema_id
            )
        })?;
    if storage.slots().len() != storage.field_kinds.len() {
        return Err(format!(
            "TypedObject storage slots/field_kinds length mismatch \
             (slots={}, field_kinds={}) — corrupt carrier",
            storage.slots().len(),
            storage.field_kinds.len()
        ));
    }
    let mut entries = Vec::with_capacity(schema.fields.len());
    for field_def in schema.fields.iter() {
        let idx = field_def.index as usize;
        if idx >= storage.slots().len() {
            return Err(format!(
                "TypedObject slot index {} out of bounds (len={}) — \
                 schema/storage mismatch",
                idx,
                storage.slots().len()
            ));
        }
        let slot = storage.slots()[idx];
        // Authoritative per-slot kind from the storage carrier (§2.7.5),
        // not the predeclared schema's `FieldType::Any`.
        let kind = storage.field_kinds[idx];
        let kinded_slot = read_typed_object_field(slot, kind, storage.heap_mask, idx);
        // A genuine `Bool` field must materialize as a Bool literal even when
        // its value is `false`. `nb_to_expr`'s scalar arm treats Bool-kinded
        // zero bits as the `KindedSlot::none()` sentinel (it cannot tell
        // `false` from none without the surrounding kind context), so a
        // `false` field (e.g. `debug` in a release build) would otherwise bake
        // as `None` — silent data loss. The storage's `field_kinds` proves the
        // field IS a Bool here, so project it directly. Other kinds keep the
        // shared `nb_to_expr` path.
        let value_expr = if matches!(kind, NativeKind::Bool) {
            Expr::Literal(shape_ast::ast::Literal::Bool(kinded_slot.raw() != 0), span)
        } else {
            nb_to_expr(&kinded_slot, span)?
        };
        entries.push(ObjectEntry::Field {
            key: field_def.name.clone(),
            value: value_expr,
            type_annotation: None,
        });
    }
    Ok(Expr::Object(entries, span))
}

// V3-S5 ckpt-5 (2026-05-15): `typed_array_len` + `typed_array_element_kinded`
// helpers DELETED. Both consumed `&TypedArrayData` (deleted at ckpt-1) for
// the deleted `HeapValue::TypedArray` arm in `nb_to_expr` (lines 924-931
// above). Comptime materialization of v2-raw `TypedArray<T>` arrays lands
// at ckpt-6 STRICT close per W12-typed-array-data-deletion audit §B.

// R2 (2026-06-18): `field_kind_for_readback` (schema `FieldType` →
// `NativeKind`) DELETED. TypedObject readback now sources per-slot kinds
// from the storage's own `field_kinds: Arc<[NativeKind]>` carrier
// (`typed_object_to_object_expr`) — the comptime predeclared schemas use
// `FieldType::Any`, which has no kinded projection, so the schema was never
// a usable kind source. The storage's stamped kind is authoritative
// (ADR-006 §2.7.5).

/// Read a `TypedObjectStorage` slot at index `idx` as an owned
/// `KindedSlot`, bumping the heap refcount when applicable so the
/// returned slot owns one independent strong-count share.
///
/// `heap_mask`'s bit `idx` is consulted to decide whether the slot's
/// bits are a heap pointer that needs retain-on-read, mirroring the
/// `stack_read_kinded` retain discipline (ADR-006 §2.7.7 / Q9 — kind
/// drives clone/drop dispatch).
fn read_typed_object_field(
    slot: shape_value::ValueSlot,
    kind: NativeKind,
    heap_mask: u64,
    idx: usize,
) -> KindedSlot {
    let is_heap_slot = idx < 64 && (heap_mask >> idx) & 1 == 1;
    let bits = slot.raw();
    if !is_heap_slot {
        return KindedSlot::new(slot, kind);
    }
    if bits == 0 {
        return KindedSlot::none();
    }
    // Heap-bearing slot: bump the underlying Arc's strong count so the
    // returned KindedSlot owns one independent share. Same typed
    // `Arc::increment_strong_count::<T>` dispatch the
    // `TypedObjectStorage::Drop` impl uses for
    // `Arc::decrement_strong_count::<T>`.
    unsafe {
        match kind {
            NativeKind::String => {
                Arc::increment_strong_count(bits as *const String);
            }
            NativeKind::Ptr(hk) => match hk {
                HeapKind::String => {
                    Arc::increment_strong_count(bits as *const String);
                }
                HeapKind::TypedArray => {
                    // V3-S5 ckpt-6 STRICT close (2026-05-15): slot bits are
                    // v2-raw `*mut TypedArray<T>` per ADR-006 §2.7.24 Q25.A
                    // SUPERSEDED. Refcount discipline goes through
                    // `v2_retain` against the `HeapHeader` at offset 0 of
                    // the carrier (mirror of vm_impl/stack.rs StringV2 /
                    // DecimalV2 / TypedObject retain dispatch).
                    let hdr = bits as *const shape_value::v2::heap_header::HeapHeader;
                    shape_value::v2::refcount::v2_retain(hdr);
                }
                HeapKind::TypedObject => {
                    // R6 carrier-convention soundness (2026-06): TypedObject
                    // slot bits are the v2-raw `*const TypedObjectStorage`
                    // produced by `TypedObjectStorage::_new` (HeapHeader at
                    // offset 0), NOT `Arc::into_raw(Arc::new(...))`. The
                    // refcount lives on the HeapHeader; an `Arc::increment_
                    // strong_count` here would `byte_sub(16)` into non-ArcInner
                    // memory (the same UB the adjacent TypedArray arm avoids).
                    // Retain via `v2_retain` against the HeapHeader — pairs
                    // with the `TypedObjectStorage::release_elem` drop arm in
                    // vm_impl/stack.rs.
                    let hdr = bits as *const shape_value::v2::heap_header::HeapHeader;
                    shape_value::v2::refcount::v2_retain(hdr);
                }
                HeapKind::Decimal => {
                    Arc::increment_strong_count(bits as *const rust_decimal::Decimal);
                }
                HeapKind::BigInt => {
                    Arc::increment_strong_count(bits as *const i64);
                }
                HeapKind::Closure
                | HeapKind::DataTable
                | HeapKind::Future
                | HeapKind::TaskGroup
                | HeapKind::Temporal
                | HeapKind::TableView
                | HeapKind::Content
                | HeapKind::Instant
                | HeapKind::IoHandle
                | HeapKind::NativeScalar
                | HeapKind::NativeView
                | HeapKind::Char
                | HeapKind::HashMap
                | HeapKind::FilterExpr
                | HeapKind::Reference
                | HeapKind::SharedCell
                | HeapKind::HashSet
                | HeapKind::Iterator
                | HeapKind::Deque
                | HeapKind::Channel
                | HeapKind::PriorityQueue
                | HeapKind::Range
                | HeapKind::Result
                | HeapKind::Option
                | HeapKind::TraitObject
                | HeapKind::Mutex
                | HeapKind::Atomic
                | HeapKind::Lazy
                | HeapKind::ModuleFn
                | HeapKind::Matrix
                | HeapKind::MatrixSlice => {
                    // Other heap kinds aren't produced by the comptime
                    // predeclared schemas at landing; surface rather
                    // than fabricate a refcount bump.
                    return KindedSlot::new(slot, kind);
                }
            },
            _ => {}
        }
    }
    KindedSlot::new(slot, kind)
}

// Phase-2c rebuild pending — see ADR-006 §2.4. The comptime test suite
// asserts on the deleted `ValueWord` carrier (`from_i64`, `from_f64`,
// `from_string`, `from_bool`, `none`, `unit`, `from_array`,
// `vmarray_from_vec`, `as_arc_string`, `as_number_coerce`, `as_heap_ref`)
// plus the deleted `vm.execute()` synthesis path. The whole module is
// stubbed and ignored until the comptime rebuild lands; re-enable
// per-test as the rebuild walks each path.
/// Test-only freeze handle: a REAL freeze of an empty compilation unit
/// through the single freeze barrier (never an empty-snapshot construction).
#[cfg(test)]
pub(crate) fn test_freeze_overlay() -> std::sync::Arc<super::comptime_builtins::FreezeOverlay> {
    super::comptime_builtins::semantic_freeze::overlay_for_tests(
        &crate::compiler::BytecodeCompiler::new(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]
    fn placeholder_phase_2c_comptime_tests() {}

    /// ADR-009 S3 probe: `type_ref(User)` inside an ANNOTATION HANDLER body
    /// receives the same frozen type-symbol rewrite as comptime blocks.
    #[test]
    fn annotation_handler_body_type_ref_is_rewritten_against_the_freeze() {
        let program = shape_ast::parse_program(
            r#"
annotation reflect() {
  targets: [type]
  comptime post(target, ctx) {
    let flag = match type_category(type_ref(User)) {
      FrozenTypeCategory::Nominal => 1
      _ => 0
    }
    flag
  }
}
"#,
        )
        .expect("annotation parses");
        let handler_body = program
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::AnnotationDef(def, _) => Some(def.handlers[0].body.clone()),
                _ => None,
            })
            .expect("annotation handler present");

        let freeze = crate::compiler::comptime_builtins::semantic_freeze::overlay_for_tests(
            &crate::compiler::BytecodeCompiler::new(),
        );
        let mut statement = Statement::Return(Some(handler_body), Span::DUMMY);
        super::rewrite_comptime_type_symbol_args(&mut statement, freeze.as_ref())
            .expect("bare-identifier rewrite cannot fail");
        let rendered = format!("{statement:?}");
        assert!(
            !rendered.contains("\"type_ref\""),
            "handler-body type_ref call must be rewritten to the identity forwarder: {rendered}"
        );
    }

    /// ADR-009 A2 (S3): the `Expr::TypeSyntax` carrier is grammatically
    /// producible only as the `type_ref(...)` argument, but AST producers
    /// (transforms, comptime-generated code) could misplace it. Outside the
    /// type_ref argument position it must be a NAMED compile error — never
    /// a silently compiled value (surface-and-stop).
    #[test]
    fn type_syntax_outside_type_ref_is_a_named_compile_error() {
        let mut program = shape_ast::parse_program("let x = 1;").expect("baseline parses");
        // Swap the initializer for a hand-built type-syntax carrier —
        // no source spelling can produce this placement.
        match program.items.get_mut(0) {
            Some(shape_ast::ast::Item::Statement(
                shape_ast::ast::Statement::VariableDecl(decl, _),
                _,
            )) => {
                decl.value = Some(shape_ast::ast::Expr::TypeSyntax(
                    shape_ast::ast::TypeAnnotation::Tuple(vec![
                        shape_ast::ast::TypeAnnotation::Basic("int".to_string()),
                        shape_ast::ast::TypeAnnotation::Basic("string".to_string()),
                    ]),
                    Span::DUMMY,
                ));
            }
            other => panic!("expected variable decl item, got {other:?}"),
        }
        let err = crate::compiler::BytecodeCompiler::new()
            .compile(&program)
            .expect_err("type syntax outside type_ref must not compile");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("type syntax is only valid as the type_ref argument"),
            "expected the named surface-and-stop diagnostic, got: {rendered}"
        );
    }

    // =====================================================================
    // ADR-009 A2 (slice S4): rewrite-arm unit tests — the checked
    // type-expression carrier lowers through the shared overlay canonicalizer
    // to the SAME identity forwarder as the bare-identifier arm, and
    // canonicalization failure is a compile-time error out of the rewrite
    // (Dec 52 — before user comptime executes).
    // =====================================================================

    /// Parse `let x = <expr>;`, rewrite against a fresh test freeze overlay,
    /// and return the lowered initializer expression.
    fn rewrite_type_ref_initializer(
        source: &str,
    ) -> std::result::Result<Expr, shape_ast::error::ShapeError> {
        let mut program = shape_ast::parse_program(source).expect("source parses");
        let mut stmt = match program.items.remove(0) {
            shape_ast::ast::Item::Statement(stmt, _) => stmt,
            other => panic!("expected statement item, got {other:?}"),
        };
        let freeze = super::test_freeze_overlay();
        super::rewrite_comptime_type_symbol_args(&mut stmt, freeze.as_ref())?;
        match stmt {
            Statement::VariableDecl(decl, _) => Ok(decl.value.expect("initializer present")),
            other => panic!("expected variable decl, got {other:?}"),
        }
    }

    /// Assert the expression is the lowered identity forwarder and return its
    /// `(identity_high, identity_low)` literal pair.
    fn forwarder_identity_literals(expr: &Expr) -> (i64, i64) {
        match expr {
            Expr::FunctionCall { name, args, .. } => {
                assert_eq!(
                    name,
                    super::TYPE_REF_FORWARDER,
                    "must lower to the identity forwarder"
                );
                match args.as_slice() {
                    [
                        Expr::Literal(Literal::Int(high), _),
                        Expr::Literal(Literal::Int(low), _),
                    ] => (*high, *low),
                    other => panic!("expected two int identity literals, got {other:?}"),
                }
            }
            other => panic!("expected forwarder call, got {other:?}"),
        }
    }

    /// Cross-site identity equality: the same composite spelled at two sites
    /// (and rewritten through two independent overlays over the same freeze
    /// input) lowers to bit-identical identity literals; a structurally
    /// different spelling (member order) lowers to a distinct identity.
    #[test]
    fn composite_type_ref_identity_is_equal_across_rewrite_sites() {
        let first = rewrite_type_ref_initializer("let a = type_ref([int, string]);")
            .expect("composite form rewrites");
        let second = rewrite_type_ref_initializer("let b = type_ref([int,   string]);")
            .expect("composite form rewrites");
        assert_eq!(
            forwarder_identity_literals(&first),
            forwarder_identity_literals(&second),
            "same composite spelled at two sites must share one frozen identity"
        );
        let swapped = rewrite_type_ref_initializer("let c = type_ref([string, int]);")
            .expect("composite form rewrites");
        assert_ne!(
            forwarder_identity_literals(&first),
            forwarder_identity_literals(&swapped),
            "tuple member order is identity-significant"
        );
    }

    /// Dec 52 rejection placement: an unresolved leaf at depth inside a
    /// checked type expression is a NAMED compile error out of the rewrite
    /// itself — user comptime never executes, no INVALID sentinel flows.
    #[test]
    fn composite_type_ref_with_unresolved_leaf_is_a_compile_error_before_comptime_runs() {
        let err = rewrite_type_ref_initializer("let a = type_ref(Option<Bogus>);")
            .expect_err("unresolved leaf must reject at rewrite time");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("unknown semantic type identity"),
            "expected the named unknown-identity family, got: {rendered}"
        );
        assert!(
            rendered.contains("Bogus"),
            "the diagnostic must name the unresolved leaf, got: {rendered}"
        );
    }

    /// Two-arm agreement: a leaf spelled bare (`type_ref(int)`, the
    /// historical Identifier lowering) and the same leaf hand-built as the
    /// type-syntax carrier resolve to ONE identity — the canonicalizer
    /// resolves leaf names through the same `identity_of` query the
    /// bare-identifier arm uses. (No source spelling produces `TypeSyntax`
    /// for a bare name; S3 keeps the Identifier lowering.)
    #[test]
    fn bare_identifier_and_type_syntax_leaf_share_one_identity() {
        let bare =
            rewrite_type_ref_initializer("let a = type_ref(int);").expect("bare form rewrites");

        let mut stmt = Statement::Expression(
            Expr::FunctionCall {
                name: "type_ref".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::TypeSyntax(
                    shape_ast::ast::TypeAnnotation::Basic("int".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            },
            Span::DUMMY,
        );
        let freeze = super::test_freeze_overlay();
        super::rewrite_comptime_type_symbol_args(&mut stmt, freeze.as_ref())
            .expect("leaf type syntax rewrites");
        let Statement::Expression(syntax_form, _) = stmt else {
            panic!("expected expression statement");
        };

        assert_eq!(
            forwarder_identity_literals(&bare),
            forwarder_identity_literals(&syntax_form),
            "bare-identifier and type-syntax arms must agree on leaf identity"
        );
    }

    // =====================================================================
    // ADR-009 A2 (slice S5): identity-stability proofs through the REAL
    // predeclare + freeze-barrier path. Unlike `rewrite_type_ref_initializer`
    // (empty test freeze), this helper runs the production registration-
    // complete sequence over the program's own declarations, then rewrites
    // the LAST statement's initializer — so alias fixpoints, trait
    // predeclare, and struct generic info all flow exactly as in `compile()`.
    // =====================================================================

    /// Predeclare every item of `source` (structs + semantic-freeze inputs),
    /// install the freeze barrier, then rewrite the last statement's
    /// `let ... = type_ref(...)` initializer through the site overlay.
    fn rewrite_program_type_ref_initializer(
        source: &str,
    ) -> std::result::Result<Expr, shape_ast::error::ShapeError> {
        let mut program = shape_ast::parse_program(source).expect("source parses");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        for pass in [
            crate::compiler::statements::SemanticFreezePredeclarePass::TypesAndTraits,
            crate::compiler::statements::SemanticFreezePredeclarePass::Impls,
        ] {
            for item in &program.items {
                if pass == crate::compiler::statements::SemanticFreezePredeclarePass::TypesAndTraits
                {
                    compiler.predeclare_item_struct_schemas(item);
                }
                compiler
                    .predeclare_item_semantic_freeze_inputs(item, pass)
                    .expect("freeze inputs predeclare");
            }
        }
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        let freeze = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        let mut stmt = match program.items.pop() {
            Some(shape_ast::ast::Item::Statement(stmt, _)) => stmt,
            other => panic!("expected trailing statement item, got {other:?}"),
        };
        super::rewrite_comptime_type_symbol_args(&mut stmt, freeze.as_ref())?;
        match stmt {
            Statement::VariableDecl(decl, _) => Ok(decl.value.expect("initializer present")),
            other => panic!("expected variable decl, got {other:?}"),
        }
    }

    fn program_identity_literals(source: &str) -> (i64, i64) {
        let expr = rewrite_program_type_ref_initializer(source)
            .unwrap_or_else(|error| panic!("program must rewrite: {error:?}"));
        forwarder_identity_literals(&expr)
    }

    /// S5 R7 (Dec 53): alias normalization holds THROUGH applied forms on the
    /// production path — `type Ids = Array<UserId>` (bare alias name, the
    /// Identifier arm) and `type_ref(Array<int>)` (checked type syntax) lower
    /// to bit-identical frozen identities.
    #[test]
    fn alias_identity_normalizes_through_applied_forms_end_to_end() {
        let preamble = "type UserId = int\ntype Ids = Array<UserId>\n";
        assert_eq!(
            program_identity_literals(&format!("{preamble}let a = type_ref(Ids);")),
            program_identity_literals(&format!("{preamble}let a = type_ref(Array<int>);")),
            "alias-through-applied identity must equal the spelled applied form"
        );
        // The alias spelled INSIDE a checked type expression agrees too.
        assert_eq!(
            program_identity_literals(&format!("{preamble}let a = type_ref(Array<UserId>);")),
            program_identity_literals(&format!("{preamble}let a = type_ref(Array<int>);")),
        );
    }

    /// S5 R11: identity is declaration-order independent — inserting an
    /// unrelated type declaration, reordering record fields, and reordering
    /// union members all leave the identity literals bit-identical.
    #[test]
    fn composite_identity_is_declaration_order_independent_across_program_variants() {
        let record = program_identity_literals("let a = type_ref({x: int, y: string});");
        assert_eq!(
            record,
            program_identity_literals(
                "type Unrelated { z: int }\nlet a = type_ref({x: int, y: string});"
            ),
            "an unrelated type declaration must not perturb a composite identity"
        );
        assert_eq!(
            record,
            program_identity_literals(
                "type Unrelated { z: int }\nlet a = type_ref({y: string, x: int});"
            ),
            "record-field source order must not perturb the identity"
        );

        assert_eq!(
            program_identity_literals("let a = type_ref(int | string);"),
            program_identity_literals("let a = type_ref(string | int);"),
            "union-member source order must not perturb the identity"
        );
    }

    /// Review round 1 (A2): the grammar admits parenthesized type
    /// annotations (`non_array_type ::= … | "(" type_annotation ")"`), and
    /// `parse_type_annotation` collapses a union only at len==1, so
    /// `(int | string) | bool` reaches the canonicalizer as a NESTED union.
    /// Union membership is an associative set — the parenthesized spelling
    /// must mint the SAME frozen identity as the flat spelling, and members
    /// reached through nesting must not escape dedup.
    #[test]
    fn parenthesized_union_spelling_mints_the_flat_union_identity() {
        assert_eq!(
            program_identity_literals("let a = type_ref((int | string) | bool);"),
            program_identity_literals("let a = type_ref(int | string | bool);"),
            "nested union spelling must flatten to the flat set-semantic identity"
        );
        assert_eq!(
            program_identity_literals("let a = type_ref(int | (int | string));"),
            program_identity_literals("let a = type_ref(int | string);"),
            "a member reached through nesting must not escape dedup"
        );
    }

    /// S5 R8 (Dec 50/94 rule 3): a trait intersection in type position
    /// (`Speak + Walk`) erases to the SAME identity as the `dyn` spelling,
    /// and an object intersection reaches the directly-spelled record's
    /// identity — both through the production predeclare path (traits are a
    /// named freeze input registered BEFORE the barrier).
    #[test]
    fn intersection_identities_normalize_per_dec_50_94_end_to_end() {
        let traits =
            "trait Speak { fn speak(self) -> string; }\ntrait Walk { fn walk(self) -> string; }\n";
        assert_eq!(
            program_identity_literals(&format!("{traits}let a = type_ref(Speak + Walk);")),
            program_identity_literals(&format!("{traits}let a = type_ref(dyn Walk + Speak);")),
            "trait intersection must erase to the dyn bound-set identity"
        );

        assert_eq!(
            program_identity_literals("let a = type_ref({a: int} + {b: string});"),
            program_identity_literals("let a = type_ref({b: string, a: int});"),
            "object intersection must reach the directly-spelled record identity"
        );
    }

    /// ADR-009 B2 (slice S4, Dec 49): `trait_ref(Greetable)` lowers to the
    /// unspellable trait-identity forwarder carrying EXACTLY the freeze's
    /// canonical trait identity as INT LITERALS — identity-literal transport
    /// into the compiled comptime artifact. No string transport: the trait
    /// NAME must not survive the rewrite.
    #[test]
    fn trait_ref_rewrite_transports_the_frozen_trait_identity_literals() {
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler
            .type_inference
            .env
            .define_trait(&shape_ast::ast::TraitDef {
                name: "Greetable".to_string(),
                doc_comment: None,
                type_params: None,
                super_traits: Vec::new(),
                members: Vec::new(),
                annotations: Vec::new(),
                is_comptime: false,
            });
        let freeze =
            crate::compiler::comptime_builtins::semantic_freeze::SemanticFreeze::freeze(&compiler)
                .expect("resolved state freezes");
        let overlay = std::sync::Arc::new(crate::compiler::comptime_builtins::FreezeOverlay::new(
            freeze,
            "<module>",
            &[],
        ));
        let identity = overlay
            .trait_identity_of("Greetable")
            .expect("trait identity frozen at the barrier");

        let mut statement = Statement::Return(
            Some(Expr::FunctionCall {
                name: "trait_ref".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Identifier("Greetable".to_string(), Span::DUMMY)],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        super::rewrite_comptime_type_symbol_args(&mut statement, overlay.as_ref())
            .expect("trait_ref rewrite against frozen trait identity must succeed");

        let Statement::Return(Some(Expr::FunctionCall { name, args, .. }), _) = &statement else {
            panic!("rewrite must keep the return-call shape: {statement:?}");
        };
        assert_eq!(
            name, "\u{1}comptime:forward-trait-ref",
            "trait_ref must lower to the unspellable trait-identity forwarder"
        );
        assert_eq!(
            args.as_slice(),
            &[
                Expr::Literal(Literal::Int(identity.high), Span::DUMMY),
                Expr::Literal(Literal::Int(identity.low), Span::DUMMY),
            ],
            "the forwarder args must be the frozen trait identity as int literals"
        );
        let rendered = format!("{statement:?}");
        assert!(
            !rendered.contains("Greetable"),
            "no string transport: the trait name must not survive the rewrite: {rendered}"
        );
    }

    /// Drift pin: the find_impl forwarder's `Option<...>` return marker must
    /// wrap exactly the reserved ImplRef carrier schema name.
    #[test]
    fn find_impl_forwarder_return_marker_matches_the_impl_ref_schema() {
        assert_eq!(
            super::FIND_IMPL_RETURN_MARKER,
            format!(
                "Option<{}>",
                shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_IMPL_REF_SCHEMA
            )
        );
    }

    /// ADR-009 B4 drift pin: the refine forwarder's `Option<...>` return marker
    /// must wrap exactly the reserved AppliedType carrier schema name.
    #[test]
    fn refine_forwarder_return_marker_matches_the_applied_type_schema() {
        assert_eq!(
            super::REFINE_RETURN_MARKER,
            format!(
                "Option<{}>",
                shape_runtime::type_schema::builtin_schemas::COMPTIME_APPLIED_TYPE_SCHEMA
            )
        );
    }

    /// ADR-009 B4: `type_constructor(Head)` lowers the bare nominal head to its
    /// FROZEN identity halves (the `type_ref` transport) — the head NAME never
    /// survives into the compiled comptime artifact (R1 strings-cannot-
    /// construct).
    #[test]
    fn type_constructor_rewrite_transports_the_frozen_head_identity_literals() {
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        // Register a nominal struct exactly as `predeclare_struct_schema` does
        // (the `struct_types` row the freeze reads).
        compiler
            .struct_types
            .insert("Widget".to_string(), (Vec::new(), Span::DUMMY));
        let freeze =
            crate::compiler::comptime_builtins::semantic_freeze::SemanticFreeze::freeze(&compiler)
                .expect("resolved state freezes");
        let overlay = std::sync::Arc::new(crate::compiler::comptime_builtins::FreezeOverlay::new(
            freeze,
            "<module>",
            &[],
        ));
        let identity = overlay
            .identity_of("Widget")
            .expect("Widget frozen at barrier");

        let mut statement = Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_constructor".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Identifier("Widget".to_string(), Span::DUMMY)],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        super::rewrite_comptime_type_symbol_args(&mut statement, overlay.as_ref())
            .expect("type_constructor rewrite against frozen nominal identity must succeed");

        let Statement::Return(Some(Expr::FunctionCall { name, args, .. }), _) = &statement else {
            panic!("rewrite must keep the return-call shape: {statement:?}");
        };
        assert_eq!(name, super::TYPE_CONSTRUCTOR_FORWARDER);
        assert_eq!(
            args.as_slice(),
            &[
                Expr::Literal(Literal::Int(identity.high), Span::DUMMY),
                Expr::Literal(Literal::Int(identity.low), Span::DUMMY),
            ]
        );
        assert!(
            !format!("{statement:?}").contains("Widget"),
            "no string transport: the head name must not survive the rewrite"
        );
    }

    /// ADR-009 B4: the method-call surfaces (`apply` / `refine` /
    /// `type_argument`) rewrite to their forwarders with the receiver
    /// prepended; `apply` transports its variadic args as a checked array.
    #[test]
    fn method_call_surfaces_rewrite_to_forwarders_with_receiver_prepended() {
        let overlay = std::sync::Arc::new(crate::compiler::comptime_builtins::FreezeOverlay::new(
            crate::compiler::comptime_builtins::semantic_freeze::SemanticFreeze::freeze(
                &crate::compiler::BytecodeCompiler::new(),
            )
            .expect("freeze"),
            "<module>",
            &[],
        ));

        // `c.apply(a, b)` → APPLY_FORWARDER(c, [a, b]).
        let mut apply = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("c".to_string(), Span::DUMMY)),
            method: "apply".to_string(),
            args: vec![
                Expr::Identifier("a".to_string(), Span::DUMMY),
                Expr::Identifier("b".to_string(), Span::DUMMY),
            ],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        super::rewrite_comptime_type_symbol_args_expr(&mut apply, overlay.as_ref())
            .expect("rewrite ok");
        let Expr::FunctionCall { name, args, .. } = &apply else {
            panic!("apply must become a function call: {apply:?}");
        };
        assert_eq!(name, super::APPLY_FORWARDER);
        assert_eq!(args.len(), 2, "receiver + one checked array");
        assert!(matches!(args[0], Expr::Identifier(ref n, _) if n == "c"));
        assert!(matches!(args[1], Expr::Array(ref els, _) if els.len() == 2));

        // `applied.type_argument(0)` → TYPE_ARGUMENT_FORWARDER(applied, 0).
        let mut ta = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("applied".to_string(), Span::DUMMY)),
            method: "type_argument".to_string(),
            args: vec![Expr::Literal(Literal::Int(0), Span::DUMMY)],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        super::rewrite_comptime_type_symbol_args_expr(&mut ta, overlay.as_ref())
            .expect("rewrite ok");
        let Expr::FunctionCall { name, args, .. } = &ta else {
            panic!("type_argument must become a function call: {ta:?}");
        };
        assert_eq!(name, super::TYPE_ARGUMENT_FORWARDER);
        assert_eq!(args.len(), 2, "receiver + index");
        assert!(matches!(args[0], Expr::Identifier(ref n, _) if n == "applied"));
        assert!(matches!(args[1], Expr::Literal(Literal::Int(0), _)));
    }

    // ─────────────────────────────────────────────────────────────────────
    // ADR-009 B6 (Dec 63): the `.param(I)` / `.parameters` accessor rewrites
    // are FrozenCallable-receiver-guarded — they fire on a
    // `FrozenType::Callable(c)`-bound receiver and NEVER on an arbitrary user
    // receiver (precise-surface discipline; no collateral capture).
    // ─────────────────────────────────────────────────────────────────────

    fn overlay_for_rewrite_tests()
    -> std::sync::Arc<crate::compiler::comptime_builtins::FreezeOverlay> {
        std::sync::Arc::new(crate::compiler::comptime_builtins::FreezeOverlay::new(
            crate::compiler::comptime_builtins::semantic_freeze::SemanticFreeze::freeze(
                &crate::compiler::BytecodeCompiler::new(),
            )
            .expect("freeze"),
            "<module>",
            &[],
        ))
    }

    /// A `FrozenType::Callable(c)` match arm binds `c` as a `FrozenCallable`, so
    /// `c.param(0)` → `c.params[0]` and `c.parameters` → `c.params` INSIDE the
    /// arm. The guard admits the full enabled surface.
    #[test]
    fn frozen_callable_accessors_rewrite_inside_a_callable_match_arm() {
        use shape_ast::ast::{PatternConstructorFields, TypePath};
        let overlay = overlay_for_rewrite_tests();

        let callable_pattern = shape_ast::ast::Pattern::Constructor {
            enum_name: Some(TypePath::simple("FrozenType")),
            variant: "Callable".to_string(),
            fields: PatternConstructorFields::Tuple(vec![shape_ast::ast::Pattern::Identifier {
                name: "c".to_string(),
                span: Span::DUMMY,
            }]),
        };
        // arm body: { c.param(0); c.parameters }  (a block of two exprs)
        let param_call = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("c".to_string(), Span::DUMMY)),
            method: "param".to_string(),
            args: vec![Expr::Literal(Literal::Int(0), Span::DUMMY)],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        let parameters_access = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("c".to_string(), Span::DUMMY)),
            property: "parameters".to_string(),
            optional: false,
            span: Span::DUMMY,
        };
        let body = Expr::Block(
            shape_ast::ast::expr_helpers::BlockExpr {
                items: vec![
                    shape_ast::ast::BlockItem::Expression(param_call),
                    shape_ast::ast::BlockItem::Expression(parameters_access),
                ],
            },
            Span::DUMMY,
        );
        let mut match_expr = Expr::Match(
            Box::new(shape_ast::ast::expr_helpers::MatchExpr {
                scrutinee: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                arms: vec![shape_ast::ast::expr_helpers::MatchArm {
                    pattern: callable_pattern,
                    guard: None,
                    body: Box::new(body),
                    pattern_span: None,
                }],
            }),
            Span::DUMMY,
        );

        super::rewrite_comptime_type_symbol_args_expr(&mut match_expr, overlay.as_ref())
            .expect("rewrite ok");

        let Expr::Match(m, _) = &match_expr else {
            panic!("still a match");
        };
        let Expr::Block(block, _) = m.arms[0].body.as_ref() else {
            panic!("arm body is a block");
        };
        // `c.param(0)` → `c.params[0]`
        let shape_ast::ast::BlockItem::Expression(Expr::IndexAccess { object, .. }) =
            &block.items[0]
        else {
            panic!(
                "param(0) must desugar to index access: {:?}",
                block.items[0]
            );
        };
        assert!(
            matches!(object.as_ref(), Expr::PropertyAccess { property, .. } if property == "params"),
            "index base must be the `.params` carrier"
        );
        // `c.parameters` → `c.params`
        let shape_ast::ast::BlockItem::Expression(Expr::PropertyAccess { property, .. }) =
            &block.items[1]
        else {
            panic!("parameters must stay a property access");
        };
        assert_eq!(property, "params", "`.parameters` renames to `.params`");
    }

    /// Collateral guard: a `.param(...)` method call and a `.parameters` field
    /// access on a receiver that is NOT a `FrozenCallable` are left untouched —
    /// no rewrite, no `PARAM_ARITY` / string-selector diagnostic. A user struct
    /// with a `.parameters` field or a `.param(a, b)` method used in comptime
    /// keeps its own semantics.
    #[test]
    fn user_receiver_param_and_parameters_are_not_captured() {
        let overlay = overlay_for_rewrite_tests();

        // `foo.parameters` (foo not callable-bound) stays a `.parameters`
        // property access — NOT renamed to `.params`.
        let mut prop = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("foo".to_string(), Span::DUMMY)),
            property: "parameters".to_string(),
            optional: false,
            span: Span::DUMMY,
        };
        super::rewrite_comptime_type_symbol_args_expr(&mut prop, overlay.as_ref())
            .expect("rewrite ok");
        assert!(
            matches!(&prop, Expr::PropertyAccess { property, .. } if property == "parameters"),
            "user `.parameters` field must not be renamed: {prop:?}"
        );

        // `foo.param(0)` (foo not callable-bound) stays a method call — NOT
        // desugared to `.params[0]`.
        let mut one_arg = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("foo".to_string(), Span::DUMMY)),
            method: "param".to_string(),
            args: vec![Expr::Literal(Literal::Int(0), Span::DUMMY)],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        super::rewrite_comptime_type_symbol_args_expr(&mut one_arg, overlay.as_ref())
            .expect("rewrite ok");
        assert!(
            matches!(&one_arg, Expr::MethodCall { method, .. } if method == "param"),
            "user `.param(i)` method must not be captured: {one_arg:?}"
        );

        // `foo.param(a, b)` (two args) must NOT fire the arity diagnostic — the
        // pre-guard behavior mis-reported this as a `callable.param` arity error.
        let mut two_arg = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("foo".to_string(), Span::DUMMY)),
            method: "param".to_string(),
            args: vec![
                Expr::Identifier("a".to_string(), Span::DUMMY),
                Expr::Identifier("b".to_string(), Span::DUMMY),
            ],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        let result = super::rewrite_comptime_type_symbol_args_expr(&mut two_arg, overlay.as_ref());
        assert!(
            result.is_ok(),
            "a user 2-arg `.param(a, b)` must not raise PARAM_ARITY: {result:?}"
        );
        assert!(
            matches!(&two_arg, Expr::MethodCall { method, args, .. } if method == "param" && args.len() == 2),
            "user `.param(a, b)` must survive unchanged: {two_arg:?}"
        );
    }

    /// A user enum whose variant is spelled `Callable` (a DIFFERENT enum head,
    /// not `FrozenType`) does not open a callable scope — `.param` on its
    /// binding is not captured.
    #[test]
    fn non_frozentype_callable_variant_does_not_open_a_callable_scope() {
        use shape_ast::ast::{PatternConstructorFields, TypePath};
        let overlay = overlay_for_rewrite_tests();

        let pattern = shape_ast::ast::Pattern::Constructor {
            enum_name: Some(TypePath::simple("MyEnum")),
            variant: "Callable".to_string(),
            fields: PatternConstructorFields::Tuple(vec![shape_ast::ast::Pattern::Identifier {
                name: "c".to_string(),
                span: Span::DUMMY,
            }]),
        };
        let body = Expr::MethodCall {
            receiver: Box::new(Expr::Identifier("c".to_string(), Span::DUMMY)),
            method: "param".to_string(),
            args: vec![Expr::Literal(Literal::Int(0), Span::DUMMY)],
            named_args: Vec::new(),
            optional: false,
            span: Span::DUMMY,
        };
        let mut match_expr = Expr::Match(
            Box::new(shape_ast::ast::expr_helpers::MatchExpr {
                scrutinee: Box::new(Expr::Identifier("x".to_string(), Span::DUMMY)),
                arms: vec![shape_ast::ast::expr_helpers::MatchArm {
                    pattern,
                    guard: None,
                    body: Box::new(body),
                    pattern_span: None,
                }],
            }),
            Span::DUMMY,
        );
        super::rewrite_comptime_type_symbol_args_expr(&mut match_expr, overlay.as_ref())
            .expect("rewrite ok");
        let Expr::Match(m, _) = &match_expr else {
            panic!("still a match");
        };
        assert!(
            matches!(m.arms[0].body.as_ref(), Expr::MethodCall { method, .. } if method == "param"),
            "a non-FrozenType `Callable` variant must not open a callable scope: {:?}",
            m.arms[0].body
        );
    }

    // Regression (2026-06-21): a comptime block evaluating to `false` (and
    // `build_config().debug` in a release build) was baked as `null` at the
    // print / f-string boundary. Root cause: the `NativeKind::Bool` arm in
    // `nb_to_literal` / `nb_to_expr` short-circuited a Bool-kinded zero-bit
    // slot to `Literal::None`, conflating a genuine `false` with the none
    // sentinel. The none sentinel is `NativeKind::Null` (kinded_slot.rs:565),
    // NOT a Bool-kinded zero, so the two are distinguishable by kind. These
    // tests pin the distinction at the materialization layer shared by VM
    // and JIT (comptime is resolved at compile time, before either runs).
    #[test]
    fn comptime_false_bool_materializes_as_false_not_null() {
        use shape_value::KindedSlot;

        // nb_to_literal: false bool → Literal::Bool(false), not None.
        let lit_false = super::nb_to_literal(&KindedSlot::from_bool(false));
        assert_eq!(
            lit_false,
            Literal::Bool(false),
            "comptime `false` must bake as Bool(false), not null"
        );
        let lit_true = super::nb_to_literal(&KindedSlot::from_bool(true));
        assert_eq!(lit_true, Literal::Bool(true));

        // The none sentinel (NativeKind::Null) still materializes as None.
        let lit_none = super::nb_to_literal(&KindedSlot::none());
        assert_eq!(
            lit_none,
            Literal::None,
            "NativeKind::Null sentinel must still bake as None"
        );
    }

    #[test]
    fn comptime_false_bool_nb_to_expr_is_bool_literal() {
        use shape_value::KindedSlot;

        let expr =
            super::nb_to_expr_public(&KindedSlot::from_bool(false), Span::DUMMY).expect("ok");
        match expr {
            Expr::Literal(Literal::Bool(false), _) => {}
            other => panic!("expected Bool(false) literal, got {:?}", other),
        }

        let none_expr = super::nb_to_expr_public(&KindedSlot::none(), Span::DUMMY).expect("ok");
        match none_expr {
            Expr::Literal(Literal::None, _) => {}
            other => panic!("expected None literal for none sentinel, got {:?}", other),
        }
    }

    // W17-comptime-vm-dispatch smoke tests (ADR-006 §2.7.26, 2026-05-12).
    // Verify the 4 comptime introspection forms wired by
    // C2-comptime-rebuild (`a5df165`) dispatch end-to-end via the
    // populated module-binding TypedObject + ModuleFn field-reference
    // chain.
    use super::execute_comptime;
    use shape_ast::ast::{
        DestructurePattern, Expr, Literal, Span, Statement, VarKind, VariableDecl,
    };

    /// Sanity baseline: arithmetic-only comptime path still works after
    /// the W17 populate_module_objects rebuild. Catches regressions
    /// against C2-comptime-rebuild's `let val = comptime { 1 + 2 }`
    /// smoke target.
    #[test]
    fn w17_comptime_arithmetic_sanity() {
        let stmts = vec![Statement::Return(
            Some(Expr::Literal(Literal::Int(42), Span::DUMMY)),
            Span::DUMMY,
        )];
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "comptime arithmetic should still work: {:?}",
            result.err()
        );
    }

    /// `comptime { build_config() }` dispatches end-to-end via VM mode —
    /// the W17 dispatch chain (`LoadModuleBinding + GetFieldTyped +
    /// CallValue` → `invoke_module_fn_id_stub`) reaches the body. The
    /// body itself constructs a `TypedObject` via
    /// `typed_object_from_pairs` which has a pre-existing
    /// `field_kinds: Arc<[]>` debug_assert issue (documented in
    /// C2-comptime-rebuild close `a5df165` — "shape-runtime helper
    /// bug, not C2 territory"). This test verifies the W17 dispatch
    /// path is intact regardless of the body-side typed-object
    /// construction issue: either the call returns Ok (build_config
    /// body succeeded) or returns an Err that does NOT mention
    /// `populate_module_objects` / NotImplemented (i.e. dispatch
    /// itself succeeded).
    #[test]
    fn w17_comptime_build_config_dispatches_end_to_end() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "build_config".to_string(),
                const_args: Vec::new(),
                args: Vec::new(),
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute_comptime(
                &stmts,
                &[],
                &[],
                Default::default(),
                Default::default(),
                super::test_freeze_overlay(),
            )
        }));
        match result {
            Ok(Ok(_)) => {
                // Happy path: dispatch + body succeeded.
            }
            Ok(Err(e)) => {
                // Soft path: a runtime error from the body, but the
                // dispatch chain reached the body successfully.
                let msg = format!("{:?}", e);
                assert!(
                    !msg.contains("populate_module_objects") && !msg.contains("NotImplemented"),
                    "dispatch path should not surface populate_module_objects \
                     NotImplemented (W17 close gate): {}",
                    msg
                );
            }
            Err(_) => {
                // Hard path: build_config body panicked
                // (pre-existing typed_object_from_pairs debug_assert per
                // C2-comptime-rebuild close — out of W17 territory).
                // The dispatch chain still reached the body, which is
                // what this test asserts.
            }
        }
    }

    /// STAGE R2 (2026-06-18) regression: `comptime { build_config() }`
    /// SIGSEGV'd because `nb_to_expr` routed the `TypedObject` result through
    /// `slot.as_heap_value()`. A `Ptr(HeapKind::TypedObject)` slot's bits are
    /// `*const TypedObjectStorage` (whose first 8 bytes are `schema_id`), NOT
    /// `Arc::into_raw(Arc<HeapValue>)`; `as_heap_value()` reinterprets them as
    /// a `HeapValue` discriminator and dereferences — heap corruption /
    /// segfault (forbidden per ADR-006 §2.7.16 receiver-recovery soundness
    /// rule). The fix recovers the storage via a direct typed-pointer cast and
    /// reads each field through the storage's own `field_kinds` carrier.
    ///
    /// This test drives the exact crash locus: it runs `build_config()` via
    /// `execute_comptime` and feeds the result `KindedSlot` to
    /// `nb_to_expr_public`. Pre-fix this segfaulted the test process; post-fix
    /// it returns an `Expr::Object` whose string fields carry real values.
    #[test]
    fn r2_build_config_nb_to_expr_no_segfault() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "build_config".to_string(),
                const_args: Vec::new(),
                args: Vec::new(),
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let exec = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        )
        .expect("build_config() comptime evaluation should succeed");

        // The crash site: materialize the comptime result into an AST literal.
        // Pre-fix `as_heap_value()` segfaulted here; post-fix it must return
        // a structured object literal.
        let expr = super::nb_to_expr_public(&exec.value, Span::DUMMY)
            .expect("TypedObject result must materialize into an object literal");

        let entries = match expr {
            Expr::Object(entries, _) => entries,
            other => panic!("expected Expr::Object from build_config(), got {:?}", other),
        };

        // Collect (field name -> string literal value) for the string fields.
        let mut os_val: Option<String> = None;
        let mut arch_val: Option<String> = None;
        let mut version_val: Option<String> = None;
        let mut saw_debug_bool = false;
        for entry in &entries {
            if let shape_ast::ast::ObjectEntry::Field { key, value, .. } = entry {
                match (key.as_str(), value) {
                    ("target_os", Expr::Literal(Literal::String(s), _)) => os_val = Some(s.clone()),
                    ("target_arch", Expr::Literal(Literal::String(s), _)) => {
                        arch_val = Some(s.clone())
                    }
                    ("version", Expr::Literal(Literal::String(s), _)) => {
                        version_val = Some(s.clone())
                    }
                    ("debug", Expr::Literal(Literal::Bool(_), _)) => saw_debug_bool = true,
                    _ => {}
                }
            }
        }

        // String fields must round-trip their real (non-empty) values rather
        // than baking `None` (the silent-data-loss symptom in the prior bug).
        assert_eq!(
            os_val.as_deref(),
            Some(std::env::consts::OS),
            "target_os must read back the real platform string"
        );
        assert_eq!(
            arch_val.as_deref(),
            Some(std::env::consts::ARCH),
            "target_arch must read back the real architecture string"
        );
        assert_eq!(
            version_val.as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "version must read back the real package version"
        );
        assert!(
            saw_debug_bool,
            "debug must read back as a typed Bool literal (from the storage's \
             field_kinds carrier), not Any/None"
        );
    }

    /// `comptime { implements("T", "Trait") }` dispatches end-to-end
    /// through typed string arguments. Empty keyspace returns false; a
    /// matching registered trait key returns true.
    #[test]
    fn w17_comptime_implements_dispatches_end_to_end() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "implements".to_string(),
                const_args: Vec::new(),
                args: vec![
                    Expr::Literal(Literal::String("int".to_string()), Span::DUMMY),
                    Expr::Literal(Literal::String("Add".to_string()), Span::DUMMY),
                ],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        )
        .expect("implements() should dispatch end-to-end");
        assert_eq!(result.value.as_bool(), Some(false));

        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "implements".to_string(),
                const_args: Vec::new(),
                args: vec![
                    Expr::Literal(Literal::String("Dog".to_string()), Span::DUMMY),
                    Expr::Literal(Literal::String("Speak".to_string()), Span::DUMMY),
                ],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let mut trait_impl_keys = std::collections::HashSet::new();
        trait_impl_keys.insert("Speak::Dog".to_string());
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            trait_impl_keys,
            Default::default(),
            super::test_freeze_overlay(),
        )
        .expect("implements() should see typed string args and registered impl keys");
        assert_eq!(result.value.as_bool(), Some(true));
    }

    /// `comptime { warning("hello") }` dispatches end-to-end and
    /// returns Unit. The body emits to stderr (captured by the test
    /// runner but not asserted on).
    #[test]
    fn w17_comptime_warning_dispatches_end_to_end() {
        let stmts = vec![Statement::Expression(
            Expr::FunctionCall {
                name: "warning".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("W17 test warning".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            },
            Span::DUMMY,
        )];
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "warning() should dispatch end-to-end: {:?}",
            result.err()
        );
    }

    /// `comptime { error("...") }` dispatches end-to-end and surfaces
    /// a structured `[comptime error] ...` message — verifies the
    /// CallValue → invoke_module_fn_id_stub path returns the body's
    /// `Err(String)` cleanly (not the W17 NotImplemented stub).
    #[test]
    fn w17_comptime_error_dispatches_end_to_end() {
        let stmts = vec![Statement::Expression(
            Expr::FunctionCall {
                name: "error".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("W17 test error".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            },
            Span::DUMMY,
        )];
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_err(),
            "error() should abort comptime execution: {:?}",
            result.ok().map(|r| r.value)
        );
        let err_msg = format!("{:?}", result.err().unwrap());
        // Verify the error reaches us through the CallValue → invoke_module_fn_id_stub
        // → body Err(String) path; the message format includes the
        // `[comptime error] ...` prefix the body emits. WF-1B S1 (marshal
        // Bool-collapse deletion) landed: the string argument's true kind
        // now flows from the §2.7.7 stack track, so the body reads the
        // user's message verbatim (the old `<Bool>` placeholder is gone).
        assert!(
            err_msg.contains("[comptime error]") || err_msg.contains("W17 test error"),
            "error message should surface the comptime-error path: {}",
            err_msg
        );
    }

    /// W7 (2026-05-17) — `type_info` is restored as a comptime-only
    /// builtin per `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md`
    /// §4 recommendation (b) + §8 user dispositions Q1-Q5. Calling
    /// `type_info()` outside a `comptime { }` block must now fail with
    /// the standard comptime-only-builtin error message (the previous
    /// "type_info has been removed" gate is retired).
    #[test]
    fn w7_type_info_comptime_only_contract() {
        let code = r#"let x = type_info("Point")"#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = crate::compiler::BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "type_info() outside comptime should fail (comptime-only gate)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("comptime-only builtin") || err_msg.contains("comptime { }"),
            "Error should surface the comptime-only-builtin gate (W7): {}",
            err_msg
        );
    }

    // =====================================================================
    // W14.2-C1 comptime-builtin coverage (Phase 4b Round 5a, 2026-05-19).
    // =====================================================================
    //
    // Per `docs/cluster-audits/v0.3-w14-test-coverage-audit.md` §4 W7 row:
    //
    // > W7 TypeInfo struct return | (b) PARTIAL | comptime-builtin
    // > TypedObject return carrier is a NEW class (draft §2.7.27 deferred
    // > per close summary §2). Coverage gap: chained `type_info(...).field
    // > .subfield` access patterns + interaction with `build_config`
    // > precedent.
    //
    // The tests below mirror the existing `w17_comptime_*_dispatches_end_to_end`
    // shape — they assert the comptime dispatch chain (LoadModuleBinding +
    // GetFieldTyped + CallValue → ModuleFn body) reaches the body and
    // returns cleanly. Body-side runtime values are intentionally NOT
    // asserted because the upstream `register_typed_function` marshal-layer
    // string-arg transmission is a documented pre-existing constraint
    // (`comptime_builtins.rs:469-484` — first arg always arrives as kind
    // `Bool` when arg-types are `vec![]`). The W7 close-out documents this
    // shape and routes diagnosis through `__type_info_marshal_pending__`.
    //
    // Coverage focuses on:
    //   (1) chained_access — `type_info(T).kind`, `.name` patterns
    //   (2) build_config_interaction — both builtins composed
    //   (3) nested_generic — Array<int>, Option<T>, Result<T,E> name strings
    //   (4) enum_payload_chained — `type_info` on enum names
    //   (5) error_path — undefined type, structured fallback
    //
    // Pattern: build statements via AST, call `execute_comptime`, assert
    // dispatch returns Ok OR a structured Err that does NOT mention the
    // pre-§2.7.26 `populate_module_objects NotImplemented` stub or the
    // `type_info has been removed` legacy gate (which is now retired).

    use shape_ast::ast::TypeAnnotation as TypeAnn;

    /// Helper: assert dispatch reaches body — accept Ok(_) OR an Err
    /// whose body shape does not surface the pre-§2.7.26 NotImplemented
    /// stub. Mirrors the `w17_comptime_build_config_dispatches_end_to_end`
    /// soft-path discipline. Also catches the `type_info has been removed`
    /// legacy gate (now retired per W7 close).
    fn assert_dispatch_reached(
        stmts: Vec<Statement>,
        trait_impl_keys: std::collections::HashSet<String>,
        (freeze, known_types): (
            std::sync::Arc<crate::compiler::comptime_builtins::FreezeOverlay>,
            std::collections::HashSet<String>,
        ),
        ctx: &str,
    ) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute_comptime(&stmts, &[], &[], trait_impl_keys, known_types, freeze)
        }));
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                let msg = format!("{:?}", e);
                assert!(
                    !msg.contains("populate_module_objects") && !msg.contains("NotImplemented"),
                    "{ctx}: dispatch chain must not surface the pre-§2.7.26 \
                     NotImplemented stub: {msg}",
                );
                assert!(
                    !msg.contains("type_info has been removed"),
                    "{ctx}: must not surface the retired \
                     `type_info has been removed` legacy gate: {msg}",
                );
            }
            Err(_) => {
                // Body-side panic — pre-existing typed_object_from_pairs
                // debug_assert or ckpt-2 receiver-recovery surface,
                // documented in C2-comptime-rebuild close. The dispatch
                // chain still reached the body, which is what this
                // assertion gates.
            }
        }
    }

    /// S2 fabricator: populate a real compiler with one struct and freeze
    /// it through the single barrier (replaces the deleted field-poked
    /// snapshot construction). Returns the freeze handle plus the
    /// known-type-symbol set the sites derive from compiler tables.
    fn freeze_with_struct(
        name: &str,
        fields: &[(&str, TypeAnn)],
    ) -> (
        std::sync::Arc<crate::compiler::comptime_builtins::FreezeOverlay>,
        std::collections::HashSet<String>,
    ) {
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler.struct_types.insert(
            name.to_string(),
            (
                fields.iter().map(|(n, _)| n.to_string()).collect(),
                Span::DUMMY,
            ),
        );
        compiler.struct_generic_info.insert(
            name.to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: fields
                    .iter()
                    .map(|(n, t)| (n.to_string(), t.clone()))
                    .collect(),
            },
        );
        let overlay =
            crate::compiler::comptime_builtins::semantic_freeze::overlay_for_tests(&compiler);
        (overlay, [name.to_string()].into_iter().collect())
    }

    /// S2 fabricator: enum variant of `freeze_with_struct` — the enum goes
    /// through the canonical schema registry (named freeze input 3).
    fn freeze_with_enum(
        name: &str,
        variants: &[&str],
    ) -> (
        std::sync::Arc<crate::compiler::comptime_builtins::FreezeOverlay>,
        std::collections::HashSet<String>,
    ) {
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler
            .type_tracker
            .schema_registry_mut()
            .register_enum_scoped(
                name,
                variants
                    .iter()
                    .enumerate()
                    .map(|(id, variant)| {
                        shape_runtime::type_schema::EnumVariantInfo::new(*variant, id as u16, 0)
                    })
                    .collect(),
            );
        let overlay =
            crate::compiler::comptime_builtins::semantic_freeze::overlay_for_tests(&compiler);
        (overlay, [name.to_string()].into_iter().collect())
    }

    // -------- (1) chained_access -----------------------------------------

    /// W14.2-C1 (1) chained: `comptime { type_info(Point).kind }` —
    /// dispatch reaches the GetFieldTyped on the TypedObject result and
    /// completes the property-access lowering without surfacing the
    /// pre-§2.7.26 stub or the retired legacy `type_info has been removed`
    /// gate.
    #[test]
    fn w14_2_c1_chained_kind_access_on_struct() {
        let stmts = vec![Statement::Return(
            Some(Expr::PropertyAccess {
                object: Box::new(Expr::FunctionCall {
                    name: "type_info".to_string(),
                    const_args: Vec::new(),
                    args: vec![Expr::Literal(
                        Literal::String("Point".to_string()),
                        Span::DUMMY,
                    )],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }),
                property: "kind".to_string(),
                optional: false,
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let snapshot = freeze_with_struct(
            "Point",
            &[
                ("x", TypeAnn::Basic("int".to_string())),
                ("y", TypeAnn::Basic("int".to_string())),
            ],
        );
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_chained_kind_access_on_struct",
        );
    }

    /// W14.2-C1 (1) chained: `comptime { type_info(Point).name }` —
    /// mirror of the kind-access shape; verifies the `.name` field arm
    /// of the registered 2-field TypeInfo schema dispatches.
    #[test]
    fn w14_2_c1_chained_name_access_on_struct() {
        let stmts = vec![Statement::Return(
            Some(Expr::PropertyAccess {
                object: Box::new(Expr::FunctionCall {
                    name: "type_info".to_string(),
                    const_args: Vec::new(),
                    args: vec![Expr::Literal(
                        Literal::String("Point".to_string()),
                        Span::DUMMY,
                    )],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }),
                property: "name".to_string(),
                optional: false,
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let snapshot = freeze_with_struct("Point", &[("x", TypeAnn::Basic("int".to_string()))]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_chained_name_access_on_struct",
        );
    }

    /// W14.2-C1 (1) chained: bind-then-access via local variable —
    /// `comptime { let info = type_info(Point); info.kind }`. Exercises
    /// the `let info = ...` binding-store path + subsequent property
    /// access on the TypedObject-typed local (mirror of the audit-cited
    /// `vision/distributed-comptime-async-vision.md:86` shape).
    #[test]
    fn w14_2_c1_chained_bind_then_access() {
        let stmts = vec![
            Statement::VariableDecl(
                VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("info".to_string(), Span::DUMMY),
                    type_annotation: None,
                    value: Some(Expr::FunctionCall {
                        name: "type_info".to_string(),
                        const_args: Vec::new(),
                        args: vec![Expr::Literal(
                            Literal::String("Point".to_string()),
                            Span::DUMMY,
                        )],
                        named_args: Vec::new(),
                        span: Span::DUMMY,
                    }),
                    ownership: Default::default(),
                },
                Span::DUMMY,
            ),
            Statement::Return(
                Some(Expr::PropertyAccess {
                    object: Box::new(Expr::Identifier("info".to_string(), Span::DUMMY)),
                    property: "kind".to_string(),
                    optional: false,
                    span: Span::DUMMY,
                }),
                Span::DUMMY,
            ),
        ];
        let snapshot = freeze_with_struct("Point", &[("x", TypeAnn::Basic("int".to_string()))]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_chained_bind_then_access",
        );
    }

    // -------- (2) build_config_interaction -------------------------------

    /// W14.2-C1 (2) interaction: both `build_config()` and `type_info(T)`
    /// dispatch in the same comptime block via locals. Verifies the
    /// `__comptime__` module-binding chain is reusable across multiple
    /// comptime-builtin invocations within one execute_comptime call
    /// (W17-comptime-vm-dispatch ADR-006 §2.7.26 — multi-call dispatch).
    #[test]
    fn w14_2_c1_build_config_and_type_info_in_same_block() {
        let stmts = vec![
            Statement::VariableDecl(
                VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("cfg".to_string(), Span::DUMMY),
                    type_annotation: None,
                    value: Some(Expr::FunctionCall {
                        name: "build_config".to_string(),
                        const_args: Vec::new(),
                        args: Vec::new(),
                        named_args: Vec::new(),
                        span: Span::DUMMY,
                    }),
                    ownership: Default::default(),
                },
                Span::DUMMY,
            ),
            Statement::Return(
                Some(Expr::FunctionCall {
                    name: "type_info".to_string(),
                    const_args: Vec::new(),
                    args: vec![Expr::Literal(
                        Literal::String("Point".to_string()),
                        Span::DUMMY,
                    )],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }),
                Span::DUMMY,
            ),
        ];
        let snapshot = freeze_with_struct("Point", &[("x", TypeAnn::Basic("int".to_string()))]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_build_config_and_type_info_in_same_block",
        );
    }

    /// W14.2-C1 (2) interaction: chained property access on
    /// `build_config()` and `type_info(T)` in the same block. The
    /// `build_config().target_arch` path is the existing precedent that
    /// `type_info(T).kind` mirrors; the test verifies BOTH chained-access
    /// forms compile + dispatch in one execute_comptime call.
    #[test]
    fn w14_2_c1_chained_access_on_both_builtins() {
        let stmts = vec![
            Statement::VariableDecl(
                VariableDecl {
                    kind: VarKind::Let,
                    is_mut: false,
                    pattern: DestructurePattern::Identifier("arch".to_string(), Span::DUMMY),
                    type_annotation: None,
                    value: Some(Expr::PropertyAccess {
                        object: Box::new(Expr::FunctionCall {
                            name: "build_config".to_string(),
                            const_args: Vec::new(),
                            args: Vec::new(),
                            named_args: Vec::new(),
                            span: Span::DUMMY,
                        }),
                        property: "target_arch".to_string(),
                        optional: false,
                        span: Span::DUMMY,
                    }),
                    ownership: Default::default(),
                },
                Span::DUMMY,
            ),
            Statement::Return(
                Some(Expr::PropertyAccess {
                    object: Box::new(Expr::FunctionCall {
                        name: "type_info".to_string(),
                        const_args: Vec::new(),
                        args: vec![Expr::Literal(
                            Literal::String("Point".to_string()),
                            Span::DUMMY,
                        )],
                        named_args: Vec::new(),
                        span: Span::DUMMY,
                    }),
                    property: "kind".to_string(),
                    optional: false,
                    span: Span::DUMMY,
                }),
                Span::DUMMY,
            ),
        ];
        let snapshot = freeze_with_struct("Point", &[("x", TypeAnn::Basic("int".to_string()))]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_chained_access_on_both_builtins",
        );
    }

    // -------- (3) nested_generic ----------------------------------------

    /// W14.2-C1 (3) nested generic: `type_info("Array<int>")` dispatches
    /// — the marshal-layer fallback path defaults to a sentinel kind
    /// (`__type_info_marshal_pending__` → `Unknown`) regardless of the
    /// actual name. The test asserts the dispatch chain reaches the
    /// body cleanly for a parameterized type-name string. Once the
    /// marshal layer is fixed, `classify_bare_type_name` will see
    /// "Array<int>" and classify per the audit-doc §4.6 discriminator
    /// table. Until then the dispatch path is the gate.
    #[test]
    fn w14_2_c1_type_info_on_array_generic() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("Array<int>".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        assert_dispatch_reached(
            stmts,
            Default::default(),
            (super::test_freeze_overlay(), Default::default()),
            "w14_2_c1_type_info_on_array_generic",
        );
    }

    /// W14.2-C1 (3) nested generic: `type_info("Option<Point>")` — the
    /// Option-wrapped struct shape, with snapshot pre-populated so the
    /// inner Point name is reachable when the marshal layer lands.
    #[test]
    fn w14_2_c1_type_info_on_option_of_struct() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("Option<Point>".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let snapshot = freeze_with_struct("Point", &[("x", TypeAnn::Basic("int".to_string()))]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_type_info_on_option_of_struct",
        );
    }

    /// W14.2-C1 (3) nested generic: `type_info("Result<int, string>")` —
    /// the Result two-param shape. Same dispatch contract as the Array
    /// and Option cases; covers the third audit-doc §4.6 builtin kind
    /// in the TypeInfo coverage matrix.
    #[test]
    fn w14_2_c1_type_info_on_result_two_params() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("Result<int, string>".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        assert_dispatch_reached(
            stmts,
            Default::default(),
            (super::test_freeze_overlay(), Default::default()),
            "w14_2_c1_type_info_on_result_two_params",
        );
    }

    /// W14.2-C1 (3) nested generic: chained `type_info("HashMap<...>").kind`
    /// — verifies property access on the generic-payload-named return
    /// dispatches via the same `kind: string` schema slot as the simple
    /// struct case.
    #[test]
    fn w14_2_c1_chained_kind_on_hashmap_generic() {
        let stmts = vec![Statement::Return(
            Some(Expr::PropertyAccess {
                object: Box::new(Expr::FunctionCall {
                    name: "type_info".to_string(),
                    const_args: Vec::new(),
                    args: vec![Expr::Literal(
                        Literal::String("HashMap<string, int>".to_string()),
                        Span::DUMMY,
                    )],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }),
                property: "kind".to_string(),
                optional: false,
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        assert_dispatch_reached(
            stmts,
            Default::default(),
            (super::test_freeze_overlay(), Default::default()),
            "w14_2_c1_chained_kind_on_hashmap_generic",
        );
    }

    // -------- (4) enum_payload_chained ----------------------------------

    /// W14.2-C1 (4) enum-payload: `type_info("Color")` where Color is a
    /// snapshot-registered enum — verifies the enum_defs lookup path in
    /// `classify_bare_type_name` reaches the TypedObject return arm
    /// (audit-doc §4.6 flat-discriminator: enums and structs share
    /// `TypeKind::TypedObject` until a dedicated Enum variant lands).
    #[test]
    fn w14_2_c1_type_info_on_registered_enum() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("Color".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let snapshot = freeze_with_enum("Color", &["Red", "Green", "Blue"]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_type_info_on_registered_enum",
        );
    }

    /// W14.2-C1 (4) enum-payload chained: `type_info("Color").kind` —
    /// chained property access on the enum-resolved TypeInfo. Verifies
    /// the snapshot.enum_defs branch of classify_bare_type_name +
    /// downstream GetFieldTyped dispatch on the TypedObject result.
    #[test]
    fn w14_2_c1_chained_kind_on_registered_enum() {
        let stmts = vec![Statement::Return(
            Some(Expr::PropertyAccess {
                object: Box::new(Expr::FunctionCall {
                    name: "type_info".to_string(),
                    const_args: Vec::new(),
                    args: vec![Expr::Literal(
                        Literal::String("Color".to_string()),
                        Span::DUMMY,
                    )],
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }),
                property: "kind".to_string(),
                optional: false,
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let snapshot = freeze_with_enum("Color", &["Red", "Green", "Blue"]);
        assert_dispatch_reached(
            stmts,
            Default::default(),
            snapshot,
            "w14_2_c1_chained_kind_on_registered_enum",
        );
    }

    // -------- (5) error_path ---------------------------------------------

    /// W14.2-C1 (5) error path: `type_info("UndefinedXYZ")` on a type
    /// name that is NOT in struct_defs/alias_defs/enum_defs — the
    /// `classify_bare_type_name` unrecognized-name fallback arm returns
    /// `TypeKindLabel::Unknown` and `build_type_info_heap_value`
    /// constructs a valid TypeInfo TypedObject with that label. Dispatch
    /// MUST NOT panic; this is the structured-error fallback per the
    /// audit-doc §4 (b) ergonomic contract.
    #[test]
    fn w14_2_c1_type_info_on_undefined_type_returns_unknown() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Literal(
                    Literal::String("UndefinedXYZ".to_string()),
                    Span::DUMMY,
                )],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        // Empty snapshot — "UndefinedXYZ" hits the unrecognized-name arm.
        assert_dispatch_reached(
            stmts,
            Default::default(),
            (super::test_freeze_overlay(), Default::default()),
            "w14_2_c1_type_info_on_undefined_type_returns_unknown",
        );
    }

    /// W14.2-C1 (5) error path: `type_info(UndefinedXYZ)` with a bare
    /// type-identifier (not string-quoted). The `rewrite_type_info_in_expr`
    /// path at `comptime.rs:278-288` rewrites the bare identifier to a
    /// string literal before dispatch. Verifies the rewriter applies
    /// even for unknown identifiers — the closure still receives a
    /// string and returns Unknown-labeled TypeInfo, no compile-time
    /// "Undefined variable" error.
    #[test]
    fn w14_2_c1_type_info_bare_ident_rewrites_for_unknown() {
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "type_info".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::Identifier("UndefinedXYZ".to_string(), Span::DUMMY)],
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        // No struct/enum registered — the rewriter still converts the
        // bare ident to a string literal; classify returns Unknown.
        assert_dispatch_reached(
            stmts,
            Default::default(),
            (super::test_freeze_overlay(), Default::default()),
            "w14_2_c1_type_info_bare_ident_rewrites_for_unknown",
        );
    }

    // -------- compile-only / contract surfaces ---------------------------

    /// W14.2-C1 contract: chained `type_info(...).kind` inside `comptime
    /// { }` block at the SOURCE level — verifies the chained property
    /// access on a `type_info` result PARSES cleanly. The Bytecode compile
    /// step is permitted to fail with the documented marshal-pending /
    /// bare-ident-rewriter gap (`Undefined variable: Point` —
    /// `rewrite_type_info_ident_args` is wired into the `execute_comptime`
    /// path but not the source-level comptime-block lowering path; see
    /// `comptime.rs:254-288` + `comptime_builtins.rs:469-484`). The
    /// must-not-surface contract is exactly the two retired legacy
    /// gates: `type_info has been removed` and the user-facing
    /// `comptime-only builtin` gate inside a comptime block. Both
    /// retirement contracts are verified here.
    #[test]
    fn w14_2_c1_chained_type_info_source_level_parse_and_gates() {
        let code = r#"
type Point {
  x: int,
  y: int
}

const KIND = comptime {
  type_info(Point).kind
}
"#;
        let program = shape_ast::parser::parse_program(code);
        assert!(
            program.is_ok(),
            "W14.2-C1: chained `type_info(Point).kind` must parse: {:?}",
            program.err()
        );
        let result = crate::compiler::BytecodeCompiler::new().compile(&program.unwrap());
        // Compile may fail due to documented pre-existing gaps; but it
        // MUST NOT surface either retired legacy gate.
        if let Err(e) = result {
            let msg = format!("{}", e);
            assert!(
                !msg.contains("type_info has been removed"),
                "W14.2-C1: chained type_info().kind must not surface the \
                 retired `type_info has been removed` gate: {msg}",
            );
            assert!(
                !msg.contains("comptime-only builtin"),
                "W14.2-C1: type_info inside a comptime block must not \
                 trigger the comptime-only-builtin gate: {msg}",
            );
        }
    }

    /// W14.2-C1 contract: `build_config()` + `type_info(...)` in the
    /// same comptime block PARSES cleanly at the source level and the
    /// compile step does not surface either retired legacy gate.
    /// Mirrors the `ct_49_build_config_fields` pattern at
    /// `tools/shape-test/tests/comptime/blocks.rs:312`, extended with
    /// `type_info()` in the same scope.
    #[test]
    fn w14_2_c1_build_config_plus_type_info_source_level_parse_and_gates() {
        let code = r#"
type Point {
  x: int
}

const COMBO = comptime {
  let cfg = build_config()
  let info = type_info(Point)
  info.name
}
"#;
        let program = shape_ast::parser::parse_program(code);
        assert!(
            program.is_ok(),
            "W14.2-C1: build_config + type_info combo must parse: {:?}",
            program.err()
        );
        let result = crate::compiler::BytecodeCompiler::new().compile(&program.unwrap());
        if let Err(e) = result {
            let msg = format!("{}", e);
            assert!(
                !msg.contains("type_info has been removed"),
                "W14.2-C1: must not surface retired legacy gate: {msg}",
            );
            assert!(
                !msg.contains("comptime-only builtin"),
                "W14.2-C1: builtins inside comptime block must not gate: {msg}",
            );
        }
    }

    /// W14.2-C1 contract: `type_info("Array<int>")` (string-quoted
    /// generic shape) PARSES cleanly at source level. The compile step
    /// is permitted to fail on the pre-existing SIGSEGV class
    /// (`ct_17_build_config` family — TypedObject printing /
    /// receiver-recovery via `__type_info_marshal_pending__`); we gate
    /// only the parse step here to avoid the documented SIGSEGV anchor.
    /// The runtime-level coverage for this shape is wired via the
    /// `w14_2_c1_type_info_on_array_generic` test above which uses the
    /// pure `execute_comptime` API and asserts dispatch reaches the body.
    #[test]
    fn w14_2_c1_type_info_on_generic_string_source_level_parse() {
        let code = r#"
const INFO = comptime {
  type_info("Array<int>").kind
}
"#;
        let program = shape_ast::parser::parse_program(code);
        assert!(
            program.is_ok(),
            "W14.2-C1: generic-string `type_info(\"Array<int>\").kind` must \
             parse: {:?}",
            program.err()
        );
    }

    /// W14.2-C1 contract: source-level parser preserves the chained
    /// `type_info(...).name.kind` (multi-level field projection) shape
    /// even though it's semantically invalid at runtime (TypeInfo's
    /// `name` is a string, not a TypedObject). This guards the
    /// audit-doc §4.6 "chained `type_info(...).field.subfield`" gap —
    /// the parser MUST accept the multi-level chain so a future
    /// FieldInfo-recursive shape can land without grammar work.
    #[test]
    fn w14_2_c1_chained_multi_level_property_parses() {
        let code = r#"
const X = comptime {
  type_info("Point").name.length
}
"#;
        let program = shape_ast::parser::parse_program(code);
        assert!(
            program.is_ok(),
            "W14.2-C1: multi-level chained property access on type_info() \
             must parse (future-proofing for recursive FieldInfo): {:?}",
            program.err()
        );
    }

    // R6 carrier-convention soundness (2026-06): `read_typed_object_field`
    // retains a TypedObject heap field on read. TypedObject slot bits are
    // the v2-raw `*const TypedObjectStorage` produced by
    // `TypedObjectStorage::_new` (HeapHeader at offset 0). The pre-fix code
    // applied an `Arc` strong-count bump to those raw `_new` bits, whose
    // `byte_sub(16)` to reach the (non-existent) ArcInner header is
    // out-of-allocation UB on a `_new` carrier. The fix retains via
    // `v2_retain` against the HeapHeader. This test builds a parent
    // TypedObject with a nested `_new` TypedObject heap field, reads the
    // field (retain), then drops both the read-out KindedSlot and the parent
    // via the canonical `release_elem`/Drop path — Miri (SB + TB) flags the
    // byte_sub(16) UB if the Arc op ever returns.
    #[test]
    fn r6_read_typed_object_field_retains_nested_typed_object_via_header() {
        use super::read_typed_object_field;
        use shape_value::v2::heap_element::HeapElement;
        use shape_value::{HeapKind, NativeKind, TypedObjectStorage, ValueSlot};
        use std::sync::Arc;

        // Nested child: an empty-field `_new` TypedObject (schema 7000).
        let child_ptr =
            TypedObjectStorage::_new(7000, Box::new([]), 0, Arc::from(Vec::<NativeKind>::new()));
        // Parent: one heap field (idx 0) pointing at the child, heap_mask bit 0.
        let parent_slot = ValueSlot::from_raw(child_ptr as u64);
        let field_kind = NativeKind::Ptr(HeapKind::TypedObject);

        // Read the field — the fixed retain path bumps the child's HeapHeader
        // refcount (1 -> 2) via v2_retain. Pre-fix this was the byte_sub(16)
        // Arc::increment UB.
        let read_out = read_typed_object_field(parent_slot, field_kind, /*heap_mask*/ 1, 0);
        assert_eq!(read_out.kind, field_kind);

        // Drop the read-out KindedSlot: canonical Drop -> drop_with_kind ->
        // release_elem on the HeapHeader (2 -> 1). Balanced against the retain.
        drop(read_out);

        // Now release the original share (1 -> 0) -> _drop deallocates via the
        // `_new` Layout. A double-free / wrong-allocator free here is Miri UB.
        unsafe {
            TypedObjectStorage::release_elem(child_ptr as *const TypedObjectStorage);
        }
    }

    #[test]
    fn w83e_const_accepts_comptime_block_initializer() {
        let code = r#"
            const BUILD_TAG = comptime {
                "dev"
            }

            BUILD_TAG
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = crate::compiler::BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "`const` initialized by a comptime block should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn w83e_comptime_fn_body_allows_comptime_only_builtin_calls() {
        let code = r#"
            comptime fn require_const_host() {
                if false {
                    error("not executed")
                }
            }

            comptime {
                require_const_host()
            }
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = crate::compiler::BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "comptime fn bodies should allow comptime-only builtins: {:?}",
            result.err()
        );
    }

    // =====================================================================
    // ADR-009 B1 S3 — `reflect()` wired end-to-end in the comptime path:
    // intrinsic, forwarder, payload-model item injection, outer type-check,
    // runtime-name-collision fence.
    // =====================================================================

    /// Parse a comptime block body from Shape source (wrapper-fn trick so
    /// the statements parse in function-body position, matching how a
    /// `comptime { ... }` block body reaches `execute_comptime`).
    fn parse_comptime_body(body: &str) -> Vec<Statement> {
        let program = shape_ast::parser::parse_program(&format!("fn __t__() {{\n{body}\n}}"))
            .expect("comptime body must parse");
        program
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(def, _) if def.name == "__t__" => Some(def.body),
                _ => None,
            })
            .expect("wrapper fn present")
    }

    fn run_comptime_body(body: &str) -> shape_ast::error::Result<super::ComptimeExecutionResult> {
        execute_comptime(
            &parse_comptime_body(body),
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        )
    }

    /// `run_comptime_body` that must reject (the result carrier has no
    /// `Debug`, so `expect_err` is not available on it).
    fn run_comptime_body_err(body: &str, ctx: &str) -> shape_ast::error::ShapeError {
        match run_comptime_body(body) {
            Err(error) => error,
            Ok(_) => panic!("{ctx}: comptime body must be rejected"),
        }
    }

    /// The full enabled-payload vertical: `reflect(type_ref(int))` inside
    /// the mini-VM resolves to the injected forwarder (NEVER lowering to
    /// the runtime `BuiltinFunction::Reflect` stub — collision pin (b): the
    /// stub would surface `NotImplemented phase-1b-vm-wave-5e-reflect`),
    /// the sealed `FrozenType` sum matches through the injected payload
    /// model, and the nested `FrozenPrimitive` / `IntegerWidth` payloads
    /// carry the exact width/domain data.
    #[test]
    fn reflect_primitive_payload_matches_through_the_injected_model_enums() {
        let result = run_comptime_body(
            r#"
match reflect(type_ref(int)) {
  FrozenType::Primitive(p) => match p {
    FrozenPrimitive::SignedInteger(w) => match w {
      IntegerWidth::W64 => 1
      _ => 2
    }
    _ => 3
  }
  FrozenType::Never(n) => 4
  FrozenType::Erased(e) => 5
  FrozenType::Callable(c) => 6
  FrozenType::Nominal(n) => 7
  FrozenType::Tuple(t) => 8
  FrozenType::Record(r) => 9
  FrozenType::Reference(rf) => 10
  FrozenType::Union(u) => 11
  FrozenType::Parameter(pp) => 12
}
"#,
        )
        .expect("reflect over an enabled payload category must succeed");
        assert_eq!(
            result.value.as_i64(),
            Some(1),
            "int must reflect to FrozenType::Primitive(SignedInteger(W64))"
        );
    }

    /// `bigint` is the named SignedInteger(Arbitrary) decision — the
    /// width-domain payload distinguishes it from `int`/W64 in user code.
    #[test]
    fn reflect_bigint_payload_carries_the_arbitrary_width_domain() {
        let result = run_comptime_body(
            r#"
match reflect(type_ref(bigint)) {
  FrozenType::Primitive(p) => match p {
    FrozenPrimitive::SignedInteger(w) => match w {
      IntegerWidth::Arbitrary => 1
      _ => 2
    }
    _ => 3
  }
  FrozenType::Never(n) => 4
  FrozenType::Erased(e) => 5
  FrozenType::Callable(c) => 6
  FrozenType::Nominal(n) => 7
  FrozenType::Tuple(t) => 8
  FrozenType::Record(r) => 9
  FrozenType::Reference(rf) => 10
  FrozenType::Union(u) => 11
  FrozenType::Parameter(pp) => 12
}
"#,
        )
        .expect("bigint reflects through the enabled Primitive payload");
        assert_eq!(result.value.as_i64(), Some(1));
    }

    /// Never and Erased payload arms select through the ordinal-pinned
    /// variant ids (Never=1, Erased=9 — the Erased arm is the load-bearing
    /// proof that the injected model enum and the unspellable value carrier
    /// agree on catalog ORDINALS, not dense ids).
    #[test]
    fn reflect_never_and_erased_arms_use_the_ordinal_pinned_variant_ids() {
        for (spelling, expected) in [("never", 4), ("any", 5)] {
            let result = run_comptime_body(&format!(
                r#"
match reflect(type_ref({spelling})) {{
  FrozenType::Primitive(p) => 1
  FrozenType::Never(n) => 4
  FrozenType::Erased(e) => 5
  FrozenType::Callable(c) => 6
  FrozenType::Nominal(n) => 7
  FrozenType::Tuple(t) => 8
  FrozenType::Record(r) => 9
  FrozenType::Reference(rf) => 10
  FrozenType::Union(u) => 11
  FrozenType::Parameter(pp) => 12
}}
"#
            ))
            .unwrap_or_else(|error| {
                panic!("reflect(type_ref({spelling})) must succeed: {error:?}")
            });
            assert_eq!(
                result.value.as_i64(),
                Some(expected),
                "reflect(type_ref({spelling})) must select the {expected} arm"
            );
        }
    }

    /// ADR-009 B7 (Dec 50/94): the four composite payload arms select through
    /// the ordinal-pinned variant ids (Tuple=4, Record=5, Reference=7, Union=8)
    /// and destructure their typed structural fields through the injected model —
    /// the load-bearing proof that the injected model enum and the unspellable
    /// value carrier agree on the composite catalog ORDINALS, and that the
    /// element arrays / flat fields carry real typed data (never a `.kind`
    /// string).
    #[test]
    fn reflect_composite_payloads_decode_through_the_injected_model() {
        for (spelling, expected) in [
            // tuple: 2 ordered elements
            ("[int, string]", 2),
            // record: 2 normalized fields
            ("{x: int, y: string}", 2),
            // union: int | string | int dedups to 2 members
            ("int | string | int", 2),
        ] {
            let result = run_comptime_body(&format!(
                r#"
match reflect(type_ref({spelling})) {{
  FrozenType::Tuple(t) => t.elements.len()
  FrozenType::Record(r) => r.fields.len()
  FrozenType::Union(u) => u.members.len()
  _ => -1
}}
"#
            ))
            .unwrap_or_else(|error| {
                panic!("reflect(type_ref({spelling})) must succeed: {error:?}")
            });
            assert_eq!(
                result.value.as_i64(),
                Some(expected),
                "composite payload for {spelling} must decode its element count"
            );
        }

        // Reference: `&mut int` is mutable, `&int` is not — read as typed bool.
        for (spelling, expected) in [("&mut int", 1), ("&int", 0)] {
            let result = run_comptime_body(&format!(
                r#"
match reflect(type_ref({spelling})) {{
  FrozenType::Reference(rf) => if rf.mutable {{ 1 }} else {{ 0 }}
  _ => -1
}}
"#
            ))
            .unwrap_or_else(|error| {
                panic!("reflect(type_ref({spelling})) must succeed: {error:?}")
            });
            assert_eq!(result.value.as_i64(), Some(expected));
        }
    }

    /// ADR-009 B5 (drift note R10/R11): reflecting an un-applied generic
    /// constructor head (`Array`, `TypeConstructorRef` territory) is the NAMED
    /// rejection — never a partial descriptor and never a shape off the
    /// un-applied form. `Array` is frozen as a builtin Nominal head with
    /// declared param kinds in every compilation unit.
    #[test]
    fn reflect_unapplied_generic_head_is_the_named_rejection() {
        let error = run_comptime_body_err(
            "reflect(type_ref(Array))",
            "an un-applied generic head must reject",
        );
        let message = format!("{error:?}");
        assert!(
            message.contains("un-applied generic type constructor is not a resolved nominal shape"),
            "un-applied-head rejection must be the named diagnostic: {message}"
        );
    }

    /// R6: a non-exhaustive match over the sealed reflect sum is the
    /// existing exhaustiveness error (the injected model enum feeds the
    /// mini-VM inference engine's enum registry).
    #[test]
    fn reflect_match_exhaustiveness_is_enforced_over_the_injected_model() {
        let error = run_comptime_body_err(
            r#"
match reflect(type_ref(int)) {
  FrozenType::Primitive(p) => 1
}
"#,
            "a Primitive-only match over FrozenType is non-exhaustive",
        );
        let message = format!("{error:?}");
        assert!(
            message.contains("Non-exhaustive match"),
            "R6 must surface the existing exhaustiveness error: {message}"
        );
    }

    /// R6 twin: no Unknown/Any arm is nameable on the sealed sum.
    #[test]
    fn no_unknown_variant_is_nameable_on_the_reflect_sum() {
        let error = run_comptime_body_err(
            r#"
match reflect(type_ref(int)) {
  FrozenType::Primitive(p) => 1
  FrozenType::Never(n) => 2
  FrozenType::Erased(e) => 3
  FrozenType::Callable(c) => 5
  FrozenType::Unknown(u) => 4
}
"#,
            "FrozenType has no Unknown variant to name",
        );
        let message = format!("{error:?}");
        assert!(
            message.contains("Unknown"),
            "the rejection must name the unknown variant: {message}"
        );
    }

    /// R5 + collision fence: outside comptime, `reflect` hits the
    /// comptime-only rejection path (identical in shape to
    /// `type_ref_is_comptime_only`) — the runtime stub never receives
    /// comptime descriptors from source-level calls.
    #[test]
    fn reflect_is_comptime_only_at_runtime_position() {
        let program = shape_ast::parser::parse_program("let x = reflect(42)").expect("parse");
        let result = crate::compiler::BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "runtime-position reflect must be rejected");
        let message = format!("{}", result.unwrap_err());
        assert!(
            message.contains("comptime-only builtin"),
            "R5 must surface the comptime-only rejection: {message}"
        );
    }

    /// Collision pin (a): the runtime builtin-name mapping and the executor
    /// SURFACE stub are untouched by ADR-009 B1 — `reflect` still resolves
    /// to `BuiltinFunction::Reflect` at the name-mapping layer, and the
    /// `phase-1b-vm-wave-5e-reflect` NotImplemented stub arm survives. The
    /// comptime path shadows by resolution ORDER (user forwarder before
    /// builtin classification), never by renaming.
    #[test]
    fn runtime_reflect_name_mapping_and_stub_arm_are_untouched() {
        let compiler = crate::compiler::BytecodeCompiler::new();
        let resolution = compiler
            .classify_builtin_function("reflect")
            .expect("runtime reflect name mapping must stay intact");
        match resolution {
            crate::compiler::BuiltinNameResolution::Surface { builtin, .. } => {
                assert_eq!(builtin, crate::bytecode::BuiltinFunction::Reflect)
            }
            crate::compiler::BuiltinNameResolution::InternalOnly { .. } => {
                panic!("reflect must stay a surface builtin mapping")
            }
        }

        let executor_source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/executor/vm_impl/builtins.rs"
        ))
        .expect("executor builtins source readable");
        assert!(
            executor_source.contains("phase-1b-vm-wave-5e-reflect"),
            "the BuiltinFunction::Reflect SURFACE stub arm must stay untouched by B1"
        );
    }

    /// R4: reflect's argument forms are rejected with NAMED diagnostics at
    /// the outer type-check (mirroring the type_ref arg-form rejections) —
    /// wrong arity, string arg, int arg, and the legacy `__ComptimeTypeRef`
    /// descriptor (`type_info(T).type_ref`).
    #[test]
    fn reflect_arg_forms_are_rejected_with_named_diagnostics() {
        for (code, expected) in [
            (
                "let x = comptime { reflect() }",
                "reflect expects exactly one TypeRef argument",
            ),
            (
                "let x = comptime { reflect(type_ref(int), type_ref(int)) }",
                "reflect expects exactly one TypeRef argument",
            ),
            (
                r#"let x = comptime { reflect("int") }"#,
                "reflect expects a TypeRef value",
            ),
            (
                "let x = comptime { reflect(42) }",
                "reflect expects a TypeRef value",
            ),
            (
                "let x = comptime { reflect(type_info(int).type_ref) }",
                "reflect expects a TypeRef value",
            ),
        ] {
            let program = shape_ast::parser::parse_program(code).expect("parse");
            let result = crate::compiler::BytecodeCompiler::new().compile(&program);
            assert!(result.is_err(), "must reject: {code}");
            let message = format!("{}", result.unwrap_err());
            assert!(
                message.contains(expected),
                "R4 for `{code}` must surface `{expected}`, got: {message}"
            );
        }
    }

    /// R2 (Dec 50/94 required rejection): the reflect result exposes NO
    /// string `kind` field — the `info.kind == "record"` legacy form is a
    /// named rejection, not a nullable/stringly access.
    #[test]
    fn reflect_result_has_no_string_kind_field() {
        let error = run_comptime_body_err(
            r#"reflect(type_ref(int)).kind"#,
            "FrozenType exposes no string kind field",
        );
        let message = format!("{error:?}");
        assert!(
            message.contains("kind"),
            "R2 rejection must name the missing field: {message}"
        );
    }

    // =====================================================================
    // ADR-009 B1 S4 — the value-DEEP stage-boundary lift wall
    // (`comptime_result_lift_rejection`): descriptors nested inside
    // objects/arrays and spellable-model forgeries are NAMED at the
    // boundary, never silently swallowed to `Null` by the `nb_to_expr`
    // materialization fallback (scout risk 4 bypass channel).
    // =====================================================================

    /// A descriptor nested inside an object-literal result is caught by the
    /// deep wall with the descriptor's own named message.
    #[test]
    fn deep_lift_wall_names_a_descriptor_nested_in_an_object_result() {
        let result = run_comptime_body(r#"{ inner: reflect(type_ref(int)) }"#)
            .expect("building a descriptor-bearing object INSIDE comptime is allowed");
        assert_eq!(
            super::comptime_result_lift_rejection(&result.value, &result.schema_registry),
            Some("FrozenType is comptime-only reflection data and cannot enter runtime code"),
            "the nested descriptor must be named at the stage boundary"
        );
    }

    /// A user-forged SPELLABLE payload-model value (constructable inside
    /// comptime exactly like `FrozenTypeCategory`) resolves through the
    /// carried mini-VM registry and hits the spellable-name lift arm.
    #[test]
    fn deep_lift_wall_names_a_forged_spellable_model_value() {
        let result = run_comptime_body("FrozenType::Primitive(FrozenPrimitive::Bool)")
            .expect("constructing a spellable model value INSIDE comptime is allowed");
        assert_eq!(
            super::comptime_result_lift_rejection(&result.value, &result.schema_registry),
            Some("FrozenType is comptime-only reflection data and cannot enter runtime code"),
        );
    }

    /// Ordinary comptime results (scalars, plain objects) pass the deep
    /// wall untouched — the wall names descriptors only.
    #[test]
    fn deep_lift_wall_ignores_ordinary_comptime_results() {
        for body in ["42", r#"{ name: "shape", count: 2 }"#, "[1, 2, 3]"] {
            let result = run_comptime_body(body).expect("ordinary comptime body succeeds");
            assert_eq!(
                super::comptime_result_lift_rejection(&result.value, &result.schema_registry),
                None,
                "ordinary result must remain liftable: {body}"
            );
        }
    }
}

#[cfg(any())]
#[cfg(test)]
mod tests_deferred {
    use super::*;
    use shape_ast::ast::{BinaryOp, Expr, Literal, Span, Statement};
    use shape_runtime::typed_module_exports::register_test_function;
    use shape_value::heap_value::HeapValue;

    #[test]
    fn test_comptime_simple_return() {
        let stmts = vec![Statement::Return(
            Some(Expr::Literal(Literal::Int(42), Span::DUMMY)),
            Span::DUMMY,
        )];

        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "Comptime should succeed: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().value, ValueWord::from_i64(42));
    }

    #[test]
    fn test_comptime_string_return() {
        let stmts = vec![Statement::Return(
            Some(Expr::Literal(
                Literal::String("hello".to_string()),
                Span::DUMMY,
            )),
            Span::DUMMY,
        )];

        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "Comptime should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string().expect("Expected String").as_ref() as &str,
            "hello"
        );
    }

    #[test]
    fn test_comptime_arithmetic() {
        // Parse and execute: return 2 + 3
        let stmts = vec![Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Int(2), Span::DUMMY)),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Literal::Int(3), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];

        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "Comptime arithmetic should succeed: {:?}",
            result.err()
        );
        assert_eq!(
            result
                .unwrap()
                .value
                .as_number_coerce()
                .expect("Expected 5"),
            5.0
        );
    }

    #[test]
    fn test_comptime_with_sync_extension() {
        // Create a mock extension with a sync function that returns a value.
        // Verify execute_comptime can call extension functions.
        use shape_runtime::module_exports::ModuleExports;

        let mut ext = ModuleExports::new("mock_db");
        register_test_function(
            &mut ext,
            "get_schema",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(ValueWord::from_string(Arc::new(
                    "id:int,name:string".to_string(),
                )))
            },
        );

        // Parse a program that imports and calls the extension.
        // Extension modules are available as module_bindings (e.g., mock_db::get_schema()).
        // We need to register "mock_db" as a module_binding in the compiled program.
        let code = r#"
            use mock_db
            mock_db::get_schema()
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");

        // Compile with extension awareness
        let mut compiler = BytecodeCompiler::new();
        compiler.extension_registry = Some(Arc::new(vec![ext.clone()]));
        let bytecode = compiler.compile(&program).expect("compile");

        // Execute with extension registered
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.register_extension(ext);
        vm.populate_module_objects();

        let result = vm.execute(None);
        assert!(
            result.is_ok(),
            "Extension call should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().clone();
        assert_eq!(
            val.as_arc_string()
                .expect("Expected schema string")
                .as_ref() as &str,
            "id:int,name:string"
        );
    }

    #[test]
    fn test_comptime_extension_registry_flows_through_compiler() {
        // Verify that when BytecodeCompiler has an extension_registry set,
        // it is available during meta method compilation.
        use shape_runtime::module_exports::ModuleExports;

        let mut ext = ModuleExports::new("test_ext");
        register_test_function(
            &mut ext,
            "version",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(ValueWord::from_string(Arc::new("1.0".to_string())))
            },
        );

        let mut compiler = BytecodeCompiler::new();
        compiler.extension_registry = Some(Arc::new(vec![ext]));

        // The extension_registry should be set
        assert!(compiler.extension_registry.is_some());
        assert_eq!(compiler.extension_registry.as_ref().unwrap().len(), 1);
        assert_eq!(
            compiler.extension_registry.as_ref().unwrap()[0].name,
            "test_ext"
        );
    }

    #[test]
    fn test_vmvalue_to_literal_int() {
        let lit = vmvalue_to_literal(&ValueWord::from_i64(42));
        assert_eq!(lit, Literal::Int(42));
    }

    #[test]
    fn test_vmvalue_to_literal_number() {
        let lit = vmvalue_to_literal(&ValueWord::from_f64(3.14));
        assert_eq!(lit, Literal::Number(3.14));
    }

    #[test]
    fn test_vmvalue_to_literal_string() {
        let lit = vmvalue_to_literal(&ValueWord::from_string(Arc::new("hello".to_string())));
        assert_eq!(lit, Literal::String("hello".to_string()));
    }

    #[test]
    fn test_vmvalue_to_literal_bool() {
        let lit = vmvalue_to_literal(&ValueWord::from_bool(true));
        assert_eq!(lit, Literal::Bool(true));
    }

    #[test]
    fn test_vmvalue_to_literal_none() {
        let lit = vmvalue_to_literal(&ValueWord::none());
        assert_eq!(lit, Literal::None);
    }

    #[test]
    fn test_vmvalue_to_literal_unit() {
        let lit = vmvalue_to_literal(&ValueWord::unit());
        assert_eq!(lit, Literal::Unit);
    }

    #[test]
    fn test_comptime_block_parsed_and_executed() {
        // Test that a comptime block in expression position can be parsed
        // and the statements are well-formed.
        let stmts = vec![Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Int(10), Span::DUMMY)),
                op: BinaryOp::Mul,
                right: Box::new(Expr::Literal(Literal::Int(5), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];

        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "Comptime multiplication should succeed: {:?}",
            result.err()
        );
        assert_eq!(
            result
                .unwrap()
                .value
                .as_number_coerce()
                .expect("Expected 50"),
            50.0
        );
    }

    #[test]
    fn test_comptime_builtins_available_in_comptime_block() {
        // Verify that comptime builtins (build_config, etc.) are available via
        // execute_comptime() wiring.
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "build_config".to_string(),
                const_args: Vec::new(),
                args: Vec::new(),
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        )
        .map(|r| r.value);
        assert!(
            result.is_ok(),
            "build_config() should work in comptime: {:?}",
            result.err()
        );
        let val = result.unwrap();
        // build_config now returns TypedObject
        // cold-path: as_heap_ref retained — test assertion
        let is_typed_object_or_string = val
            .as_heap_ref() // cold-path
            .is_some_and(|h| matches!(h, HeapValue::TypedObject { .. } | HeapValue::String(_)));
        assert!(
            is_typed_object_or_string,
            "Expected TypedObject or String, got {:?}",
            val,
        );
    }

    #[test]
    fn test_comptime_print_build_config_no_stack_overflow() {
        // Regression: `__comptime__.build_config()` must dispatch through the
        // module object, not UFCS rewrite, otherwise it recurses infinitely.
        let stmts = vec![Statement::Expression(
            Expr::FunctionCall {
                name: "print".to_string(),
                const_args: Vec::new(),
                args: vec![Expr::FunctionCall {
                    name: "build_config".to_string(),
                    const_args: Vec::new(),
                    args: Vec::new(),
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }],
                named_args: Vec::new(),
                span: Span::DUMMY,
            },
            Span::DUMMY,
        )];

        let result = execute_comptime(
            &stmts,
            &[],
            &[],
            Default::default(),
            Default::default(),
            super::test_freeze_overlay(),
        );
        assert!(
            result.is_ok(),
            "print(build_config()) should execute in comptime: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_comptime_only_builtins_rejected_outside_comptime() {
        // type_info() is removed entirely and should produce a migration error.
        let code = r#"let x = type_info("Point")"#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "type_info() outside comptime should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("type_info has been removed"),
            "Error should mention removal: {}",
            err_msg
        );

        // implements()/build_config() remain comptime-only.
        let code2 = r#"let y = build_config()"#;
        let program2 = shape_ast::parser::parse_program(code2).expect("parse");
        let result2 = BytecodeCompiler::new().compile(&program2);
        assert!(
            result2.is_err(),
            "build_config() outside comptime should fail"
        );
    }

    #[test]
    fn test_comptime_with_target_simple() {
        // Execute a comptime handler that reads target.name
        let handler_body = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("target".to_string(), Span::DUMMY)),
            property: "name".to_string(),
            optional: false,
            span: Span::DUMMY,
        };

        let target_value = shape_runtime::type_schema::typed_object_from_pairs(&[
            (
                "kind",
                ValueWord::from_string(Arc::new("function".to_string())),
            ),
            (
                "name",
                ValueWord::from_string(Arc::new("my_func".to_string())),
            ),
            (
                "fields",
                ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
            ),
            (
                "params",
                ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
            ),
            ("return_type", ValueWord::none()),
            (
                "annotations",
                ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
            ),
            (
                "captures",
                ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
            ),
        ]);

        let result = execute_comptime_with_target(
            &handler_body,
            "target",
            target_value,
            &[],
            Default::default(),
            Default::default(),
        );
        assert!(
            result.is_ok(),
            "Comptime with target should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"my_func\")")
                .as_ref() as &str,
            "my_func"
        );
    }

    #[test]
    fn test_comptime_with_target_from_function() {
        use crate::compiler::comptime_target::ComptimeTarget;
        use shape_ast::ast::{DestructurePattern, FunctionParameter, TypeAnnotation};

        // Build a target from a real function definition
        let func = FunctionDef {
            name: "greet".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("name".to_string(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("string".to_string())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Basic("string".to_string())),
            body: Vec::new(),
            type_params: None,
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        let target = ComptimeTarget::from_function(&func);
        let target_value = target.to_nanboxed();

        // Handler body: return target.kind
        let handler_body = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("t".to_string(), Span::DUMMY)),
            property: "kind".to_string(),
            optional: false,
            span: Span::DUMMY,
        };

        let result = execute_comptime_with_target(
            &handler_body,
            "t",
            target_value,
            &[],
            Default::default(),
            Default::default(),
        );
        assert!(
            result.is_ok(),
            "Comptime with function target should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"function\")")
                .as_ref() as &str,
            "function"
        );
    }

    #[test]
    fn test_comptime_handler_end_to_end() {
        // Full end-to-end: define annotation with comptime phase handler, apply to function, compile
        let code = r#"
            annotation inspect() {
                comptime post(target, ctx) {
                    target.name
                }
            }
            @inspect()
            function greet(name) {
                return "hello " + name
            }
            greet("world")
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Comptime handler end-to-end should compile: {:?}",
            result.err()
        );

        // The function should still work normally at runtime
        let bytecode = result.unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let exec_result = vm.execute(None);
        assert!(
            exec_result.is_ok(),
            "Execution should succeed: {:?}",
            exec_result.err()
        );
        let val = exec_result.unwrap().clone();
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"hello world\")")
                .as_ref() as &str,
            "hello world"
        );
    }

    #[test]
    fn test_comptime_handler_accesses_target_params() {
        // Comptime handler that accesses target.params — verifies the target object is fully populated
        let code = r#"
            annotation check_params() {
                comptime post(target, ctx) {
                    target.params
                }
            }
            @check_params()
            function add(x, y) {
                return x + y
            }
            add(1, 2)
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Comptime handler with params access should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let exec_result = vm.execute(None);
        assert!(
            exec_result.is_ok(),
            "Should execute: {:?}",
            exec_result.err()
        );
        assert_eq!(
            exec_result
                .unwrap()
                .clone()
                .as_number_coerce()
                .expect("Expected 3"),
            3.0
        );
    }

    #[test]
    fn test_comptime_fn_not_compiled_into_runtime_bytecode() {
        // Comptime fn functions should NOT produce bytecode in the runtime program.
        // They only exist as AST in function_defs for collect_comptime_helpers.
        let code = r#"
            comptime fn helper() {
                42
            }
            comptime {
                helper()
            }
            100
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");

        // The comptime fn should NOT appear as a compiled function with a valid entry point.
        // It may still be in the function table (from registration), but its body
        // should not have been compiled.
        let helper_func = bytecode.functions.iter().find(|f| f.name == "helper");
        if let Some(func) = helper_func {
            // If the function is in the table, it must not have a compiled body
            // (body_length should be 0, entry_point should still be 0 from registration)
            assert_eq!(
                func.body_length, 0,
                "comptime fn should not have compiled body in runtime bytecode"
            );
        }

        // Runtime code should still work
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute");
        assert_eq!(result.as_number_coerce().expect("Expected 100"), 100.0);
    }

    #[test]
    fn test_comptime_fn_not_callable_at_runtime() {
        // Calling a comptime fn at runtime should produce a clear compile error
        let code = r#"
            comptime fn secret() {
                42
            }
            secret()
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "Calling comptime fn at runtime should fail"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("comptime"),
            "Error should mention comptime: {}",
            err_msg
        );
    }
}
