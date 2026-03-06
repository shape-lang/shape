//! Query clause tests
//! Covers let clauses, orderBy, and groupBy in LINQ-style queries.
//! Many of these are TDD — the semantic analyzer does not track variables
//! introduced by let/join clauses, and groupBy/descending are not fully parsed.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Let Clauses
// =========================================================================

// TDD: Semantic analyzer does not track variables introduced by let-clauses in from-queries
#[test]
fn query_let_clause_basic() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3] let doubled = x * 2 select doubled
        print(result[0])
        print(result[1])
        print(result[2])
    "#,
    )
    .expect_run_err_contains("Undefined variable");
}

// TDD: Semantic analyzer does not track variables introduced by let-clauses in from-queries
#[test]
fn query_let_clause_with_where() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5]
            let sq = x * x
            where sq > 10
            select sq
        print(result[0])
    "#,
    )
    .expect_run_err_contains("Undefined variable");
}

// TDD: Semantic analyzer does not track variables introduced by let-clauses in from-queries
#[test]
fn query_multiple_let_clauses() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3]
            let doubled = x * 2
            let tripled = x * 3
            select doubled + tripled
        print(result[0])
    "#,
    )
    .expect_run_err_contains("Undefined variable");
}

// =========================================================================
// OrderBy
// =========================================================================

#[test]
fn query_order_by_ascending() {
    ShapeTest::new(
        r#"
        let result = from x in [3, 1, 4, 1, 5] orderby x select x
        print(result[0])
        print(result[4])
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n5");
}

// TDD: descending keyword not fully parsed in orderby clause
#[test]
fn query_order_by_descending() {
    ShapeTest::new(
        r#"
        let result = from x in [3, 1, 4, 1, 5] orderby x descending select x
        print(result[0])
    "#,
    )
    .expect_run_err_contains("expected something else");
}

// =========================================================================
// GroupBy
// =========================================================================

// TDD: groupBy with expressions not fully parsed
#[test]
fn query_group_by_parity() {
    ShapeTest::new(
        r#"
        let result = from x in [1, 2, 3, 4, 5, 6] group x by x % 2
        print(result)
    "#,
    )
    .expect_run_err_contains("expected something else");
}

// TDD: groupBy with into clause not fully parsed
#[test]
fn query_group_by_with_select() {
    ShapeTest::new(
        r#"
        let data = [1, 2, 3, 4, 5, 6]
        let result = from x in data group x by x % 2 into g select g
        print(result.length)
    "#,
    )
    .expect_run_err_contains("expected something else");
}
