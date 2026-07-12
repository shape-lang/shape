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
