//! Stress tests for HashMap creation and construction patterns.

use shape_test::shape_test::ShapeTest;

/// Verifies creating an empty HashMap returns a valid HashMap.
#[test]
fn test_hashmap_create_empty() {
    ShapeTest::new(
        r#"
        let m = HashMap()
        print(m.len())
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

/// Verifies empty HashMap has length zero.
#[test]
fn test_hashmap_empty_len_is_zero() {
    ShapeTest::new("HashMap().len()").expect_number(0.0);
}

/// Verifies isEmpty returns true for empty HashMap.
#[test]
fn test_hashmap_empty_is_empty_true() {
    ShapeTest::new("HashMap().isEmpty()").expect_bool(true);
}

/// Verifies HashMap with a single entry has length 1.
#[test]
fn test_hashmap_single_entry() {
    ShapeTest::new(r#"HashMap().set("a", 1).len()"#).expect_number(1.0);
}

/// Verifies HashMap with multiple entries has correct length.
#[test]
fn test_hashmap_multiple_entries() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).set("c", 3).len()"#).expect_number(3.0);
}

/// Verifies chained set returns a non-empty HashMap.
#[test]
fn test_hashmap_chained_set_returns_hashmap() {
    ShapeTest::new(r#"HashMap().set("x", 10).set("y", 20).isEmpty()"#).expect_bool(false);
}

/// Verifies HashMap can be created inside a block.
#[test]
fn test_hashmap_created_in_block() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("key", "val")
        m.get("key")
    }"#,
    )
    .expect_string("val");
}

/// Verifies HashMap can be created inside a function.
#[test]
fn test_hashmap_created_in_function() {
    ShapeTest::new(
        r#"
        fn make_map() {
            HashMap().set("x", 42)
        }
        make_map().get("x")
    "#,
    )
    .expect_number(42.0);
}

/// Verifies building a HashMap with 20 entries via chaining.
#[test]
fn test_hashmap_build_20_entries() {
    ShapeTest::new(
        r#"
        HashMap()
            .set("k01", 1).set("k02", 2).set("k03", 3).set("k04", 4).set("k05", 5)
            .set("k06", 6).set("k07", 7).set("k08", 8).set("k09", 9).set("k10", 10)
            .set("k11", 11).set("k12", 12).set("k13", 13).set("k14", 14).set("k15", 15)
            .set("k16", 16).set("k17", 17).set("k18", 18).set("k19", 19).set("k20", 20)
            .len()
    "#,
    )
    .expect_number(20.0);
}

/// Verifies building a HashMap in a loop with 50 entries.
#[test]
fn test_hashmap_build_loop() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap()
        let mut i = 0
        while i < 50 {
            m = m.set(i, i * i)
            i = i + 1
        }
        m.len()
    }"#,
    )
    .expect_number(50.0);
}

/// Verifies querying individual entries from a loop-built HashMap.
#[test]
fn test_hashmap_loop_build_and_query() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap()
        let mut i = 0
        while i < 10 {
            m = m.set(i, i * 2)
            i = i + 1
        }
        print(m.get(0))
        print(m.get(5))
        print(m.get(9))
    }"#,
    )
    .expect_run_ok()
    .expect_output("0\n10\n18");
}
