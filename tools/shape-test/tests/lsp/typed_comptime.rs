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
