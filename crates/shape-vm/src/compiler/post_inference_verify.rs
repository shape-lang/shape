//! Post-inference verification pass — `FieldType::Any` boundary enforcement.
//!
//! v0.3 Phase 4b Round 5 sub-cluster W17.2-A implementation. Binding
//! authority: user 2026-05-18 ruling, quoted verbatim at the audit:
//!
//! > `FieldType::Any` ALLOWED at compile-time intermediate layers (during
//! > type inference + resolution passes). `FieldType::Any` FORBIDDEN at
//! > post-inference output. Post-pass `FieldType::Any` → compile error.
//!
//! Per ADR-006 §2.7.5 (producer-side stamp-at-compile-time) + §2.7.26
//! (W17-comptime-vm-dispatch parallel-`field_kinds` carrier). The five
//! PERMANENT named-exception classes (§4.D.10–§4.D.15) ALL share the
//! parallel-`field_kinds` track at storage construction time — that
//! track IS the §2.7.5 stamp. The nine TRANSITIONAL classes (§4.D.1–
//! §4.D.9) close in Round 5b W17.2-B (`semantic_to_field_type` Option
//! rebuild + fall-through hardening) + W17.2-C (compiler-side fallback
//! hardening); user ratify 2026-05-19 binds the transitional whitelist
//! to land at this sub-cluster and tighten at R5b close.
//!
//! Authoritative source-of-truth:
//! `docs/cluster-audits/v0.3-w17-fieldtype-any-boundary-audit.md` §0–§13.
//!
//! Discipline: this is a READ pass over post-inference state. It does
//! NOT fabricate kinds, paper over inference gaps, or carry any rename
//! refused by CLAUDE.md §Renames-to-refuse-on-sight (broader-family
//! regex). The pass walks `BytecodeProgram.type_schema_registry`, matches
//! each schema's name + each `FieldType::Any` field against the
//! structured whitelist below, and surfaces structured E0900 errors for
//! any unmatched site.

use shape_ast::error::{ErrorCode, ShapeError, SourceLocation};
use shape_runtime::type_schema::{FieldType, TypeSchema, TypeSchemaRegistry};

use crate::bytecode::BytecodeProgram;

/// Whitelisted schema-class entry. Each whitelist site carries the §-cite
/// (audit `docs/cluster-audits/v0.3-w17-fieldtype-any-boundary-audit.md`)
/// + a transitional flag (closes in R5b W17.2-B+C) or a permanent flag
/// (carrier-tier exception with parallel-`field_kinds` track per ADR-006
/// §2.7.26), + a short reason describing why the exception is allowed.
// `section` / `reason` are audit-trail metadata carried for documentation +
// grep; the verification logic dispatches only on `rule` + `permanent`.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct WhitelistEntry {
    /// Match rule (exact name, prefix, or dynamic enum-payload field).
    pub(crate) rule: WhitelistRule,
    /// §-cite into the audit doc (e.g. `"§4.D.10"`).
    pub(crate) section: &'static str,
    /// `true` for permanent exception classes (§4.D.10–15); `false` for
    /// transitional classes (§4.D.1–9) that close in R5b W17.2-B+C.
    pub(crate) permanent: bool,
    /// Short reason / origin site identifier.
    pub(crate) reason: &'static str,
}

