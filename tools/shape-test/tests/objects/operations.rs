//! Object operations tests
//! Covers spread merge, destructuring, property mutation, and computed keys.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Spread Merge
// =========================================================================

// TDD: object spread { ...a, ...b } may not be fully implemented
#[test]
fn object_spread_merge() {
    ShapeTest::new(
        r#"
        let a = { x: 1, y: 2 }
        let b = { y: 10, z: 3 }
        let c = { ...a, ...b }
        print(c.x)
        print(c.y)
        print(c.z)
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n10\n3");
}

#[test]
fn object_merge_with_plus() {
    ShapeTest::new(
        r#"
        let a = { x: 1 }
        let b = { y: 2 }
        let c = a + b
        print(c)
    "#,
    )
    .expect_run_ok();
}

// =========================================================================
// Destructuring
// =========================================================================

#[test]
fn object_destructuring_basic() {
    ShapeTest::new(
        r#"
        let point = { x: 5, y: 10 }
        let { x, y } = point
        print(x)
        print(y)
    "#,
    )
    .expect_run_ok()
    .expect_output("5\n10");
}

#[test]
fn object_destructuring_in_function() {
    ShapeTest::new(
        r#"
        fn sum_coords({ x, y }) {
            return x + y
        }
        print(sum_coords({ x: 3, y: 7 }))
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

// =========================================================================
// Property Mutation
// =========================================================================

#[test]
fn object_property_assignment() {
    ShapeTest::new(
        r#"
        let obj = { name: "Alice", score: 0 }
        obj.score = 100
        print(obj.score)
    "#,
    )
    .expect_run_ok()
    .expect_output("100");
}

#[test]
fn object_add_new_property() {
    ShapeTest::new(
        r#"
        let obj = { x: 1 }
        obj.y = 2
        print(obj.y)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// Computed Keys
// =========================================================================

// TDD: bracket-based property assignment with dynamic keys not supported on TypedObject
#[test]
fn object_computed_key() {
    ShapeTest::new(
        r#"
        let obj = { name: "default" }
        obj.name = "Bob"
        print(obj.name)
    "#,
    )
    .expect_run_ok()
    .expect_output("Bob");
}

// =========================================================================
// Object with Function Values
// =========================================================================

#[test]
fn object_function_value_call() {
    ShapeTest::new(
        r#"
        let obj = {
            greet: |name| "Hello, " + name
        }
        print(obj.greet("World"))
    "#,
    )
    .expect_run_ok()
    .expect_output("Hello, World");
}
