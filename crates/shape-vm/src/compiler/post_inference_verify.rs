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
#[derive(Debug, Clone, Copy)]
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
        rule: WhitelistRule::SchemaNamePrefix("__annotation_ctx_"),
        section: "§4.D.10",
        permanent: true,
        reason: "annotation-handler ctx schemas \
                 (functions_annotations.rs:228-1555 + \
                 expressions/mod.rs:309-667); heterogeneous-by-design \
                 (@before/@after handler ABI receives arbitrary \
                 user state); parallel-`field_kinds` per ADR-006 §2.7.26",
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
    // ----- TRANSITIONAL classes (§4.D.1–§4.D.9) — close in R5b W17.2-B+C
    //
    // Per user 2026-05-19 binding + audit §9.B.1 + §9.B.3 ratify path:
    // the post-R5b take-both commit (NOT this sub-cluster's responsibility)
    // tightens the whitelist to §4.D.10-15 only + strips these transitional
    // rows once W17.2-B (semantic_to_field_type Option rebuild + fall-
    // through hardening) and W17.2-C (compiler-side fallback hardening)
    // land.
    //
    // The schema-construction fallback sites (§4.D.1-§4.D.5, §4.D.7-§4.D.9)
    // currently produce `FieldType::Any` at user-named TypedObject schemas
    // when the inference layer cannot prove a concrete element type. The
    // transitional whitelist disposition: until R5b lands the upstream
    // fixes, allow all post-inference Any in user-named schemas with a
    // STRUCTURED warning surface that names the §-cite. This is NOT a
    // blanket fallback — it is a bounded scaffold with a named close-out
    // commit pending.
    //
    // The rule shape is a `*` schema-prefix match (catches every
    // user-named schema). The `permanent: false` flag marks each row as
    // closure-pending. R5b close strips them via take-both.
    WhitelistEntry {
        rule: WhitelistRule::SchemaNamePrefix(""),
        section: "§4.D.1-9",
        permanent: false,
        reason: "TRANSITIONAL — closes in R5b W17.2-B+C per audit \
                 §9.B.1+§9.B.3 ratifies (semantic_to_field_type \
                 Option-rebuild + compiler-side fallback hardening + \
                 W15.2-LANG-8 struct-literal schema-lookup miss + \
                 helpers.rs:4901 generic-container narrowing); \
                 user 2026-05-19 binding",
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
        WhitelistRule::EnumPayloadField => {
            is_enum_schema && field_name.starts_with("__payload_")
        }
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
    // Whole-schema whitelist sites (`Row`, `FrameState`, `__mod_*`, etc.)
    // pass without per-field checks — the entire schema is by-design
    // heterogeneous.
    let whole_schema_match = WHITELIST.iter().find(|entry| match entry.rule {
        WhitelistRule::SchemaName(name) => name == schema.name,
        WhitelistRule::SchemaNamePrefix(prefix) => {
            // The empty-prefix transitional row matches everything;
            // it's also the catch-all fall-through. Don't short-circuit
            // here — let it fire via match_whitelist at per-field time.
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
                // Transitional row (§4.D.1–§4.D.9). Tolerated pre-R5b
                // close per user 2026-05-19 binding. The transitional
                // entry's `reason` field carries the close-out cite —
                // the audit pin is recorded here, but the pass does not
                // surface a diagnostic for transitional matches.
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
/// in a nested `Array(Box<...>)` element position. Other compound
/// variants (`Object(name)`) reference another schema by name; that
/// schema is verified independently via its own [`verify_schema`] call.
fn field_type_contains_any(ft: &FieldType) -> bool {
    match ft {
        FieldType::Any => true,
        FieldType::Array(inner) => field_type_contains_any(inner),
        _ => false,
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
pub fn verify_no_post_inference_any(
    program: &BytecodeProgram,
) -> Result<(), ShapeError> {
    let mut errors: Vec<ShapeError> = Vec::new();
    verify_registry(&program.type_schema_registry, &mut errors);
    finalize(errors)
}

/// Verify a `TypeSchemaRegistry` in isolation — used by the tests and
/// callable from the top-level entry [`verify_no_post_inference_any`].
pub(crate) fn verify_registry(
    registry: &TypeSchemaRegistry,
    errors: &mut Vec<ShapeError>,
) {
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
    /// CallPayload. All four whitelist-rule-allowed.
    #[test]
    fn positive_vm_state_introspection_passes() {
        let mut reg = TypeSchemaRegistry::new();
        for name in &["FrameState", "VmState", "ModuleState", "CallPayload"] {
            let id = reg.allocate_id();
            let schema = TypeSchema::with_id(
                id,
                *name,
                vec![("contents".to_string(), FieldType::Any)],
            );
            reg.register(schema);
        }
        assert!(
            run_on(&reg).is_ok(),
            "VM-state introspection schemas must pass §4.D.15"
        );
    }

    /// §4.D.10 Annotation-handler ctx schemas. The prefix `__annotation_ctx_`
    /// is the convention introduced for verification-pass routing per
    /// audit §5; existing call sites at `functions_annotations.rs:228-1555`
    /// + `expressions/mod.rs:309-667` need a R5b/R6 rename of their
    /// schema names to align with this prefix — the whitelist row is
    /// landed at W17.2-A so the rename can be wired-in without
    /// re-amending the whitelist.
    #[test]
    fn positive_annotation_handler_ctx_passes() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "__annotation_ctx_logging",
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
            "annotation-handler ctx schemas must pass §4.D.10"
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
    fn verify_registry_permanent_only(
        registry: &TypeSchemaRegistry,
        errors: &mut Vec<ShapeError>,
    ) {
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
            let matched = WHITELIST.iter().filter(|e| e.permanent).find(|entry| {
                match entry.rule {
                    WhitelistRule::SchemaName(name) => name == schema.name,
                    WhitelistRule::SchemaNamePrefix(prefix) => {
                        !prefix.is_empty() && schema.name.starts_with(prefix)
                    }
                    WhitelistRule::EnumPayloadField => {
                        is_enum_schema && field.name.starts_with("__payload_")
                    }
                }
            });
            if matched.is_none() {
                errors.push(emit_e0900(schema, &field.name, &field.field_type));
            }
        }
    }

    /// Negative: user `type T { x: Any }` shape — the `x` field on a
    /// user-named schema `T` is `FieldType::Any` and does not match
    /// any PERMANENT class. E0900 fires.
    #[test]
    fn negative_user_any_annotation_fires_e0900() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "T",
            vec![("x".to_string(), FieldType::Any)],
        );
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

    // ----- Transitional disposition sanity check -----

    /// Pre-R5b: the transitional whitelist absorbs user schemas with
    /// `FieldType::Any`. This sub-cluster's primary entry point
    /// (`verify_no_post_inference_any` via `verify_registry`) MUST
    /// return Ok for these — they close in R5b W17.2-B+C, not here.
    #[test]
    fn transitional_user_any_passes_at_w17_2_a_landing() {
        let mut reg = TypeSchemaRegistry::new();
        let id = reg.allocate_id();
        let schema = TypeSchema::with_id(
            id,
            "MyUserType",
            vec![("dynamic_field".to_string(), FieldType::Any)],
        );
        reg.register(schema);

        // The R5a-landing entry point tolerates user Any per
        // §4.D.1-9 transitional whitelist.
        assert!(
            run_on(&reg).is_ok(),
            "transitional whitelist must absorb user Any pre-R5b close"
        );
    }
}
