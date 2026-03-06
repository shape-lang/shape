//! Basic LINQ-style query language tests
//! Covers from..in..select, from..where..select basics.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic from..in..select
// =========================================================================

#[test]
fn query_from_select_identity() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] select x
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n2\n3");
}

#[test]
fn query_from_select_transform() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] select x * 10
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("10\n20\n30");
}

#[test]
fn query_from_select_to_object() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] select { val: x }
        print(result[0].val)
    "#,
    )
    .expect_run_ok()
    .expect_output("1");
}

// =========================================================================
// from..where..select
// =========================================================================

#[test]
fn query_from_where_select_filter() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5] where x > 3 select x
        print(result.length)
        print(result[0])
        print(result[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n5");
}

#[test]
fn query_from_where_select_with_transform() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5] where x % 2 == 0 select x * 100
        print(result[0])
        print(result[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("200\n400");
}

#[test]
fn query_from_where_no_matches() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] where x > 100 select x
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

// =========================================================================
// Query over variables
// =========================================================================

#[test]
fn query_from_variable_collection() {
    ShapeTest::new(
        r#"
        let nums = [10, 20, 30, 40, 50]
        let result = from n in nums where n >= 30 select n
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}

#[test]
fn query_result_is_array() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] select x * 2
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}
