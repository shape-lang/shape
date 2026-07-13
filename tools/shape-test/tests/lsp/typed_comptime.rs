use shape_test::shape_test::{ShapeTest, pos};

#[test]
fn comptime_completion_offers_typed_reflection_builtins() {
    ShapeTest::new("comptime {\n    \n}\n")
        .at(pos(1, 4))
        .expect_completion("type_ref")
        .expect_completion("type_category");
}

#[test]
fn runtime_completion_hides_typed_reflection_builtins() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_no_completion("type_ref")
        .expect_no_completion("type_category");
}

#[test]
fn annotation_comptime_hook_offers_typed_reflection_builtins() {
    ShapeTest::new(
        "annotation inspect() {\n  targets: [type]\n  comptime post(target, ctx) {\n    \n  }\n}\n",
    )
    .at(pos(3, 4))
    .expect_completion("type_ref")
    .expect_completion("type_category");
}

#[test]
fn type_ref_hover_explains_opaque_identity() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn type_category_hover_exposes_exhaustive_result() {
    ShapeTest::new("let category = comptime { type_category(type_ref(int)) }\n")
        .at(pos(0, 32))
        .expect_hover_contains("exhaustive semantic category");
}

#[test]
fn type_ref_hover_exposes_typed_signature() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("type_ref(T) -> TypeRef<T>");
}

#[test]
fn typed_reflection_offers_signature_help() {
    ShapeTest::new("let reflected = comptime { type_ref( ) }\n")
        .at(pos(0, 36))
        .expect_signature_help();
}

// =====================================================================
// ADR-009 A2 (S6): completion inside the `type_ref(` TYPE position.
// The argument of type_ref is checked type syntax, so completion routes
// to the type-annotation provider: type names (primitives, user types,
// in-scope generic parameters) — never value bindings.
// =====================================================================

#[test]
fn type_ref_argument_completion_offers_type_names_not_value_bindings() {
    ShapeTest::new(
        "type Point {\n  x: int\n}\nlet count = 1\nlet reflected = comptime { type_ref( ) }\n",
    )
    .at(pos(4, 36))
    .expect_completion("int")
    .expect_completion("Point")
    .expect_completion("Option")
    .expect_no_completion("count")
    .expect_no_completion("reflected");
}

#[test]
fn generic_body_type_ref_argument_completion_offers_type_parameters() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref( ) }\n  label\n}\n",
    )
    .at(pos(1, 34))
    .expect_completion("T")
    .expect_completion("int")
    .expect_no_completion("value")
    .expect_no_completion("label");
}

#[test]
fn type_ref_hover_documents_the_checked_type_expression_surface() {
    // The catalog-owned builtin row (comptime_reflection.rs) documents the
    // A2 checked type-expression forms; hover renders that row.
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("applied generics")
        .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn string_type_ref_construction_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(\"int\") }\n")
        .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

#[test]
fn unresolved_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(DoesNotExist) }\n")
        .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

/// ADR-009 A2 (S5) LSP mirror: a STRING spelling a composite type is still a
/// string — the named rejection reaches LSP semantic diagnostics through the
/// S3 expression fallback.
#[test]
fn composite_string_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(\"Option<int>\") }\n")
        .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

/// ADR-009 A2 (S5) LSP mirror: an unresolved leaf NESTED inside a checked
/// type expression surfaces the named freeze rejection as a semantic
/// diagnostic (compile-time, before user comptime executes — Dec 52).
#[test]
fn nested_unresolved_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(Option<Bogus>) }\n")
        .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

#[test]
fn legacy_type_descriptor_has_semantic_diagnostic() {
    ShapeTest::new("let category = comptime { type_category(type_info(int).type_ref) }\n")
        .expect_semantic_diagnostic_contains("TypeRef");
}

#[test]
fn raw_type_ref_escape_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\nprint(reflected)\n")
        .expect_semantic_diagnostic_contains("comptime-only compiler capability");
}

#[test]
fn frozen_category_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new("let category = comptime { FrozenTypeCategory:: }\n")
        .at(pos(0, 46))
        .expect_completion("Primitive")
        .expect_completion("Erased")
        .expect_no_completion("Unknown");
}