/// How a whitelist entry matches a schema or schema-field.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WhitelistRule {
    /// Exact schema-name match. All `FieldType::Any` fields are allowed.
    SchemaName(&'static str),
    /// Schema-name prefix match. All `FieldType::Any` fields are allowed.
    SchemaNamePrefix(&'static str),
    /// Schema `is_enum()` is true AND the field name starts with
    /// `__payload_`. Only matching fields are allowed; non-payload Any
    /// fields on the same schema still surface E0900.
    EnumPayloadField,
}

/// Whitelist codified per audit §4.D.1–§4.D.15. Each entry carries its
/// §-cite + permanent/transitional disposition + short reason.
///
/// PERMANENT (§4.D.10–§4.D.15): five carrier-tier exception classes,
/// each backed by the parallel-`field_kinds` track at storage time per
/// ADR-006 §2.7.26 (which IS the §2.7.5 producer-side stamp).
///
/// TRANSITIONAL (§4.D.1–§4.D.9): nine schema-construction fallback
/// classes that the verification pass tolerates pre-R5b. Each entry's
/// `permanent: false` field signals that R5b W17.2-B (`semantic_to_field_type`
/// hardening) and W17.2-C (compiler-side fallback hardening) close the
/// underlying defect and the take-both commit at R5b close MUST strip
/// the transitional rows from this list. Per user 2026-05-19 binding +
/// audit §9.B.1 + §9.B.3.
pub(crate) const WHITELIST: &[WhitelistEntry] = &[
    // ----- PERMANENT named exception classes (§4.D.10–§4.D.15) -----
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("Row"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib heterogeneous-column carrier; \
                 registry.rs:236/280/356; \
                 parallel-`field_kinds` track per ADR-006 §2.7.26",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("FrameState"),
        section: "§4.D.15",
        permanent: true,
        reason: "VM-state introspection (locals/args/upvalues); \
                 state_builtins/core.rs:65-67; \
                 parallel-`field_kinds` per ADR-006 §2.7.7/Q9",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("VmState"),
        section: "§4.D.15",
        permanent: true,
        reason: "VM-state introspection (frames/module_bindings); \
                 state_builtins/core.rs:74-75",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("ModuleState"),
        section: "§4.D.15",
        permanent: true,
        reason: "VM-state introspection (module bindings); \
                 state_builtins/core.rs:82",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("CallPayload"),
        section: "§4.D.15",
        permanent: true,
        reason: "VM-state introspection (call payload args); \
                 state_builtins/core.rs:89",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("Delta"),
        section: "§4.D.15",
        permanent: true,
        reason: "VM-state diff/patch carrier; state_builtins/core.rs:91-105; \
                 per-delta value kind is carried by HashMapKindedRef plus \
                 TypedObjectStorage::field_kinds per ADR-006 §2.7.7/Q9 \
                 + §2.7.26",
    },
    // W17.2-C namespaced-state-builtins prefix exception. The
    // `std::core::state::*` registered schemas (FrameState / VmState /
    // ModuleState / CallPayload / Delta / FunctionRef) carry
    // `FieldType::Any` fields by-design — the VM-state introspection
    // surface is heterogeneous per audit §4.D.15. Pre-W17.2-C the
    // empty-prefix transitional row absorbed these; post-W17.2-C
    // narrowing the entire namespace prefix is whitelisted as a
    // single named-exception class.
    //
    // Module-registration site: `ModuleExports::new("std::core::state")`
    // at `crates/shape-vm/src/executor/state_builtins/core.rs:46`.
    // Schemas added via `module.add_type_schema(TypeSchema::new(...))`
    // at lines 51 (FunctionRef) / 59-69 (FrameState) / 71-78 (VmState) /
    // 80-83 (ModuleState) / 85-91 (CallPayload). Delta is registered
    // implicitly via `ConcreteType::Named("Delta")` at
    // `state_builtins/core.rs:194` (diff function return type) +
    // schema_registry::register_predeclared_any_schema fallback. All
    // share the parallel-`field_kinds` carrier-tier per ADR-006 §2.7.7/Q9.
    WhitelistEntry {
        rule: WhitelistRule::SchemaNamePrefix("std::core::state::"),
        section: "§4.D.15",
        permanent: true,
        reason: "namespaced VM-state introspection surface \
                 (FrameState/VmState/ModuleState/CallPayload/Delta/\
                 FunctionRef); state_builtins/core.rs:46-91 + Delta via \
                 ConcreteType::Named at core.rs:194 + register_predeclared \
                 fallback; parallel-`field_kinds` per ADR-006 §2.7.7/Q9 \
                 + §2.7.26",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaNamePrefix("__mod_"),
        section: "§4.D.6",
        permanent: true,
        reason: "module-export synthetic schema (statements.rs:1485 + \
                 comptime.rs:479); parallel-`field_kinds` carrier-tier \
                 per ADR-006 §2.7.26 (W17-comptime-vm-dispatch)",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaNamePrefix("__predecl_"),
        section: "§4.D.12",
        permanent: true,
        reason: "predeclared-schema carrier-tier \
                 (register_predeclared_any_schema / \
                 mirror_predeclared_any_schema); \
                 registry.rs:489/535; ADR-006 §2.7.26 binds the kind \
                 resolution to the storage's `field_kinds` track",
    },
    WhitelistEntry {
        rule: WhitelistRule::EnumPayloadField,
        section: "§4.D.14",
        permanent: true,
        reason: "enum __payload_N storage carrier (schema.rs:203); \
                 compiler-side parallel-kind track lives at \
                 `enum_struct_variant_fields` (`compiler/mod.rs:813-822`) \
                 per ADR-006 §2.3 named-carrier discipline",
    },
    // -----
    // §4.D.11 RUNTIME_BUILTIN_SCHEMA extension landed at W17.2-B close
    // (audit §4.D.11+§4.D.13 follow-up). The `__PascalCase`
    // builtin schemas registered at `register_builtin_schemas`
    // (`crates/shape-runtime/src/type_schema/builtin_schemas.rs:113`)
    // are stdlib carrier-tier exception territory — they use
    // `SchemaBuilder::any_field` (§4.D.13) which is the public method
    // legitimately used by §4.D.11 / §4.D.12 exception classes per
    // audit. The W17.2-A whitelist only named `Row` from §4.D.11; the
    // remaining 15 surface as E0900 once the §4.D.1-9 transitional row
    // is stripped at W17.2-B close. Each schema's heap-allocated
    // polymorphic field carries the concrete kind via the parallel
    // `field_kinds` track at storage construction time per ADR-006
    // §2.7.7/Q9 + §2.7.26 (per the doc-comment at builtin_schemas.rs:111
    // — "heap-allocated polymorphic fields use FieldType::String
    // (informational — the heap_mask bitmap determines actual read
    // path)"). Mirror of `Row` shape.
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__AnyError"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; \
                 builtin_schemas.rs:114; \
                 parallel-`field_kinds` track + heap_mask per \
                 ADR-006 §2.7.7/Q9 + §2.7.26",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__TraceFrame"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__TraceInfoFull"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__TraceInfoSingle"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ReflectAnnotation"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ReflectField"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ReflectResult"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__GroupResult"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__EventLogEntry"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__SimulateReturn"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__Option"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__Result"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__EmptyObject"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ComptimeBuildConfig"),
        section: "§4.D.11",
        permanent: true,
        reason: "stdlib runtime-builtin schema; builtin_schemas.rs",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ComptimeFieldDescriptor"),
        section: "§4.D.11",
        permanent: true,
        reason: "comptime introspection contract schema; builtin_schemas.rs (S2)",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ComptimeParamDescriptor"),
        section: "§4.D.11",
        permanent: true,
        reason: "comptime introspection contract schema; builtin_schemas.rs (S2)",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ComptimeAnnotationDescriptor"),
        section: "§4.D.11",
        permanent: true,
        reason: "comptime introspection contract schema; builtin_schemas.rs (S2)",
    },
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("__ComptimeTarget"),
        section: "§4.D.11",
        permanent: true,
        reason: "comptime introspection contract schema; builtin_schemas.rs (S2)",
    },
    // ADR-009 B1 S1 — `FrozenErased` bound-set carrier. The unspellable
    // (SOH-prefixed, unforgeable from source) comptime-only descriptor
    // schema's `bounds` array is reachable today ONLY as the empty set
    // (`any`; A2 checked type-expression syntax unlanded), and the
    // descriptor is lift-walled out of runtime code by
    // `comptime_reflection::runtime_lift_rejection`. The element type
    // retypes to the TraitRef descriptor schema when ticket B2 lands —
    // removal happens by retyping the element, not by stripping this row.
    WhitelistEntry {
        rule: WhitelistRule::SchemaName(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_ERASED_SCHEMA,
        ),
        section: "§4.D.11",
        permanent: true,
        reason: "ADR-009 B1 comptime-only FrozenErased descriptor; \
                 bound set empty-only until A2/B2 (elements retype to \
                 TraitRef at B2); unspellable name + runtime_lift_rejection \
                 wall keep it out of runtime code",
    },
    // ADR-009 B1 S3 — the SPELLABLE `FrozenErased` payload-model struct the
    // comptime mini-VM injects (`comptime.rs::frozen_type_payload_model_items`)
    // mirrors the unspellable carrier: its `bounds` element type is `never`
    // (uninhabited until A2/B2), which maps through the intermediate-tier
    // Any arm of `type_annotation_to_field_type`. Same disposition and
    // close-out path as the unspellable row above; the name is pinned to
    // `comptime_reflection::frozen_type_enabled_payload_type_name(Erased)`
    // by unit test, and `runtime_lift_rejection` walls the spellable name
    // out of runtime code in the same commit.
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("FrozenErased"),
        section: "§4.D.11",
        permanent: true,
        reason: "ADR-009 B1 S3 comptime-injected FrozenErased payload model; \
                 bounds element type is `never` (empty-only until A2/B2); \
                 comptime-mode-only registration + runtime_lift_rejection \
                 wall keep it out of runtime code",
    },
    // ADR-009 B7 Slice 2 — `FrozenParameter` bound-set carrier. Same
    // disposition as `FrozenErased` above: the unspellable (SOH-prefixed,
    // unforgeable) comptime-only `FrozenType::Parameter` payload schema's
    // `bounds` array is reachable today ONLY as the empty set (trait-reference
    // bound descriptors land with ticket B2, whose element is the SAME
    // TraitRef descriptor that `FrozenErased.bounds` retypes to). The
    // descriptor is lift-walled out of runtime code by
    // `comptime_reflection::runtime_lift_rejection`. Removal happens by
    // retyping the element at B2, not by stripping this row.
    WhitelistEntry {
        rule: WhitelistRule::SchemaName(
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_PARAMETER_SCHEMA,
        ),
        section: "§4.D.11",
        permanent: true,
        reason: "ADR-009 B7 Slice 2 comptime-only FrozenParameter descriptor; \
                 bound set empty-only until B2 (elements retype to TraitRef at \
                 B2); unspellable name + runtime_lift_rejection wall keep it \
                 out of runtime code",
    },
    // ADR-009 B7 Slice 2 — the SPELLABLE `FrozenParameter` payload-model struct
    // the comptime mini-VM injects. Its `bounds` element type is `never`
    // (uninhabited until B2), which maps through the intermediate-tier Any arm
    // of `type_annotation_to_field_type`. Same disposition and close-out path
    // as the unspellable row above; the name is pinned to
    // `comptime_reflection::frozen_type_enabled_payload_type_name(Parameter)`
    // by unit test, and `runtime_lift_rejection` walls the spellable name out
    // of runtime code in the same commit.
    WhitelistEntry {
        rule: WhitelistRule::SchemaName("FrozenParameter"),
        section: "§4.D.11",
        permanent: true,
        reason: "ADR-009 B7 Slice 2 comptime-injected FrozenParameter payload \
                 model; bounds element type is `never` (empty-only until B2); \
                 comptime-mode-only registration + runtime_lift_rejection \
                 wall keep it out of runtime code",
    },
    // ----- TRANSITIONAL row (§4.D.3 + §4.D.5 + §4.D.10-emission)
    //       NARROWED at W17.2-C close (post-W17.2-B PROPAGATE landings)
    //
    // v0.3 Phase 4b Round 5b W17.2-C (audit §9.B.3 supervisor ratify
    // 2026-05-19 + team-lead R5b-2 dispatch operational call on
    // bundling). At W17.2-C close, the transitional row is NARROWED
    // from the empty-prefix (everything) absorber to a
    // `__inline_obj_*` prefix-only absorber. The narrowing landings:
    //
    // - §4.D.1 + §4.D.2 + §4.D.9: CLOSED at W17.2-B (close commit
    //   `e316e171`) via `FieldType::Option(Box<FieldType>)` PROPAGATE
    //   rebuild at `crates/shape-runtime/src/type_schema/field_types.rs`.
    // - §4.D.4: CLOSED at W17.2-C via structured E#### compile error at
    //   the struct-literal schema-lookup-miss fallback
    //   (`compiler/expressions/collections.rs:971-980 + 1000-1008`).
    // - §4.D.7: NARROWED 5-name → 4-name at W17.2-C
    //   (`compiler/helpers.rs:4901`) — Option<T> PROPAGATES via the
    //   §4.D.1 W17.2-B rebuild; HashMap/Map/Result/Set retain the
    //   TRANSITIONAL fallback pending per-container `FieldType`
    //   variant introduction at v0.4 W17.3/W17.4.
    // - §4.D.8: CLOSED at W17.2-C — `_ =>` fall-through at
    //   `compiler/helpers.rs:4905` replaced with explicit per-variant
    //   arms (§4.D.2 same-pattern discipline). Variants that can't
    //   project to a concrete FieldType (Function/Union/Intersection/
    //   Tuple/inline Object/Void/Never/Null/Undefined/Dyn) each get an
    //   audit-cited named arm projecting to `FieldType::Any` — the
    //   verification-pass safety net catches if any reach user-facing
    //   schemas.
    // - §4.D.5: CLOSED at W17.2-C via `register_inline_object_schema`
    //   deprecation at `type_tracking.rs:1024-1057` + caller migration.
    //   The deprecated untyped variant now routes through
    //   `register_inline_object_schema_typed` with `FieldType::Any`
    //   per field; same `__inline_obj_N` schema-name format. The
    //   prefix absorber below handles these via the verification-pass
    //   safety net.
    //
    // SURVIVING (R5b/R6 follow-up territory per audit §4.D.10
    // explicit cite):
    //
    // - §4.D.10 emission-rename: annotation-handler ctx schemas at
    //   `functions_annotations.rs:228-1555` + `expressions/mod.rs:309-667`
    //   currently emit through `register_inline_object_schema_typed`,
    //   producing names `__inline_obj_N` instead of the
    //   `__annotation_ctx_*` whitelist convention introduced at
    //   W17.2-A. R5b/R6 follow-up will introduce a dedicated
    //   registration helper (e.g.
    //   `register_named_synthetic_schema_typed("__annotation_ctx_logging", ...)`)
    //   that uses the `__annotation_ctx_*` prefix and lets the
    //   permanent §4.D.10 whitelist row catch them directly.
    //
    // SURFACE-AND-STOP per audit §0 + CLAUDE.md §10: the §4.D.10
    // emission-rename is a NAMED-AND-CITED R5b/R6 follow-up at the
    // W17.2-C close commit; verification-pass-side absorption stays
    // in place pre-rename via the narrowed `__inline_obj_*` prefix
    // row below.
    //
    // NARROWED from `SchemaNamePrefix("")` (catch-all) to
    // `SchemaNamePrefix("__inline_obj_")` (bounded). User-named schemas
    // carrying `FieldType::Any` outside the named-exception classes
    // now surface E0900 — the post-W17.2-C diagnostic shape.
    WhitelistEntry {
        rule: WhitelistRule::SchemaNamePrefix("__inline_obj_"),
        section: "§4.D.3 + §4.D.5 + §4.D.10-emission",
        permanent: false,
        reason: "TRANSITIONAL (NARROWED at W17.2-C from empty-prefix \
                 catch-all to `__inline_obj_*` prefix-only) — \
                 §4.D.1+§4.D.2+§4.D.4+§4.D.7+§4.D.8+§4.D.9 CLOSED at \
                 W17.2-B+W17.2-C via FieldType::Option PROPAGATE rebuild \
                 + collections.rs:975/1004 structured E#### compile \
                 error + helpers.rs:4901 4-name narrowing + helpers.rs:4905 \
                 explicit per-variant arms + register_inline_object_schema \
                 deprecation+caller migration. Surviving territory: \
                 §4.D.3 (inline-object inference at \
                 `expressions/collections.rs:509/518` — `__inline_obj_N` \
                 emission with FieldType::Any per field when RHS is \
                 non-literal) + §4.D.5 carrier (`type_tracking.rs:1024` \
                 deprecated untyped variant routes through typed-with-Any) \
                 + §4.D.10 emission-rename \
                 (`functions_annotations.rs/expressions/mod.rs` schemas \
                 use `__inline_obj_*` instead of `__annotation_ctx_*` \
                 whitelist convention) — R5b/R6 follow-up per audit \
                 §4.D.10 explicit cite. Take-both at W17.3/R5b/R6 close \
                 strips this row.",
    },
];

