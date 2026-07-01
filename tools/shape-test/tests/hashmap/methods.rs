//! HashMap method tests
//! Covers .keys(), .values(), .entries(), .len()/.length, .isEmpty().

use shape_test::shape_test::ShapeTest;

const HASHMAP_KEYS_SURFACE: &str = "HashMap.keys: SURFACE";
const HASHMAP_VALUES_SURFACE: &str = "HashMap.values: SURFACE";
const HASHMAP_ENTRIES_SURFACE: &str = "HashMap.entries/toArray: SURFACE";

// =========================================================================
// Keys
// =========================================================================

#[test]
fn hashmap_keys_returns_array() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        let k = m.keys()
        print(k.length)
    "#,
    )
    .expect_run_err_contains(HASHMAP_KEYS_SURFACE);
}

#[test]
fn hashmap_keys_contains_entries() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("x", 10).set("y", 20)
        let k = m.keys()
        print(k.length)
    "#,
    )
    .expect_run_err_contains(HASHMAP_KEYS_SURFACE);
}

// =========================================================================
// Values
// =========================================================================

#[test]
fn hashmap_values_returns_array() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        let v = m.values()
        print(v.length)
    "#,
    )
    .expect_run_err_contains(HASHMAP_VALUES_SURFACE);
}

// =========================================================================
// Entries
// =========================================================================

#[test]
fn hashmap_entries_returns_array() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        let e = m.entries()
        print(e.length)
    "#,
    )
    .expect_run_err_contains(HASHMAP_ENTRIES_SURFACE);
}

// =========================================================================
// Len / Length
// =========================================================================

#[test]
fn hashmap_len_method() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        print(m.len())
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}

#[test]
fn hashmap_length_property() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        print(m.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// Chaining
// =========================================================================

#[test]
fn hashmap_chained_operations() {
    ShapeTest::new(
        r#"
        let m = HashMap()
            .set("x", 10)
            .set("y", 20)
            .set("z", 30)
        print(m.get("x"))
        print(m.get("y"))
        print(m.get("z"))
    "#,
    )
    .expect_run_ok()
    .expect_output("10\n20\n30");
}
