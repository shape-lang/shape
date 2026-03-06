//! Type alias tests — simple aliases, struct aliases, generic aliases,
//! intersection types, union types, and meta-parameter overrides.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 2. Type aliases (15 tests)
// =========================================================================

#[test]
fn type_alias_simple_parses() {
    ShapeTest::new(
        r#"
        type Percent = number
        let x: Percent = 0.15
        x
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_struct_parses() {
    ShapeTest::new(
        r#"
        type Point { x: int, y: int }
        type P = Point
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_object_type_parses() {
    ShapeTest::new(
        r#"
        type Coord = { x: number, y: number }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_chain_parses() {
    ShapeTest::new(
        r#"
        type A = number
        type B = A
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_in_function_signature_parses() {
    ShapeTest::new(
        r#"
        type Score = number
        fn get_score() -> Score {
            return 100
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_in_param_annotation_parses() {
    ShapeTest::new(
        r#"
        type Id = int
        fn lookup(id: Id) -> string {
            return "found"
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_of_array_type_parses() {
    ShapeTest::new(
        r#"
        type IntList = Array<int>
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_with_meta_param_override_parses() {
    ShapeTest::new(
        r#"
        type Percent4 = Percent { decimals: 4 }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_with_multiple_overrides_parses() {
    ShapeTest::new(
        r#"
        type EUR = Currency { symbol: "€", decimals: 2 }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_intersection_parses() {
    ShapeTest::new(
        r#"
        type Combined = TypeA + TypeB
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_intersection_objects_parses() {
    ShapeTest::new(
        r#"
        type Combined = { x: number } + { y: string }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_union_parses() {
    ShapeTest::new(
        r#"
        type StringOrInt = string | int
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_union_intersection_combined_parses() {
    ShapeTest::new(
        r#"
        type Mixed = A | B + C
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_optional_field_parses() {
    ShapeTest::new(
        r#"
        type Config = { host: string, port?: int }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn type_alias_generic_parses() {
    ShapeTest::new(
        r#"
        type Container<T> { value: T }
    "#,
    )
    .expect_parse_ok();
}
