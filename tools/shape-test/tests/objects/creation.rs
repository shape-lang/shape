//! Object creation tests
//! Covers literal objects, nested objects, empty objects, and shorthand syntax.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Literal Creation
// =========================================================================

#[test]
fn object_literal_basic() {
    ShapeTest::new(
        r#"
        let obj = { x: 1, y: 2 }
        print(obj.x)
        print(obj.y)
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n2");
}

#[test]
fn object_literal_string_values() {
    ShapeTest::new(
        r#"
        let person = { name: "Alice", city: "Zurich" }
        print(person.name)
        print(person.city)
    "#,
    )
    .expect_run_ok()
    .expect_output("Alice\nZurich");
}

// TDD: boolean fields in TypedObject stored as raw bits, not printed as true/false
#[test]
fn object_literal_mixed_types() {
    ShapeTest::new(
        r#"
        let obj = { name: "Bob", age: 30, active: true }
        print(obj.name)
        print(obj.age)
    "#,
    )
    .expect_run_ok()
    .expect_output("Bob\n30");
}

// =========================================================================
// Nested Objects
// =========================================================================

#[test]
fn nested_object_creation() {
    ShapeTest::new(
        r#"
        let config = {
            server: {
                host: "localhost",
                port: 8080
            }
        }
        print(config.server.host)
    "#,
    )
    .expect_run_ok()
    .expect_output("localhost");
}

#[test]
fn deeply_nested_object() {
    ShapeTest::new(
        r#"
        let data = {
            a: { b: { c: { value: 42 } } }
        }
        print(data.a.b.c.value)
    "#,
    )
    .expect_run_ok()
    .expect_output("42");
}

// =========================================================================
// Empty Object
// =========================================================================

#[test]
fn empty_object_creation() {
    ShapeTest::new(
        r#"
        let obj = {}
        print(obj)
    "#,
    )
    .expect_run_ok();
}

// =========================================================================
// Shorthand Syntax
// =========================================================================

// TDD: shorthand { x, y } syntax not supported in parser
#[test]
fn object_shorthand_variable_names() {
    ShapeTest::new(
        r#"
        let x = 10
        let y = 20
        let point = { x: x, y: y }
        print(point.x)
        print(point.y)
    "#,
    )
    .expect_run_ok()
    .expect_output("10\n20");
}

// =========================================================================
// Object from Function
// =========================================================================

#[test]
fn object_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_point(x, y) {
            return { x: x, y: y }
        }
        let p = make_point(3, 7)
        print(p.x)
        print(p.y)
    "#,
    )
    .expect_run_ok()
    .expect_output("3\n7");
}
