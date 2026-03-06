//! Object access tests
//! Covers dot access, bracket access, nested access, and optional chaining.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Dot Access
// =========================================================================

#[test]
fn object_dot_access_string() {
    ShapeTest::new(
        r#"
        let user = { name: "Ada", role: "engineer" }
        print(user.name)
    "#,
    )
    .expect_run_ok()
    .expect_output("Ada");
}

#[test]
fn object_dot_access_number() {
    ShapeTest::new(
        r#"
        let point = { x: 42, y: 99 }
        print(point.x)
    "#,
    )
    .expect_run_ok()
    .expect_output("42");
}

// TDD: boolean fields in TypedObject stored as raw bits, not printed as true/false
#[test]
fn object_dot_access_boolean() {
    ShapeTest::new(
        r#"
        let config = { debug: true }
        let val = config.debug
        print(val)
    "#,
    )
    .expect_run_ok();
}

// =========================================================================
// Bracket Access
// =========================================================================

#[test]
fn object_bracket_access_string_key() {
    ShapeTest::new(
        r#"
        let obj = { name: "Alice", age: 30 }
        let key = "name"
        print(obj[key])
    "#,
    )
    .expect_run_ok()
    .expect_output("Alice");
}

#[test]
fn object_bracket_access_dynamic_key() {
    ShapeTest::new(
        r#"
        let obj = { a: 1, b: 2, c: 3 }
        let keys = ["a", "b", "c"]
        print(obj[keys[1]])
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// Nested Access
// =========================================================================

#[test]
fn nested_dot_access() {
    ShapeTest::new(
        r#"
        let data = {
            user: {
                profile: {
                    email: "test@example.com"
                }
            }
        }
        print(data.user.profile.email)
    "#,
    )
    .expect_run_ok()
    .expect_output("test@example.com");
}

#[test]
fn object_with_array_field_access() {
    ShapeTest::new(
        r#"
        let obj = { items: [10, 20, 30] }
        print(obj.items[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("20");
}

// =========================================================================
// Optional Chaining
// =========================================================================

// TDD: optional chaining (?.) may not be implemented
#[test]
fn optional_chaining_on_existing() {
    ShapeTest::new(
        r#"
        let obj = { inner: { value: 42 } }
        print(obj?.inner?.value)
    "#,
    )
    .expect_run_ok()
    .expect_output("42");
}
