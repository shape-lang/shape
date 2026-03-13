//! Stress tests for HashMap get/set/delete/has operations, len/length/isEmpty,
//! integer keys, boolean keys, immutability, nested hashmaps, and edge cases.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// set / get
// =========================================================================

/// Verifies set and get with a string value.
#[test]
fn test_hashmap_set_get_string_value() {
    ShapeTest::new(r#"HashMap().set("name", "Alice").get("name")"#).expect_string("Alice");
}

/// Verifies set and get with an integer value.
#[test]
fn test_hashmap_set_get_integer_value() {
    ShapeTest::new(r#"HashMap().set("age", 30).get("age")"#).expect_number(30.0);
}

/// Verifies set and get with a float value.
#[test]
fn test_hashmap_set_get_float_value() {
    ShapeTest::new(r#"HashMap().set("pi", 3.14).get("pi")"#).expect_number(3.14);
}

/// Verifies set and get with a boolean true value.
#[test]
fn test_hashmap_set_get_bool_value() {
    ShapeTest::new(r#"HashMap().set("flag", true).get("flag")"#).expect_bool(true);
}

/// Verifies set and get with a boolean false value.
#[test]
fn test_hashmap_set_get_false_value() {
    ShapeTest::new(r#"HashMap().set("flag", false).get("flag")"#).expect_bool(false);
}

/// Verifies set and get with a None value.
#[test]
fn test_hashmap_set_get_none_value() {
    ShapeTest::new(r#"HashMap().set("x", None).get("x")"#).expect_none();
}

/// Verifies overwriting an existing key updates the value.
#[test]
fn test_hashmap_overwrite_existing_key() {
    ShapeTest::new(r#"HashMap().set("k", 1).set("k", 99).get("k")"#).expect_number(99.0);
}

/// Verifies overwriting preserves the length (no duplicates).
#[test]
fn test_hashmap_overwrite_preserves_len() {
    ShapeTest::new(r#"HashMap().set("k", 1).set("k", 99).len()"#).expect_number(1.0);
}

/// Verifies multiple overwrites of same key.
#[test]
fn test_hashmap_overwrite_multiple_times() {
    ShapeTest::new(
        r#"
        HashMap().set("x", 1).set("x", 2).set("x", 3).set("x", 4).get("x")
    "#,
    )
    .expect_number(4.0);
}

/// Verifies get of missing key returns None.
#[test]
fn test_hashmap_get_missing_key_returns_none() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1)
        m.get("missing")
    }"#,
    )
    .expect_none();
}

/// Verifies get from empty HashMap returns None.
#[test]
fn test_hashmap_get_from_empty() {
    ShapeTest::new(r#"HashMap().get("x")"#).expect_none();
}

/// Verifies set and get with an array value.
#[test]
fn test_hashmap_set_array_value() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("nums", [1, 2, 3])
        m.get("nums").length
    }"#,
    )
    .expect_number(3.0);
}

/// Verifies nested HashMap as value.
#[test]
fn test_hashmap_set_nested_hashmap_value() {
    ShapeTest::new(
        r#"{
        let inner = HashMap().set("nested", true)
        let outer = HashMap().set("child", inner)
        outer.get("child").get("nested")
    }"#,
    )
    .expect_bool(true);
}

/// Verifies HashMap with many entries.
#[test]
fn test_hashmap_many_entries() {
    ShapeTest::new(
        r#"
        HashMap()
            .set("a", 1).set("b", 2).set("c", 3).set("d", 4).set("e", 5)
            .set("f", 6).set("g", 7).set("h", 8).set("i", 9).set("j", 10)
            .len()
    "#,
    )
    .expect_number(10.0);
}

/// Verifies empty string key works.
#[test]
fn test_hashmap_empty_string_key() {
    ShapeTest::new(r#"HashMap().set("", "empty_key").get("")"#).expect_string("empty_key");
}

/// Verifies space in key works.
#[test]
fn test_hashmap_space_in_key() {
    ShapeTest::new(r#"HashMap().set("hello world", 42).get("hello world")"#).expect_number(42.0);
}

// =========================================================================
// has
// =========================================================================

/// Verifies has returns true for existing key.
#[test]
fn test_hashmap_has_existing_key() {
    ShapeTest::new(r#"HashMap().set("a", 1).has("a")"#).expect_bool(true);
}

/// Verifies has returns false for missing key.
#[test]
fn test_hashmap_has_missing_key() {
    ShapeTest::new(r#"HashMap().set("a", 1).has("b")"#).expect_bool(false);
}

/// Verifies has on empty HashMap.
#[test]
fn test_hashmap_has_on_empty() {
    ShapeTest::new(r#"HashMap().has("anything")"#).expect_bool(false);
}

/// Verifies has reflects set is immutable (original unchanged).
#[test]
fn test_hashmap_has_after_set() {
    ShapeTest::new(
        r#"{
        let m = HashMap()
        let m2 = m.set("x", 1)
        print(m.has("x"))
        print(m2.has("x"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("false\ntrue");
}

/// Verifies has returns true even when value is None.
#[test]
fn test_hashmap_has_with_none_value() {
    ShapeTest::new(r#"HashMap().set("x", None).has("x")"#).expect_bool(true);
}

/// Verifies has with integer key.
#[test]
fn test_hashmap_has_integer_key() {
    ShapeTest::new(r#"HashMap().set(42, "answer").has(42)"#).expect_bool(true);
}

/// Verifies has with missing integer key.
#[test]
fn test_hashmap_has_integer_key_missing() {
    ShapeTest::new(r#"HashMap().set(42, "answer").has(99)"#).expect_bool(false);
}

// =========================================================================
// delete
// =========================================================================

/// Verifies delete removes an entry.
#[test]
fn test_hashmap_delete_existing_key() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).delete("a").len()"#).expect_number(1.0);
}

/// Verifies delete of missing key is a no-op.
#[test]
fn test_hashmap_delete_missing_key() {
    ShapeTest::new(r#"HashMap().set("a", 1).delete("z").len()"#).expect_number(1.0);
}

/// Verifies delete then has returns false.
#[test]
fn test_hashmap_delete_then_has() {
    ShapeTest::new(r#"HashMap().set("a", 1).delete("a").has("a")"#).expect_bool(false);
}

/// Verifies delete then get returns None.
#[test]
fn test_hashmap_delete_then_get() {
    ShapeTest::new(r#"HashMap().set("a", 1).delete("a").get("a")"#).expect_none();
}

/// Verifies delete preserves other keys.
#[test]
fn test_hashmap_delete_preserves_other_keys() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3).delete("b")
        print(m.get("a"))
        print(m.has("b"))
        print(m.get("c"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\nfalse\n3");
}

/// Verifies delete from empty HashMap.
#[test]
fn test_hashmap_delete_from_empty() {
    ShapeTest::new(r#"HashMap().delete("x").len()"#).expect_number(0.0);
}

/// Verifies delete all keys results in empty HashMap.
#[test]
fn test_hashmap_delete_all_keys() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).delete("a").delete("b").isEmpty()
    "#,
    )
    .expect_bool(true);
}

/// Verifies delete is immutable (original unchanged).
#[test]
fn test_hashmap_delete_immutability() {
    ShapeTest::new(
        r#"{
        let original = HashMap().set("a", 1).set("b", 2)
        let deleted = original.delete("a")
        print(original.len())
        print(deleted.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("2\n1");
}

/// Verifies delete and re-add.
#[test]
fn test_hashmap_delete_and_re_add() {
    ShapeTest::new(
        r#"
        HashMap().set("x", 1).delete("x").set("x", 99).get("x")
    "#,
    )
    .expect_number(99.0);
}

// =========================================================================
// len / length / isEmpty
// =========================================================================

/// Verifies len on empty HashMap.
#[test]
fn test_hashmap_len_empty() {
    ShapeTest::new("HashMap().len()").expect_number(0.0);
}

/// Verifies len with entries.
#[test]
fn test_hashmap_len_with_entries() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).len()"#).expect_number(2.0);
}

/// Verifies length property.
#[test]
fn test_hashmap_length_property() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).length"#).expect_number(2.0);
}

/// Verifies len equals length.
#[test]
fn test_hashmap_len_equals_length() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", 1).set("y", 2).set("z", 3)
        m.len() == m.length
    }"#,
    )
    .expect_bool(true);
}

/// Verifies len after overwrite stays the same.
#[test]
fn test_hashmap_len_after_overwrite() {
    ShapeTest::new(r#"HashMap().set("k", 1).set("k", 2).len()"#).expect_number(1.0);
}

/// Verifies len after delete.
#[test]
fn test_hashmap_len_after_delete() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).delete("a").len()"#).expect_number(1.0);
}

/// Verifies isEmpty on empty.
#[test]
fn test_hashmap_is_empty_on_empty() {
    ShapeTest::new("HashMap().isEmpty()").expect_bool(true);
}

/// Verifies isEmpty on non-empty.
#[test]
fn test_hashmap_is_empty_on_nonempty() {
    ShapeTest::new(r#"HashMap().set("x", 1).isEmpty()"#).expect_bool(false);
}

/// Verifies isEmpty after deleting all entries.
#[test]
fn test_hashmap_is_empty_after_delete_all() {
    ShapeTest::new(r#"HashMap().set("a", 1).delete("a").isEmpty()"#).expect_bool(true);
}

/// Verifies length on empty.
#[test]
fn test_hashmap_length_empty() {
    ShapeTest::new("HashMap().length").expect_number(0.0);
}

// =========================================================================
// Integer keys
// =========================================================================

/// Verifies integer key set and get.
#[test]
fn test_hashmap_integer_key_set_get() {
    ShapeTest::new(r#"HashMap().set(1, "one").get(1)"#).expect_string("one");
}

/// Verifies multiple integer keys.
#[test]
fn test_hashmap_integer_key_multiple() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set(1, "one").set(2, "two").set(3, "three")
        print(m.get(1))
        print(m.get(2))
        print(m.get(3))
    }"#,
    )
    .expect_run_ok()
    .expect_output("one\ntwo\nthree");
}

/// Verifies has with integer key.
#[test]
fn test_hashmap_integer_key_has() {
    ShapeTest::new(r#"HashMap().set(42, "answer").has(42)"#).expect_bool(true);
}

/// Verifies delete with integer key.
#[test]
fn test_hashmap_integer_key_delete() {
    ShapeTest::new(r#"HashMap().set(1, "a").set(2, "b").delete(1).len()"#).expect_number(1.0);
}

/// Verifies get of missing integer key returns None.
#[test]
fn test_hashmap_integer_key_missing() {
    ShapeTest::new(r#"HashMap().set(1, "a").get(999)"#).expect_none();
}

/// Verifies mixed string and integer keys.
#[test]
fn test_hashmap_mixed_key_types() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("str", 1).set(42, 2)
        print(m.get("str"))
        print(m.get(42))
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\n2");
}

// =========================================================================
// Boolean keys
// =========================================================================

/// Verifies boolean true as key.
#[test]
fn test_hashmap_bool_key_true() {
    ShapeTest::new(r#"HashMap().set(true, "yes").get(true)"#).expect_string("yes");
}

/// Verifies boolean false as key.
#[test]
fn test_hashmap_bool_key_false() {
    ShapeTest::new(r#"HashMap().set(false, "no").get(false)"#).expect_string("no");
}

/// Verifies both boolean keys coexist.
#[test]
fn test_hashmap_bool_key_both() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set(true, "yes").set(false, "no")
        print(m.get(true))
        print(m.get(false))
        print(m.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("yes\nno\n2");
}

// =========================================================================
// Immutability
// =========================================================================

/// Verifies set is immutable (original unchanged).
#[test]
fn test_hashmap_set_immutability() {
    ShapeTest::new(
        r#"{
        let m = HashMap()
        let m2 = m.set("a", 1)
        print(m.len())
        print(m2.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("0\n1");
}

/// Verifies delete immutability.
#[test]
fn test_hashmap_delete_immutability_values() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2)
        let m2 = m.delete("a")
        print(m.has("a"))
        print(m2.has("a"))
        print(m.has("b"))
        print(m2.has("b"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse\ntrue\ntrue");
}

/// Verifies chain immutability.
#[test]
fn test_hashmap_chain_immutability() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("a", 1)
        let m2 = m1.set("b", 2)
        let m3 = m2.set("c", 3)
        print(m1.len())
        print(m2.len())
        print(m3.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\n2\n3");
}

/// Verifies filter immutability.
#[test]
fn test_hashmap_filter_immutability() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        let filtered = m.filter(|k, v| v > 1)
        print(m.len())
        print(filtered.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("3\n2");
}

// =========================================================================
// Nested HashMaps
// =========================================================================

/// Verifies nested get.
#[test]
fn test_hashmap_nested_get() {
    ShapeTest::new(
        r#"{
        let inner = HashMap().set("deep", 42)
        let outer = HashMap().set("inner", inner)
        outer.get("inner").get("deep")
    }"#,
    )
    .expect_number(42.0);
}

/// Verifies double nested get.
#[test]
fn test_hashmap_double_nested() {
    ShapeTest::new(
        r#"{
        let level2 = HashMap().set("val", 99)
        let level1 = HashMap().set("child", level2)
        let root = HashMap().set("child", level1)
        root.get("child").get("child").get("val")
    }"#,
    )
    .expect_number(99.0);
}

/// Verifies nested has.
#[test]
fn test_hashmap_nested_has() {
    ShapeTest::new(
        r#"{
        let inner = HashMap().set("key", 1)
        let outer = HashMap().set("inner", inner)
        outer.get("inner").has("key")
    }"#,
    )
    .expect_bool(true);
}

/// Verifies nested len.
#[test]
fn test_hashmap_nested_len() {
    ShapeTest::new(
        r#"{
        let inner = HashMap().set("a", 1).set("b", 2)
        let outer = HashMap().set("map", inner)
        outer.get("map").len()
    }"#,
    )
    .expect_number(2.0);
}

// =========================================================================
// Edge cases
// =========================================================================

/// Verifies empty string key has.
#[test]
fn test_hashmap_empty_string_key_has() {
    ShapeTest::new(r#"HashMap().set("", 1).has("")"#).expect_bool(true);
}

/// Verifies empty string key delete.
#[test]
fn test_hashmap_empty_string_key_delete() {
    ShapeTest::new(r#"HashMap().set("", 1).delete("").len()"#).expect_number(0.0);
}

/// Verifies zero value.
#[test]
fn test_hashmap_zero_value() {
    ShapeTest::new(r#"HashMap().set("zero", 0).get("zero")"#).expect_number(0.0);
}

/// Verifies negative value.
#[test]
fn test_hashmap_negative_value() {
    ShapeTest::new(r#"HashMap().set("neg", -42).get("neg")"#).expect_number(-42.0);
}

/// Verifies large number value.
#[test]
fn test_hashmap_large_number_value() {
    ShapeTest::new(r#"HashMap().set("big", 1000000).get("big")"#).expect_number(1000000.0);
}

/// Verifies array value access through HashMap.
#[test]
fn test_hashmap_array_value_access() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("arr", [10, 20, 30])
        let arr = m.get("arr")
        arr[1]
    }"#,
    )
    .expect_number(20.0);
}

/// Verifies HashMap returned from function.
#[test]
fn test_hashmap_returned_from_function() {
    ShapeTest::new(
        r#"
        fn build() {
            HashMap().set("result", 42)
        }
        fn query(m) {
            m.get("result")
        }
        query(build())
    "#,
    )
    .expect_number(42.0);
}

/// Verifies HashMap passed to function.
#[test]
fn test_hashmap_passed_to_function() {
    ShapeTest::new(
        r#"
        fn get_value(m, key) {
            m.get(key)
        }
        let m = HashMap().set("x", 77)
        get_value(m, "x")
    "#,
    )
    .expect_number(77.0);
}

/// Verifies HashMap stored in array.
#[test]
fn test_hashmap_in_array() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("id", 1)
        let m2 = HashMap().set("id", 2)
        let arr = [m1, m2]
        arr[0].get("id")
    }"#,
    )
    .expect_number(1.0);
}

// =========================================================================
// getOrDefault
// =========================================================================

/// Verifies getOrDefault when key exists.
#[test]
fn test_hashmap_get_or_default_key_exists() {
    ShapeTest::new(r#"HashMap().set("a", 42).getOrDefault("a", 0)"#).expect_number(42.0);
}

/// Verifies getOrDefault when key is missing.
#[test]
fn test_hashmap_get_or_default_key_missing() {
    ShapeTest::new(r#"HashMap().set("a", 42).getOrDefault("b", 0)"#).expect_number(0.0);
}

/// Verifies getOrDefault with string default.
#[test]
fn test_hashmap_get_or_default_string_value() {
    ShapeTest::new(r#"HashMap().getOrDefault("missing", "default")"#).expect_string("default");
}

/// Verifies getOrDefault on empty HashMap.
#[test]
fn test_hashmap_get_or_default_on_empty() {
    ShapeTest::new(r#"HashMap().getOrDefault("x", 99)"#).expect_number(99.0);
}

/// Verifies getOrDefault with stored None value returns None.
#[test]
fn test_hashmap_get_or_default_with_none_value() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", None)
        m.getOrDefault("x", "fallback")
    }"#,
    )
    .expect_none();
}

/// Verifies getOrDefault with bool default.
#[test]
fn test_hashmap_get_or_default_bool_default() {
    ShapeTest::new(r#"HashMap().getOrDefault("flag", false)"#).expect_bool(false);
}

// =========================================================================
// Var reassignment
// =========================================================================

/// Verifies var reassignment with set.
#[test]
fn test_hashmap_var_reassignment() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap()
        m = m.set("a", 1)
        m = m.set("b", 2)
        m = m.set("c", 3)
        m.len()
    }"#,
    )
    .expect_number(3.0);
}

/// Verifies var reassignment with overwrite.
#[test]
fn test_hashmap_var_reassignment_overwrite() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap().set("x", 1)
        m = m.set("x", 2)
        m = m.set("x", 3)
        m.get("x")
    }"#,
    )
    .expect_number(3.0);
}

/// Verifies var reassignment with delete.
#[test]
fn test_hashmap_var_reassignment_delete() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap().set("a", 1).set("b", 2)
        m = m.delete("a")
        print(m.len())
        print(m.has("a"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\nfalse");
}

/// Verifies var reassignment with merge.
#[test]
fn test_hashmap_var_merge_reassignment() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap().set("a", 1)
        let extra = HashMap().set("b", 2)
        m = m.merge(extra)
        m.len()
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies var reassignment with filter.
#[test]
fn test_hashmap_var_filter_reassignment() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap().set("a", 1).set("b", 10).set("c", 100)
        m = m.filter(|k, v| v >= 10)
        m.len()
    }"#,
    )
    .expect_number(2.0);
}
