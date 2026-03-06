//! Tests for table/array methods that underpin the query system.
//!
//! The from..in..select syntax desugars to filter/map/orderBy method chains.
//! These tests verify those methods work correctly on arrays (the most common
//! Queryable source in user code).

use shape_test::shape_test::ShapeTest;

// =========================================================================
// filter (where clause target)
// =========================================================================

#[test]
fn filter_array_with_predicate() {
    ShapeTest::new(
        r#"
        let nums = [1, 2, 3, 4, 5]
        let evens = nums.filter(|x| x % 2 == 0)
        print(evens.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

#[test]
fn filter_objects_by_field() {
    ShapeTest::new(
        r#"
        let items = [
            { name: "a", value: 10 },
            { name: "b", value: 25 },
            { name: "c", value: 5 }
        ]
        let expensive = items.filter(|item| item.value > 9)
        print(expensive.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// map (select clause target)
// =========================================================================

#[test]
fn map_array_to_transformed_values() {
    ShapeTest::new(
        r#"
        let nums = [1, 2, 3]
        let doubled = nums.map(|x| x * 2)
        print(doubled[0])
        print(doubled[1])
        print(doubled[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n4\n6");
}

#[test]
fn map_extract_field_from_objects() {
    ShapeTest::new(
        r#"
        let users = [
            { name: "Alice", age: 30 },
            { name: "Bob", age: 25 }
        ]
        let names = users.map(|u| u.name)
        print(names[0])
        print(names[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("Alice\nBob");
}

// =========================================================================
// filter + map chain (where + select)
// =========================================================================

#[test]
fn filter_then_map_chain() {
    ShapeTest::new(
        r#"
        let nums = [1, 2, 3, 4, 5, 6]
        let result = nums.filter(|x| x > 3).map(|x| x * 10)
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("40\n50\n60");
}