#[test]
fn enum_variant_completion_path_is_not_specific_to_comptime() {
    ShapeTest::new("enum Color { Red, Green }\nlet color = Color::\n")
        .at(pos(1, 19))
        .expect_completion("Red")
        .expect_completion("Green");
}

#[test]
fn frozen_category_completion_filters_partial_variant_prefix() {
    ShapeTest::new("let category = comptime { FrozenTypeCategory::Pr }\n")
        .at(pos(0, 48))
        .expect_completion("Primitive")
        .expect_no_completion("Erased");
}

// =====================================================================
// ADR-009 B1 (S5): reflect + the payload descriptor types. Hover flows
// from the catalog-owned REFLECT_BUILTIN_ROW; completion visibility is
// metadata-driven (is_comptime_builtin_function); descriptor-enum variant
// completion is a catalog-keyed lookup over the shared runtime constants
// (closed lists, no Unknown arm, no hand-written parallel list); semantic
// diagnostics surface the R1/R2/R4/R5 rejection forms through the same
// compile-error plumbing as the A1 rows.
// =====================================================================

#[test]
fn comptime_completion_offers_reflect() {
    ShapeTest::new("comptime {\n    \n}\n")
        .at(pos(1, 4))
        .expect_completion("reflect");
}

#[test]
fn runtime_completion_hides_reflect() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_no_completion("reflect");
}

#[test]
fn annotation_comptime_hook_offers_reflect() {
    ShapeTest::new(
        "annotation inspect() {\n  targets: [type]\n  comptime post(target, ctx) {\n    \n  }\n}\n",
    )
    .at(pos(3, 4))
    .expect_completion("reflect");
}

#[test]
fn reflect_hover_exposes_typed_signature() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(int)) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("reflect(type_ref: TypeRef<T>) -> FrozenType<T>");
}

#[test]
fn reflect_hover_notes_the_enabled_payload_stage() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(int)) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("named compile-time rejection");
}

#[test]
fn frozen_type_completion_is_closed_to_enabled_payload_variants() {
    ShapeTest::new("let payload = comptime { FrozenType:: }\n")
        .at(pos(0, 37))
        .expect_completion("Primitive")
        .expect_completion("Never")
        .expect_completion("Erased")
        .expect_no_completion("Unknown")
        .expect_no_completion("Nominal");
}

#[test]
fn frozen_primitive_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new("let payload = comptime { FrozenPrimitive:: }\n")
        .at(pos(0, 42))
        .expect_completion("SignedInteger")
        .expect_completion("BinaryFloat")
        .expect_completion("Decimal")
        .expect_no_completion("Unknown");
}

#[test]
fn frozen_primitive_completion_filters_partial_variant_prefix() {
    ShapeTest::new("let payload = comptime { FrozenPrimitive::Si }\n")
        .at(pos(0, 44))
        .expect_completion("SignedInteger")
        .expect_no_completion("Decimal");
}

#[test]
fn generic_body_comptime_completion_offers_reflect() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    \n  }\n  label\n}\n",
    )
    .at(pos(2, 4))
    .expect_completion("reflect");
}

#[test]
fn generic_body_runtime_position_after_comptime_block_hides_reflect() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    type_ref(T)\n  }\n  \n  label\n}\n",
    )
    .at(pos(4, 2))
    .expect_no_completion("reflect");
}

/// R1 (representative category): reflecting a non-enabled category surfaces
/// the named per-category rejection through the LSP diagnostics path.
#[test]
fn reflect_non_enabled_category_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Array)) }\n")
        .expect_semantic_diagnostic_contains(
            "reflect: the Nominal payload descriptor has not landed",
        );
}

/// R2: the legacy string-kind form (`info.kind == "record"`) is a named
/// no-such-field rejection — descriptor schemas expose no `kind` field.
#[test]
fn reflect_string_kind_access_has_semantic_diagnostic() {
    ShapeTest::new(
        "let is_record = comptime {\n  let info = reflect(type_ref(int))\n  info.kind == \"record\"\n}\n",
    )
    .expect_semantic_diagnostic_contains("kind");
}

/// R4: a string argument is the named non-TypeRef rejection.
#[test]
fn reflect_string_argument_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { reflect(\"int\") }\n")
        .expect_semantic_diagnostic_contains("reflect expects a TypeRef value");
}

/// R5: runtime-position reflect is comptime-only.
#[test]
fn runtime_position_reflect_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = reflect(type_ref(int))\n")
        .expect_semantic_diagnostic_contains("comptime-only builtin");
}

