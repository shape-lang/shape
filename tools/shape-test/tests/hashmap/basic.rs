//! HashMap basic operation tests
//! Covers creation, get, set, has, delete, and empty hashmap operations.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Creation
// =========================================================================

#[test]
fn hashmap_create_empty() {
    // ROOT B (HashMap <K,V> infer-or-annotate): a bare `let m = HashMap()`
    // whose key/value types are never pinned by usage is an un-pinnable
    // generic construction and is rejected under strict typing (sibling to the
    // empty-array `let a: Array<int> = []` remedy). The working-map intent here
    // is an empty map, so the K,V are supplied by an explicit annotation.
    // `<string, bool>` keeps the empty map on the legacy-ctor path: the
    // typed-map fast path (int/number values) lacks an empty-map `.len()`
    // lowering for typed-map locals — a separate VM-codegen gap, not ROOT B.
    ShapeTest::new(
        r#"
        let m: HashMap<string, bool> = HashMap()
        print(m.len())
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

#[test]
fn hashmap_create_and_set() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        print(m.len())
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// Get
// =========================================================================

#[test]
fn hashmap_get_existing_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("name", "Alice").set("age", 30)
        print(m.get("name"))
    "#,
    )
    .expect_run_ok()
    .expect_output("Alice");
}

#[test]
fn hashmap_get_missing_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1)
        let v = m.get("missing")
        print(v)
    "#,
    )
    .expect_run_ok();
}

#[test]
fn hashmap_get_integer_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set(1, "gold").set(2, "silver")
        print(m.get(1))
    "#,
    )
    .expect_run_ok()
    .expect_output("gold");
}

// =========================================================================
// Has
// =========================================================================

#[test]
fn hashmap_has_existing() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("key", "value")
        print(m.has("key"))
    "#,
    )
    .expect_run_ok()
    .expect_output("true");
}

#[test]
fn hashmap_has_missing() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("key", "value")
        print(m.has("other"))
    "#,
    )
    .expect_run_ok()
    .expect_output("false");
}

// =========================================================================
// Delete
// =========================================================================

#[test]
fn hashmap_delete_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        let m2 = m.delete("b")
        print(m2.len())
        print(m2.has("b"))
    "#,
    )
    .expect_run_ok()
    .expect_output("2\nfalse");
}

// =========================================================================
// Empty Check
// =========================================================================

#[test]
fn hashmap_is_empty_true() {
    // ROOT B (HashMap <K,V> infer-or-annotate): bare `let m = HashMap()` with
    // no usage to pin K,V is rejected under strict typing; supply them via an
    // explicit annotation for the empty-map intent. `<string, bool>` keeps the
    // empty map on the legacy-ctor path (see hashmap_create_empty).
    ShapeTest::new(
        r#"
        let m: HashMap<string, bool> = HashMap()
        print(m.isEmpty())
    "#,
    )
    .expect_run_ok()
    .expect_output("true");
}

#[test]
fn hashmap_is_empty_false() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("x", 1)
        print(m.isEmpty())
    "#,
    )
    .expect_run_ok()
    .expect_output("false");
}

// =========================================================================
// Immutability
// =========================================================================

#[test]
fn hashmap_set_returns_new_map() {
    ShapeTest::new(
        r#"
        let m = HashMap()
        let m2 = m.set("a", 1)
        print(m.len())
        print(m2.len())
    "#,
    )
    .expect_run_ok()
    .expect_output("0\n1");
}

// =========================================================================
// Overwrite
// =========================================================================

#[test]
fn hashmap_overwrite_existing_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("a", 99)
        print(m.get("a"))
    "#,
    )
    .expect_run_ok()
    .expect_output("99");
}
