//! Advanced query language tests
//! Covers join, multi-from, into, and query composition.
//! Many of these are TDD — semantic analyzer does not track join variables,
//! multi-from and into clauses are not parsed.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Join
// =========================================================================

// TDD: Semantic analyzer does not track variables introduced by join clauses
#[test]
fn query_join_two_collections() {
    ShapeTest::new(
        r#"
        let ids = [1, 2, 3]
        let names = [{ id: 1, name: "Alice" }, { id: 2, name: "Bob" }, { id: 3, name: "Charlie" }]
        let result = from i in ids
            join n in names on i equals n.id
            select n.name
        print(result[0])
    "#,
    )
    .expect_run_err_contains("Undefined variable");
}

// TDD: Semantic analyzer does not track variables introduced by join clauses
#[test]
fn query_join_with_filter() {
    ShapeTest::new(
        r#"
        let orders = [{ id: 1, amount: 100 }, { id: 2, amount: 50 }]
        let customers = [{ id: 1, name: "Alice" }, { id: 2, name: "Bob" }]
        let result = from o in orders
            join c in customers on o.id equals c.id
            where o.amount > 60
            select c.name
        print(result.length)
    "#,
    )
    .expect_run_err_contains("Undefined variable");
}

// =========================================================================
// Multi-From (Cross Join)
// =========================================================================

// TDD: multi-from syntax not supported in parser
#[test]
fn query_multi_from_cross_product() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2]
            from y in [10, 20]
            select x + y
        print(result.length)
    "#,
    )
    .expect_run_err_contains("expected something else");
}

// TDD: multi-from syntax not supported in parser
#[test]
fn query_multi_from_with_filter() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3]
            from y in [1, 2, 3]
            where x != y
            select x * 10 + y
        print(result.length)
    "#,
    )
    .expect_run_err_contains("expected something else");
}

// =========================================================================
// Into (Group Result Binding)
// =========================================================================

// TDD: into clause with group-by not fully parsed
#[test]
fn query_into_basic() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5, 6]
            group x by x % 2 into g
            select g
        print(result.length)
    "#,
    )
    .expect_run_err_contains("expected something else");
}

// =========================================================================
// Query Composition
// =========================================================================

#[test]
fn query_nested_in_expression() {
    ShapeTest::new(
        r#"
        let evens = from x in [1, 2, 3, 4, 5, 6] where x % 2 == 0 select x
        let doubled = from x in evens select x * 2
        print(doubled[0])
        print(doubled[1])
        print(doubled[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("4\n8\n12");
}

#[test]
fn query_result_used_with_methods() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5] where x > 2 select x * 10
        print(result.length)
        print(result[0])
    "#,
    )
    .expect_run_ok()
    .expect_output("3\n30");
}

#[test]
fn query_over_objects() {
    ShapeTest::new(
        r#"
        let users = [
            { name: "Alice", age: 30 },
            { name: "Bob", age: 25 },
            { name: "Charlie", age: 35 }
        ]
        let result = from u in users where u.age >= 30 select u.name
        print(result.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}