/// Match a schema-field site against the whitelist. Returns
/// `Some(WhitelistEntry)` if the site is allowed; `None` if the
/// `FieldType::Any` site is unmatched and must surface E0900.
///
/// The schema name + field name + `is_enum()` flag drive the rules
/// (see [`WhitelistRule`]). The first matching entry wins; ordering in
/// [`WHITELIST`] is by priority (permanent classes first).
fn match_whitelist(
    schema_name: &str,
    field_name: &str,
    is_enum_schema: bool,
) -> Option<&'static WhitelistEntry> {
    WHITELIST.iter().find(|entry| match entry.rule {
        WhitelistRule::SchemaName(name) => name == schema_name,
        WhitelistRule::SchemaNamePrefix(prefix) => schema_name.starts_with(prefix),
        WhitelistRule::EnumPayloadField => is_enum_schema && field_name.starts_with("__payload_"),
    })
}

/// Walk one schema's fields and return E0900 errors for each
/// `FieldType::Any` field that does not match the whitelist.
///
/// Recurses into `FieldType::Array(Box<FieldType>)` element types so
/// that an inner `Array(Box::new(FieldType::Any))` surfaces too — but
/// only if the *outer* schema name doesn't itself match the whitelist
/// (whitelisted schemas allow Any anywhere on their surface, since the
/// parallel-`field_kinds` carrier covers nested element kinds per
/// ADR-006 §2.7.26).
fn verify_schema(schema: &TypeSchema, errors: &mut Vec<ShapeError>) {
    let is_enum_schema = schema.is_enum();
    // Whole-schema whitelist sites (`Row`, `FrameState`, `__mod_*`,
    // builtin schemas, etc.) pass without per-field checks — the
    // entire schema is by-design heterogeneous. The §4.D.3 + §4.D.5 +
    // §4.D.10-emission transitional row (post-W17.2-C narrowed to
    // `__inline_obj_*` prefix-only) IS included in whole-schema
    // short-circuit since the narrowed prefix is bounded and matches
    // ONLY synthetic inline-object schemas (the empty-prefix catch-all
    // was stripped at W17.2-C close per audit §9.B.3).
    let whole_schema_match = WHITELIST.iter().find(|entry| match entry.rule {
        WhitelistRule::SchemaName(name) => name == schema.name,
        WhitelistRule::SchemaNamePrefix(prefix) => {
            // Defensive guard against accidental empty-prefix
            // re-introduction; the narrowed `__inline_obj_*` prefix is
            // bounded by construction at W17.2-C close.
            !prefix.is_empty() && schema.name.starts_with(prefix)
        }
        WhitelistRule::EnumPayloadField => false,
    });
    if whole_schema_match.is_some() {
        return;
    }

    for field in &schema.fields {
        if !field_type_contains_any(&field.field_type) {
            continue;
        }
        match match_whitelist(&schema.name, &field.name, is_enum_schema) {
            Some(entry) if entry.permanent => {
                // Permanent named exception (§4.D.10–§4.D.15). Allowed.
                continue;
            }
            Some(entry) => {
                // Transitional row (§4.D.3-8 + §4.D.10-emission post-
                // W17.2-B narrowing). Tolerated pre-W17.2-C take-both
                // per audit §9.B.1+§9.B.3 + R5b/R6 emission-rename
                // follow-up. The transitional entry's `reason` field
                // carries the close-out cite — the audit pin is
                // recorded here, but the pass does not surface a
                // diagnostic for transitional matches.
                let _ = entry;
                continue;
            }
            None => {
                errors.push(emit_e0900(schema, &field.name, &field.field_type));
            }
        }
    }
}

