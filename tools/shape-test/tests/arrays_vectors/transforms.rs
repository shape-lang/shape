//! Array transformation tests
//! Covers .map(), .filter(), .reduce(), .forEach(), .flatMap(), .zip().

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Map
// =========================================================================

#[test]
fn array_map_double() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let doubled = arr.map(|x| x * 2)
        print(doubled[0])
        print(doubled[1])
        print(doubled[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n6");
}

#[test]
fn array_map_to_objects() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let objs = arr.map(|x| { value: x * 10 })
        print(objs[0].value)
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

// =========================================================================
// Filter
// =========================================================================

#[test]
fn array_filter_positive() {
    ShapeTest::new(
        r#"
        let arr = [-2, -1, 0, 1, 2]
        let pos = arr.filter(|x| x > 0)
        print(pos.length)
        print(pos[0])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n1");
}

#[test]
fn array_filter_none_match() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let result = arr.filter(|x| x > 100)
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

// =========================================================================
// Reduce
// =========================================================================

#[test]
fn array_reduce_sum() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4]
        let sum = arr.reduce(|acc, x| acc + x, 0)
        print(sum)
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

#[test]
fn array_reduce_product() {
    ShapeTest::new(
        r#"
        let arr = [2, 3, 4]
        let product = arr.reduce(|acc, x| acc * x, 1)
        print(product)
    "#,
    )
    .expect_run_ok()
    .expect_output("24");
}

// =========================================================================
// ForEach
// =========================================================================

#[test]
fn array_foreach_prints_each() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30]
        arr.forEach(|x| print(x))
    "#,
    )
    .expect_output("10\n20\n30");
}

// =========================================================================
// FlatMap
// =========================================================================

#[test]
fn array_flatmap_expand() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let result = arr.flatMap(|x| [x, x * 10])
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("6");
}

#[test]
fn array_flatmap_values() {
    ShapeTest::new(
        r#"
        let arr = [1, 2]
        let result = arr.flatMap(|x| [x, x * 10])
        print(result[0])
        print(result[1])
        print(result[2])
        print(result[3])
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n10\n2\n20");
}

// =========================================================================
// Zip
// =========================================================================

// TDD: zip method may not be implemented on arrays
#[test]
fn array_zip_two_arrays() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        let b = ["a", "b", "c"]
        let zipped = a.zip(b)
        print(zipped.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}

// =========================================================================
// Chaining Transforms
// =========================================================================

#[test]
fn array_filter_then_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5, 6]
        let result = arr.filter(|x| x % 2 == 0).map(|x| x * 10)
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("20\n40\n60");
}
