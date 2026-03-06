//! Tests for passing closures to/from functions.
//!
//! Covers: closures as arguments, returning closures from functions.

use shape_test::shape_test::ShapeTest;

#[test]
fn closure_as_argument() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        let double = |x| x * 2
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn inline_closure_as_argument() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x + 10, 32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn closure_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_adder(n) { |x| x + n }
        let add5 = make_adder(5)
        add5(37)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn closure_factory_multiple() {
    ShapeTest::new(
        r#"
        fn make_multiplier(factor) { |x| x * factor }
        let double = make_multiplier(2)
        let triple = make_multiplier(3)
        double(10) + triple(10)
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn higher_order_map_style() {
    ShapeTest::new(
        r#"
        fn apply_to_each(arr, f) {
            var result = []
            for x in arr {
                result = result.push(f(x))
            }
            result.length
        }
        apply_to_each([1, 2, 3], |x| x * 2)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_composition() {
    ShapeTest::new(
        r#"
        fn compose(f, g) {
            |x| f(g(x))
        }
        let add1 = |x| x + 1
        let double = |x| x * 2
        let add1_then_double = compose(double, add1)
        add1_then_double(5)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn closure_with_array_filter() {
    ShapeTest::new(
        r#"
        let evens = [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
        evens.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_with_array_map() {
    ShapeTest::new(
        r#"
        let doubled = [1, 2, 3].map(|x| x * 2)
        doubled[0] + doubled[1] + doubled[2]
    "#,
    )
    .expect_number(12.0);
}
