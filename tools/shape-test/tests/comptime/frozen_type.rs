use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

#[test]
fn type_ref_category_is_a_typed_exhaustive_enum() {
    let source = r#"
let label = comptime {
  match type_category(type_ref(int)) {
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

print(label)
"#;
    expect_vm_and_jit_output(source, "primitive");
}

#[test]
fn user_type_ref_category_is_nominal() {
    ShapeTest::new(
        r#"
type User { id: int }

let label = comptime {
  match type_category(type_ref(User)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("nominal");
}

#[test]
fn enum_and_builtin_container_types_are_nominal() {
    ShapeTest::new(
        r#"
enum Status { Ready, Done }

let enum_label = comptime {
  match type_category(type_ref(Status)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}
let array_label = comptime {
  match type_category(type_ref(Array)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(enum_label)
print(array_label)
"#,
    )
    .expect_output("nominal\nnominal");
}

#[test]
fn never_and_explicit_any_have_closed_categories() {
    ShapeTest::new(
        r#"
let never_label = comptime {
  match type_category(type_ref(never)) {
    FrozenTypeCategory::Never => "never"
    _ => "wrong"
  }
}
let erased_label = comptime {
  match type_category(type_ref(any)) {
    FrozenTypeCategory::Erased => "erased"
    _ => "wrong"
  }
}

print(never_label)
print(erased_label)
"#,
    )
    .expect_output("never\nerased");
}

/// ADR-009 §4.1 (ticket A1, slice S2) post-freeze semantics: the semantic
/// freeze is built ONCE per compilation unit at the registration-complete
/// barrier, so a comptime block sees every type declared anywhere in the
/// unit — including aliases and enums declared textually AFTER the block.
/// (Structs were already order-independent via the schema predeclare
/// prepass; aliases and enums previously registered only when their item
/// compiled in pass 2, so the old per-site snapshot rebuild missed them.)
#[test]
fn later_declared_alias_and_enum_are_visible_to_earlier_comptime_blocks() {
    let source = r#"
let alias_label = comptime {
  match type_category(type_ref(LaterUserId)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "wrong"
  }
}
let enum_label = comptime {
  match type_category(type_ref(LaterStatus)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

type LaterUserId = int
enum LaterStatus { Ready, Done }

print(alias_label)
print(enum_label)
"#;
    expect_vm_and_jit_output(source, "primitive\nnominal");
}

#[test]
fn strings_cannot_construct_type_refs() {
    ShapeTest::new(
        r#"
let reflected = comptime { type_ref("int") }
print(reflected)
"#,
    )
    .expect_run_err_contains("strings cannot construct TypeRef");
}

#[test]
fn transparent_alias_uses_underlying_type_identity() {
    ShapeTest::new(
        r#"
type UserId = int

let label = comptime {
  match type_category(type_ref(UserId)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("primitive");
}

#[test]
fn transparent_alias_chains_preserve_nominal_category() {
    ShapeTest::new(
        r#"
type User { id: int }
type Account = User
type CurrentAccount = Account

let label = comptime {
  match type_category(type_ref(CurrentAccount)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("nominal");
}

#[test]
fn unresolved_type_cannot_cross_freeze_boundary() {
    ShapeTest::new(
        r#"
let reflected = comptime { type_ref(DoesNotExist) }
print(reflected)
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

#[test]
fn non_type_expressions_cannot_construct_type_refs() {
    ShapeTest::new("let reflected = comptime { type_ref(42) }")
        .expect_run_err_contains("expects compiler-resolved type syntax");
}

#[test]
fn type_ref_requires_exactly_one_type_argument() {
    for source in [
        "let reflected = comptime { type_ref() }",
        "let reflected = comptime { type_ref(int, bool) }",
    ] {
        ShapeTest::new(source).expect_run_err_contains("expects exactly one type argument");
    }
}

#[test]
fn type_ref_is_comptime_only() {
    ShapeTest::new("let reflected = type_ref(int)")
        .expect_run_err_contains("comptime-only builtin");
}

#[test]
fn legacy_reflection_descriptors_cannot_forge_type_refs() {
    ShapeTest::new("let category = comptime { type_category(type_info(int).type_ref) }")
        .expect_run_err_contains_any(&["TypeRef", "not compatible", "do not unify"]);
}

#[test]
fn arbitrary_values_cannot_be_used_as_type_refs() {
    ShapeTest::new("let category = comptime { type_category(42) }").expect_run_err_contains_any(&[
        "TypeRef",
        "not compatible",
        "do not unify",
    ]);
}

#[test]
fn raw_type_refs_cannot_escape_to_runtime_code() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\nprint(reflected)")
        .expect_run_err_contains("TypeRef is a comptime-only compiler capability");
}

#[test]
fn raw_frozen_categories_cannot_escape_to_runtime_code() {
    ShapeTest::new("let category = comptime { type_category(type_ref(int)) }\nprint(category)")
        .expect_run_err_contains("FrozenTypeCategory is comptime-only reflection data");
}

#[test]
fn category_matches_are_checked_for_exhaustiveness() {
    ShapeTest::new(
        r#"
let label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
  }
}
print(label)
"#,
    )
    .expect_run_err_contains("Non-exhaustive match");
}
