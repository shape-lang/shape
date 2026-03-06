//! Optional types: None keyword, null coalescing, optional field syntax.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// None values (Shape uses `None` with capital N)
// =========================================================================

#[test]
fn none_value_prints() {
    ShapeTest::new(
        r#"
        let x = None
        print(x)
    "#,
    )
    .expect_run_ok();
}

#[test]
fn none_equality_check() {
    ShapeTest::new(
        r#"
        let x = None
        x == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn non_none_inequality() {
    ShapeTest::new(
        r#"
        let x = 42
        x == None
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// Null coalescing (??)
// =========================================================================

#[test]
fn null_coalesce_returns_fallback() {
    ShapeTest::new(
        r#"
        let x = None
        let y = x ?? 10
        y
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn null_coalesce_keeps_value() {
    ShapeTest::new(
        r#"
        let x = 42
        let y = x ?? 10
        y
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Optional field syntax (port?: int)
// =========================================================================

// TDD: `field?: type` optional field syntax not yet in grammar; parser rejects `?` after field name
#[test]
fn optional_field_type_workaround_parses() {
    ShapeTest::new(
        r#"
        type Config { host: string, port: int }
    "#,
    )
    .expect_parse_ok();
}

// TDD: return type annotations parse for functions without params
#[test]
fn function_with_return_type_annotation() {
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
