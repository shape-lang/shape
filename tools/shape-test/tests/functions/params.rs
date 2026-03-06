//! Function parameter tests.
//!
//! Covers: default params, type-annotated params, multi-return (tuple).

use shape_test::shape_test::ShapeTest;

#[test]
fn default_parameter() {
    ShapeTest::new(
        r#"
        fn greet(name = "world") {
            "hello " + name
        }
        greet()
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn default_parameter_overridden() {
    ShapeTest::new(
        r#"
        fn greet(name = "world") {
            "hello " + name
        }
        greet("Shape")
    "#,
    )
    .expect_string("hello Shape");
}

#[test]
fn multiple_default_params() {
    ShapeTest::new(
        r#"
        fn make_point(x = 0, y = 0) {
            x + y
        }
        make_point()
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn partial_default_params() {
    ShapeTest::new(
        r#"
        fn add(a, b = 10) {
            a + b
        }
        add(5)
    "#,
    )
    .expect_number(15.0);
}

// TDD: type-annotated params may not be enforced at runtime
#[test]
fn type_annotated_params() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) {
            a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

// TDD: return type annotation may not be enforced at runtime
#[test]
fn return_type_annotation() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int {
            x * 2
        }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

// TDD: multi-return / tuple return may not be supported
#[test]
fn multi_return_array() {
    ShapeTest::new(
        r#"
        fn min_max(arr) {
            var lo = arr[0]
            var hi = arr[0]
            for x in arr {
                if x < lo { lo = x }
                if x > hi { hi = x }
            }
            [lo, hi]
        }
        let result = min_max([3, 1, 4, 1, 5])
        result[0]
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn many_parameters() {
    ShapeTest::new(
        r#"
        fn sum5(a, b, c, d, e) {
            a + b + c + d + e
        }
        sum5(1, 2, 3, 4, 5)
    "#,
    )
    .expect_number(15.0);
}