// =====================================================================
// ADR-009 A3 (S5): LSP surface inside comptime blocks in GENERIC fn
// bodies. Generic template bodies are never compiled at definition
// (functions.rs:770-776 skip), so completion/hover MUST come from the
// shared query surface (builtin metadata + comptime-context detection),
// never from a compiled body. Semantic diagnostics come from the
// RecoverAll compile in analyze_program_semantics: specialized-body
// freeze errors surface through the S1 parameter overlay + S2 hard
// propagation when a call site instantiates the generic.
// =====================================================================

#[test]
fn generic_body_comptime_completion_offers_typed_reflection_builtins() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    \n  }\n  label\n}\n",
    )
    .at(pos(2, 4))
    .expect_completion("type_ref")
    .expect_completion("type_category");
}

#[test]
fn generic_body_runtime_position_after_comptime_block_hides_typed_reflection_builtins() {
    // Cursor is INSIDE the generic fn body but AFTER the comptime block has
    // closed — a runtime position. Comptime-only builtins must not leak here.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    type_ref(T)\n  }\n  \n  label\n}\n",
    )
    .at(pos(4, 2))
    .expect_no_completion("type_ref")
    .expect_no_completion("type_category");
}

#[test]
fn generic_body_frozen_category_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    match type_category(type_ref(T)) {\n      FrozenTypeCategory::\n    }\n  }\n  label\n}\n",
    )
    .at(pos(3, 26))
    .expect_completion("Parameter")
    .expect_completion("Primitive")
    .expect_no_completion("Unknown");
}

#[test]
fn generic_body_type_ref_hover_explains_opaque_identity() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(T) }\n  label\n}\n",
    )
    .at(pos(1, 26))
    .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn generic_body_type_category_hover_exposes_exhaustive_result() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_category(type_ref(T)) }\n  label\n}\n",
    )
    .at(pos(1, 28))
    .expect_hover_contains("exhaustive semantic category");
}

#[test]
fn generic_body_typed_reflection_offers_signature_help() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref( ) }\n  label\n}\n",
    )
    .at(pos(1, 34))
    .expect_signature_help();
}

#[test]
fn generic_body_string_type_ref_has_semantic_diagnostic() {
    // The specialized-body compile (triggered by the call site) must surface
    // the named rejection through the LSP diagnostic path.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(\"T\") }\n  \"ok\"\n}\nprint(describe(1))\n",
    )
    .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

#[test]
fn generic_body_undeclared_param_has_unknown_identity_diagnostic() {
    // type_ref(U) inside fn describe<T> — the freeze failure must reach LSP
    // diagnostics as the NAMED error, not the masked inference diagnostic.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(U) }\n  \"ok\"\n}\nprint(describe(1))\n",
    )
    .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

#[test]
fn generic_body_runtime_type_ref_use_has_comptime_only_diagnostic() {
    // Runtime use inside a generic body is rejected by the AST-walk validator
    // even though the template body never compiles (no call site needed).
    ShapeTest::new("fn describe<T>(value: T) -> string {\n  let label = type_ref(T)\n  \"ok\"\n}\n")
        .expect_semantic_diagnostic_contains("comptime-only builtin");
}

#[test]
fn generic_body_declared_param_reflection_is_diagnostic_clean() {
    // The S1 parameter overlay must reach the LSP compile path too: a
    // declared type param observed via type_ref(T)/type_category must
    // produce neither the freeze failure nor the masked inference error.
    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Primitive => "primitive"
      FrozenTypeCategory::Never => "never"
      FrozenTypeCategory::Parameter => "parameter"
      FrozenTypeCategory::Nominal => "nominal"
      FrozenTypeCategory::Tuple => "tuple"
      FrozenTypeCategory::Record => "record"
      FrozenTypeCategory::Callable => "callable"
      FrozenTypeCategory::Reference => "reference"
      FrozenTypeCategory::Union => "union"
      FrozenTypeCategory::Erased => "erased"
    }
  }
  label
}

print(describe(1))
"#;
    ShapeTest::new(source)
        .expect_no_semantic_diagnostic_contains("unknown semantic type identity")
        .expect_no_semantic_diagnostic_contains("cannot infer type argument");
}