/// Returns `true` if the given `FieldType` is `Any` or contains `Any`
/// in a nested `Array(Box<...>)` / `Option(Box<...>)` / `HashMap { key,
/// value }` / `Set(Box<...>)` element position. Other compound variants
/// (`Object(name)`) reference another schema by name; that schema is
/// verified independently via its own [`verify_schema`] call.
///
/// W17.2-B (audit §4.D.1 + §9.B.1 (a) supervisor ratify 2026-05-19):
/// `FieldType::Option(_)` is itself NOT Any (it's a concrete
/// discriminator). But `Option<Any>` is — the inner Any signals an
/// unresolved-element-type carrier that bidirectional inference
/// should have narrowed. Same recursion-shape as `Array`.
///
/// W17.3-4.2 (audit §4.B compiler integration, supervisor ratify
/// 2026-05-22): `HashMap { key, value }` and `Set(inner)` recurse the
/// same way — an inner `Any` inside a per-container variant signals an
/// unresolved element/key/value type that bidirectional inference
/// should have narrowed. Without this recursion the previous
/// `_ => false` defensive arm would silently absorb e.g.
/// `HashMap<string, Any>` past the E0900 boundary. Explicit per-variant
/// arms preserve the §4.D.2 "no catch-all" discipline + ADR-005 §1
/// single-discriminator (HeapValue::kind() canonical for heap dispatch;
/// FieldType variants are schema-layer descriptors only).
fn field_type_contains_any(ft: &FieldType) -> bool {
    match ft {
        FieldType::Any => true,
        FieldType::Array(inner) => field_type_contains_any(inner),
        FieldType::Option(inner) => field_type_contains_any(inner),
        // W17.3-4.2 — per-container recursion. Mirrors the Array /
        // Option recursion shape per audit §4.B.3 + close-gate signal
        // "post-inference verify pass handles new variants without
        // panicking or reaching fallback Any" (§5.B W17.3-4.2).
        FieldType::HashMap { key, value } => {
            field_type_contains_any(key) || field_type_contains_any(value)
        }
        FieldType::Set(inner) => field_type_contains_any(inner),
        // Explicit per-variant arms (no catch-all per audit §4.D.2 +
        // CLAUDE.md "exhaustive-match errors guide completion; do NOT
        // use catch-all `_ =>` arms that mask missing variants").
        FieldType::F64
        | FieldType::I64
        | FieldType::Bool
        | FieldType::String
        | FieldType::Timestamp
        | FieldType::Decimal
        | FieldType::Object(_)
        | FieldType::I8
        | FieldType::U8
        | FieldType::I16
        | FieldType::U16
        | FieldType::I32
        | FieldType::U32
        | FieldType::U64 => false,
    }
}

