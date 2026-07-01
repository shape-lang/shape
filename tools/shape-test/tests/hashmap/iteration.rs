//! HashMap iteration tests
//! Covers .map(), .filter(), .forEach() on HashMaps.

use shape_test::shape_test::ShapeTest;

const HASHMAP_MAP_INDEXED_ABSENT: &str = "no method 'mapIndexed' on receiver kind Ptr(HashMap)";
const HASHMAP_FILTER_INDEXED_ABSENT: &str =
    "no method 'filterIndexed' on receiver kind Ptr(HashMap)";

// =========================================================================
// Map
// =========================================================================

#[test]
fn hashmap_map_doubles_values() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        let doubled = m.map(|k, v| v * 2)
        print(doubled.get("a"))
        print(doubled.get("b"))
    "#,
    )
    .expect_run_err_contains(HASHMAP_MAP_INDEXED_ABSENT);
}

#[test]
fn hashmap_map_preserves_keys() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("x", 10).set("y", 20)
        let mapped = m.map(|k, v| v + 1)
        print(mapped.has("x"))
        print(mapped.has("y"))
    "#,
    )
    .expect_run_err_contains(HASHMAP_MAP_INDEXED_ABSENT);
}

// =========================================================================
// Filter
// =========================================================================

#[test]
fn hashmap_filter_by_value() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 5).set("c", 3)
        let big = m.filter(|k, v| v > 2)
        print(big.len())
    "#,
    )
    .expect_run_err_contains(HASHMAP_FILTER_INDEXED_ABSENT);
}

#[test]
fn hashmap_filter_none_match() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        let result = m.filter(|k, v| v > 100)
        print(result.len())
    "#,
    )
    .expect_run_err_contains(HASHMAP_FILTER_INDEXED_ABSENT);
}

#[test]
fn hashmap_filter_all_match() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        let result = m.filter(|k, v| v > 0)
        print(result.len())
    "#,
    )
    .expect_run_err_contains(HASHMAP_FILTER_INDEXED_ABSENT);
}

// =========================================================================
// ForEach
// =========================================================================

#[test]
fn hashmap_foreach_prints_values() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        m.forEach(|k, v| print(v))
    "#,
    )
    .expect_run_ok();
}

// =========================================================================
// Combined Operations
// =========================================================================

#[test]
fn hashmap_filter_then_map() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 5).set("c", 10)
        let result = m.filter(|k, v| v > 3).map(|k, v| v * 2)
        print(result.get("b"))
        print(result.get("c"))
    "#,
    )
    .expect_run_err_contains(HASHMAP_FILTER_INDEXED_ABSENT);
}