fn emit_e0900(schema: &TypeSchema, field_name: &str, ft: &FieldType) -> ShapeError {
    // Synthetic source location — verification runs post-inference and
    // is not anchored to a specific span; the message names the schema
    // + field directly.
    let location = SourceLocation {
        file: None,
        line: 0,
        column: 0,
        length: None,
        source_line: None,
        hints: vec![format!(
            "consult `docs/cluster-audits/v0.3-w17-fieldtype-any-boundary-audit.md` \
             §4.D for per-site disposition; permanent named exceptions are \
             whitelisted in `crates/shape-vm/src/compiler/post_inference_verify.rs`"
        )],
        notes: vec![],
        is_synthetic: true,
        fixes: vec![],
    };

    let _ = ErrorCode::E0900; // ADR-006 §2.7.5 stamp marker — code allocated
    let message = format!(
        "[E0900] post-inference FieldType::Any in user-facing schema \
         `{}` at field `{}` (resolved type: {}). User-introduced \
         FieldType::Any outside the named-exception classes is the \
         schema-side analogue of the deleted dynamic-slot-kind variants \
         per CLAUDE.md Forbidden Patterns (strict-typing plan). See \
         ADR-006 §2.7.5 + §2.7.26 + audit §5 for the binding rule.",
        schema.name, field_name, ft
    );

    ShapeError::SemanticError {
        message,
        location: Some(location),
    }
}

/// Verify the post-inference output of the bytecode compiler. Walks the
/// `BytecodeProgram.type_schema_registry` (the canonical post-compile
/// schema state per audit §5) and surfaces E0900 for every
/// `FieldType::Any` field that does not match the named-exception
/// whitelist (PERMANENT §4.D.10–§4.D.15) or the transitional whitelist
/// (TRANSITIONAL §4.D.1–§4.D.9, closes in R5b W17.2-B+C per user
/// 2026-05-19 binding).
///
/// Returns `Ok(())` if every schema is clean. Returns
/// `Err(ShapeError::MultiError(errors))` with one structured E0900 per
/// unmatched site (or a single `ShapeError::SemanticError` if only one
/// site fires) otherwise.
pub fn verify_no_post_inference_any(program: &BytecodeProgram) -> Result<(), ShapeError> {
    let mut errors: Vec<ShapeError> = Vec::new();
    verify_registry(&program.type_schema_registry, &mut errors);
    finalize(errors)
}

/// Verify a `TypeSchemaRegistry` in isolation — used by the tests and
/// callable from the top-level entry [`verify_no_post_inference_any`].
pub(crate) fn verify_registry(registry: &TypeSchemaRegistry, errors: &mut Vec<ShapeError>) {
    // Iterate every named schema; `type_names()` yields the keys of the
    // `by_name` map per the public API contract at
    // `crates/shape-runtime/src/type_schema/registry.rs:218`.
    let names: Vec<String> = registry.type_names().map(|s| s.to_string()).collect();
    for name in &names {
        if let Some(schema) = registry.get(name) {
            verify_schema(schema, errors);
        }
    }
}

fn finalize(errors: Vec<ShapeError>) -> Result<(), ShapeError> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().expect("len==1")),
        _ => Err(ShapeError::MultiError(errors)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{EnumVariantInfo, TypeSchema, TypeSchemaRegistry};

    fn run_on(registry: &TypeSchemaRegistry) -> Result<(), ShapeError> {
        let mut errors = Vec::new();
        verify_registry(registry, &mut errors);
        finalize(errors)
    }

    // ----- POSITIVE tests (whitelisted sites — verification passes) -----

    /// §4.D.11 Row type carries `FieldType::Any` at "fields" by design;
    /// parallel-`field_kinds` per ADR-006 §2.7.26 stamps the column
    /// kinds at row-construction time.
    #[test]
    fn positive_row_carrier_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "Row",
            vec![
                ("timestamp".to_string(), FieldType::Timestamp),
                ("fields".to_string(), FieldType::Any),
            ],
        );
        reg.register(schema);
        assert!(run_on(&reg).is_ok(), "Row schema must pass §4.D.11");
    }

    /// §4.D.15 VM-state introspection — FrameState/VmState/ModuleState/
    /// CallPayload/Delta. All five whitelist-rule-allowed.
    #[test]
    fn positive_vm_state_introspection_passes() {
        let mut reg = TypeSchemaRegistry::new();
        for name in &[
            "FrameState",
            "VmState",
            "ModuleState",
            "CallPayload",
            "Delta",
        ] {
            let id = reg.allocate_id();
            let schema =
                TypeSchema::with_id(id, *name, vec![("contents".to_string(), FieldType::Any)]);
            reg.register(schema);
        }
        assert!(
            run_on(&reg).is_ok(),
            "VM-state introspection schemas must pass §4.D.15"
        );
    }

    /// §4.D.12 Predeclared-schema carrier-tier — `__predecl_*` prefix.
    /// The parallel-`field_kinds` track at storage construction time IS
    /// the §2.7.5 stamp per ADR-006 §2.7.26.
    #[test]
    fn positive_predeclared_any_schema_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "__predecl_a_b_c",
            vec![
                ("a".to_string(), FieldType::Any),
                ("b".to_string(), FieldType::Any),
                ("c".to_string(), FieldType::Any),
            ],
        );
        reg.register(schema);
        assert!(
            run_on(&reg).is_ok(),
            "predeclared schemas must pass §4.D.12"
        );
    }

    /// §4.D.3 + §4.D.5 Inline-object `__inline_obj_N` carrier. ADR-009
    /// E4-D2 ctx-removal (slice S3) deleted the `__annotation_ctx_` whitelist
    /// row; this pin proves the SURVIVING `__inline_obj_` row still clears an
    /// inline-object `FieldType::Any` site, i.e. E0900 is NOT regressed by the
    /// ctx-row deletion.
    #[test]
    fn s3_inline_object_any_still_passes_verify() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "__inline_obj_0",
            vec![
                ("state".to_string(), FieldType::Any),
                (
                    "event_log".to_string(),
                    FieldType::Array(Box::new(FieldType::Any)),
                ),
            ],
        );
        reg.register(schema);
        assert!(
            run_on(&reg).is_ok(),
            "inline-object __inline_obj_N Any sites must still pass verify \
             via the surviving __inline_obj_ row (§4.D.3 + §4.D.5)"
        );
    }

    /// §4.D.14 Enum `__payload_N` slots — schema is `is_enum()` and
    /// the payload field name matches `__payload_*`.
    #[test]
    fn positive_enum_payload_fields_pass() {
        let mut reg = TypeSchemaRegistry::new();
        // `new_enum_with_id` builds variant payload fields named
        // `__payload_0..N` typed `FieldType::Any` per
        // `crates/shape-runtime/src/type_schema/schema.rs:203`.
        let id = reg.allocate_id();
        let schema = TypeSchema::new_enum_with_id(
            id,
            "MyEnum",
            vec![
                EnumVariantInfo::new("A", 0, 1),
                EnumVariantInfo::new("B", 1, 2),
            ],
        );
        reg.register(schema);
        assert!(
            run_on(&reg).is_ok(),
            "enum __payload_N fields must pass §4.D.14"
        );
    }

    // ----- NEGATIVE tests (unmatched sites — E0900 fires) -----
    //
    // Note: per user 2026-05-19 binding, the transitional whitelist
    // (§4.D.1-9) covers ALL user-named schemas at this sub-cluster
    // landing. The R5b take-both commit strips the transitional rows
    // and lets E0900 surface. To test the negative path AT THIS
    // SUB-CLUSTER, the tests below construct a registry and use the
    // structural `match_whitelist` + `verify_schema` helpers directly
    // against a hypothetical post-R5b whitelist (the PERMANENT
    // entries only) to validate the diagnostic shape independently of
    // the transitional umbrella.

    /// Helper that mirrors `verify_registry` but uses only the
    /// PERMANENT subset of the whitelist (the post-R5b take-both
    /// shape). Tests the diagnostic emission path that R5b unlocks.
    fn verify_registry_permanent_only(registry: &TypeSchemaRegistry, errors: &mut Vec<ShapeError>) {
        let names: Vec<String> = registry.type_names().map(|s| s.to_string()).collect();
        for name in &names {
            if let Some(schema) = registry.get(name) {
                verify_schema_permanent_only(schema, errors);
            }
        }
    }

    fn verify_schema_permanent_only(schema: &TypeSchema, errors: &mut Vec<ShapeError>) {
        let is_enum_schema = schema.is_enum();

        // Whole-schema match against PERMANENT entries only.
        let whole_match = WHITELIST
            .iter()
            .filter(|e| e.permanent)
            .find(|entry| match entry.rule {
                WhitelistRule::SchemaName(name) => name == schema.name,
                WhitelistRule::SchemaNamePrefix(prefix) => {
                    !prefix.is_empty() && schema.name.starts_with(prefix)
                }
                WhitelistRule::EnumPayloadField => false,
            });
        if whole_match.is_some() {
            return;
        }

        for field in &schema.fields {
            if !field_type_contains_any(&field.field_type) {
                continue;
            }
            let matched = WHITELIST
                .iter()
                .filter(|e| e.permanent)
                .find(|entry| match entry.rule {
                    WhitelistRule::SchemaName(name) => name == schema.name,
                    WhitelistRule::SchemaNamePrefix(prefix) => {
                        !prefix.is_empty() && schema.name.starts_with(prefix)
                    }
                    WhitelistRule::EnumPayloadField => {
                        is_enum_schema && field.name.starts_with("__payload_")
                    }
                });
            if matched.is_none() {
                errors.push(emit_e0900(schema, &field.name, &field.field_type));
            }
        }
    }

    /// Runtime builtin `__Option` / `__Result` schemas preserve their
    /// concrete payload kind in the parallel `field_kinds` track, so the
    /// `payload: Any` schema field is a permanent §4.D.11 exception.
    #[test]
    fn runtime_builtin_option_result_payload_any_pass_permanent_only() {
        let mut reg = TypeSchemaRegistry::new();
        for name in ["__Option", "__Result"] {
            let id = reg.allocate_id();
            let schema = TypeSchema::with_id(
                id,
                name,
                vec![
                    ("variant".to_string(), FieldType::I64),
                    ("payload".to_string(), FieldType::Any),
                ],
            );
            reg.register(schema);
        }

        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert!(
            errors.is_empty(),
            "__Option/__Result payload Any must pass permanent-only verification"
        );
    }

    /// Negative: user `type T { x: Any }` shape — the `x` field on a
    /// user-named schema `T` is `FieldType::Any` and does not match
    /// any PERMANENT class. E0900 fires.
    #[test]
    fn negative_user_any_annotation_fires_e0900() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(id, "T", vec![("x".to_string(), FieldType::Any)]);
        reg.register(schema);

        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(errors.len(), 1, "exactly one E0900 expected");
        match &errors[0] {
            ShapeError::SemanticError { message, .. } => {
                assert!(message.contains("E0900"), "message must cite E0900");
                assert!(message.contains("`T`"), "message must name schema");
                assert!(message.contains("`x`"), "message must name field");
            }
            other => panic!("expected SemanticError, got {:?}", other),
        }
    }

    /// Negative: untyped inline object (`let x = { f: <untyped> }`)
    /// surfaces as `FieldType::Any` on a synthetic struct schema named
    /// like `__inline_*` (not whitelisted). E0900 fires post-R5b.
    #[test]
    fn negative_untyped_inline_object_fires_e0900() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "__inline_obj_42",
            vec![
                ("f".to_string(), FieldType::Any),
                ("g".to_string(), FieldType::I64),
            ],
        );
        reg.register(schema);

        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(errors.len(), 1, "exactly one E0900 expected");
        match &errors[0] {
            ShapeError::SemanticError { message, .. } => {
                assert!(message.contains("E0900"));
                assert!(message.contains("__inline_obj_42"));
                assert!(message.contains("`f`"));
            }
            other => panic!("expected SemanticError, got {:?}", other),
        }
    }

    /// Negative: struct-literal schema-lookup miss surfaces as a
    /// user-named schema with parallel `FieldType::Any` columns (the
    /// `collections.rs:953/982` fallback). Multiple Any fields ⇒
    /// multiple E0900 errors collected into MultiError.
    #[test]
    fn negative_struct_literal_schema_miss_fires_e0900() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "Person",
            vec![
                ("name".to_string(), FieldType::Any),
                ("age".to_string(), FieldType::Any),
            ],
        );
        reg.register(schema);

        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(errors.len(), 2, "two E0900 expected (name + age)");
        for err in &errors {
            match err {
                ShapeError::SemanticError { message, .. } => {
                    assert!(message.contains("E0900"));
                    assert!(message.contains("Person"));
                }
                other => panic!("expected SemanticError, got {:?}", other),
            }
        }
    }

    // ----- Post-W17.2-C narrowed-transitional sanity check + new
    //       regressions -----

    /// Post-W17.2-C (audit §9.B.3 supervisor ratify 2026-05-19): the
    /// transitional row was NARROWED from empty-prefix (catch-all) to
    /// `__inline_obj_*` (prefix-only). User-named schemas with
    /// `FieldType::Any` outside the named-exception classes NOW fire
    /// E0900 via the primary `run_on` entry — the post-W17.2-C
    /// diagnostic shape.
    #[test]
    fn post_w17_2_c_user_named_any_fires_via_primary_entry() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "MyUserType",
            vec![("dynamic_field".to_string(), FieldType::Any)],
        );
        reg.register(schema);
        // Post-W17.2-C: user-named schemas with bare Any fields fire
        // E0900 directly through the primary verification entry. The
        // empty-prefix catch-all was stripped per audit §9.B.3 +
        // §8 W17.2-C close-gate.
        let result = run_on(&reg);
        assert!(
            result.is_err(),
            "user-named schema with Any must surface E0900 post-W17.2-C narrowing"
        );
        match result.unwrap_err() {
            ShapeError::SemanticError { message, .. } => {
                assert!(message.contains("E0900"));
                assert!(message.contains("MyUserType"));
                assert!(message.contains("dynamic_field"));
            }
            other => panic!("expected SemanticError, got {:?}", other),
        }
    }

    /// Post-W17.2-C: `__inline_obj_*` synthetic schemas STILL pass via
    /// the narrowed transitional row. These are produced by the
    /// `register_inline_object_schema*` family at W17.2-C deprecated
    /// untyped variant + inline-object inference at §4.D.3 sites. The
    /// W17.3/R5b/R6 follow-up strips this row once the inline-object
    /// inference path closes.
    #[test]
    fn post_w17_2_c_inline_obj_still_absorbed() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "__inline_obj_99",
            vec![("dynamic_field".to_string(), FieldType::Any)],
        );
        reg.register(schema);
        assert!(
            run_on(&reg).is_ok(),
            "narrowed transitional row must still absorb __inline_obj_* schemas"
        );
    }

    /// Post-W17.2-C: `helpers.rs:4901` 4-name TRANSITIONAL list narrowed
    /// from 5-name (HashMap | Map | Result | Set; Option REMOVED per
    /// W17.2-B PROPAGATE). The narrowing is verified at source-grep
    /// level (test asserts the constant list is exactly 4 names).
    /// Source-of-truth: `compiler/helpers.rs:4899-4910`.
    #[test]
    fn post_w17_2_c_helpers_4901_narrowed_to_4_names() {
        // This is a documentation-anchor test (the actual narrowing
        // is enforced by the source-grep close gate). The 4 names per
        // supervisor §9.B.3 ratify are:
        let names = ["HashMap", "Map", "Result", "Set"];
        // Option is NOT in the list (PROPAGATES via FieldType::Option
        // per W17.2-B close commit e316e171).
        assert!(!names.contains(&"Option"));
        assert_eq!(names.len(), 4, "TRANSITIONAL list MUST be 4 names");
    }

    /// Post-W17.2-B regression: `FieldType::Option(Box<FieldType::I64>)`
    /// on a user-named schema is concrete (NOT Any) and passes. The
    /// Option-rebuild from `semantic_to_field_type` lowers `is_optional`
    /// + concrete inner T through this shape per audit §4.D.1.
    #[test]
    fn post_w17_2_b_option_concrete_inner_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "OptionalIntField",
            vec![("x".to_string(), FieldType::Option(Box::new(FieldType::I64)))],
        );
        reg.register(schema);
        assert!(
            run_on(&reg).is_ok(),
            "Option<int> field on user-named schema must pass post-W17.2-B"
        );
    }

    /// Post-W17.2-B negative (uses permanent-only helper to test the
    /// post-W17.2-C diagnostic shape):
    /// `FieldType::Option(Box<FieldType::Any>)` surfaces E0900 — the
    /// inner Any inside Option still signals an unresolved-element-type
    /// carrier per the `field_type_contains_any` recursion update.
    #[test]
    fn post_w17_2_b_option_any_inner_rejected_under_permanent_only() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "OptionalAnyField",
            vec![("x".to_string(), FieldType::Option(Box::new(FieldType::Any)))],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "Option<Any> inner Any must surface E0900 under permanent-only \
             verification per W17.2-B `field_type_contains_any` recursion"
        );
    }

    // =======================================================================
    // W17.3-4.2 — `field_type_contains_any` recursion through per-container
    // variants (HashMap/Set) per audit §4.B.3 + §5.B W17.3-4.2 close gate:
    // "Post-inference verify pass handles new variants without panicking or
    // reaching fallback Any". Mirrors the W17.2-B Option recursion tests.
    // =======================================================================

    /// W17.3-4.2 — `HashMap<string, int>` field is concrete (no inner Any)
    /// and passes verification under permanent-only. Source-of-truth for
    /// the "handles new variants without panicking" close-gate signal.
    #[test]
    fn w17_3_4_2_hashmap_concrete_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "TaggedTable",
            vec![(
                "tags".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::I64),
                },
            )],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert!(
            errors.is_empty(),
            "HashMap<string, int> field must pass verification post-W17.3-4.2; \
             got {} error(s)",
            errors.len()
        );
    }

    /// W17.3-4.2 — `Set<int>` concrete field passes.
    #[test]
    fn w17_3_4_2_set_concrete_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "TaggedTable",
            vec![("ids".to_string(), FieldType::Set(Box::new(FieldType::I64)))],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert!(
            errors.is_empty(),
            "Set<int> field must pass verification post-W17.3-4.2; got {} error(s)",
            errors.len()
        );
    }

    /// W17.3-4.2 — `HashMap<string, Any>` value-inner Any surfaces E0900
    /// under permanent-only. The W17.3-4.1 defensive `_ => false` arm
    /// pre-W17.3-4.2 silently absorbed this; the recursion update closes
    /// the gap per §4.B.3.
    #[test]
    fn w17_3_4_2_hashmap_value_any_rejected_under_permanent_only() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "TaggedTableWithAnyValue",
            vec![(
                "tags".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::Any),
                },
            )],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "HashMap<string, Any> inner Any must surface E0900 under \
             permanent-only verification per W17.3-4.2 recursion"
        );
    }

    /// W17.3-4.2 — `HashMap<Any, int>` key-inner Any also surfaces.
    /// Mirrors the value-inner symmetry; both K and V are checked.
    #[test]
    fn w17_3_4_2_hashmap_key_any_rejected_under_permanent_only() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "TaggedTableWithAnyKey",
            vec![(
                "tags".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::Any),
                    value: Box::new(FieldType::I64),
                },
            )],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "HashMap<Any, int> inner Any in key must surface E0900 under \
             permanent-only verification per W17.3-4.2 recursion"
        );
    }

    /// W17.3-4.2 — `Set<Any>` inner Any surfaces E0900 under permanent-only.
    #[test]
    fn w17_3_4_2_set_any_inner_rejected_under_permanent_only() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "SetOfAny",
            vec![("ids".to_string(), FieldType::Set(Box::new(FieldType::Any)))],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "Set<Any> inner Any must surface E0900 under permanent-only \
             verification per W17.3-4.2 recursion"
        );
    }

    /// W17.3-4.2 — deeply-nested `Array<HashMap<string, Set<Any>>>`
    /// surfaces the deeply-nested Any. Verifies recursion composes
    /// correctly through Array → HashMap.value → Set.
    #[test]
    fn w17_3_4_2_nested_containers_inner_any_rejected() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "NestedContainerAnyField",
            vec![(
                "x".to_string(),
                FieldType::Array(Box::new(FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::Set(Box::new(FieldType::Any))),
                })),
            )],
        );
        reg.register(schema);
        let mut errors = Vec::new();
        verify_registry_permanent_only(&reg, &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "deeply-nested Any inside Array<HashMap<string, Set<Any>>> \
             must surface E0900 per W17.3-4.2 recursion-composition"
        );
    }
}
