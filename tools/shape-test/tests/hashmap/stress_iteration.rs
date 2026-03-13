//! Stress tests for HashMap iteration, keys/values/entries, map, filter,
//! forEach, merge, reduce, toArray, groupBy, method chaining, and complex scenarios.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// keys / values / entries
// =========================================================================

/// Verifies keys count.
#[test]
fn test_hashmap_keys_count() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).keys().length"#).expect_number(2.0);
}

/// Verifies keys on empty HashMap.
#[test]
fn test_hashmap_keys_empty() {
    ShapeTest::new(r#"HashMap().keys().length"#).expect_number(0.0);
}

/// Verifies values count.
#[test]
fn test_hashmap_values_count() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).values().length"#).expect_number(2.0);
}

/// Verifies values on empty HashMap.
#[test]
fn test_hashmap_values_empty() {
    ShapeTest::new(r#"HashMap().values().length"#).expect_number(0.0);
}

/// Verifies entries count.
#[test]
fn test_hashmap_entries_count() {
    ShapeTest::new(r#"HashMap().set("a", 1).set("b", 2).entries().length"#).expect_number(2.0);
}

/// Verifies entries on empty HashMap.
#[test]
fn test_hashmap_entries_empty() {
    ShapeTest::new(r#"HashMap().entries().length"#).expect_number(0.0);
}

/// Verifies keys/values/entries have consistent lengths.
#[test]
fn test_hashmap_keys_values_entries_consistent() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        print(m.keys().length)
        print(m.values().length)
        print(m.entries().length)
    }"#,
    )
    .expect_run_ok()
    .expect_output("3\n3\n3");
}

/// Verifies keys returns an array.
#[test]
fn test_hashmap_keys_returns_array() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2)
        let k = m.keys()
        k.length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies values returns an array.
#[test]
fn test_hashmap_values_returns_array() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 10).set("b", 20)
        let v = m.values()
        v.length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies entries are pairs (length 2).
#[test]
fn test_hashmap_entries_are_pairs() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("only", 42)
        let e = m.entries()
        let pair = e[0]
        pair.length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies entry pair contents.
#[test]
fn test_hashmap_entries_pair_values() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("only", 42)
        let e = m.entries()
        let pair = e[0]
        print(pair[0])
        print(pair[1])
    }"#,
    )
    .expect_run_ok()
    .expect_output("only\n42");
}

/// Verifies single entry keys count.
#[test]
fn test_hashmap_single_entry_keys() {
    ShapeTest::new(r#"HashMap().set("solo", 1).keys().length"#).expect_number(1.0);
}

/// Verifies single entry values count.
#[test]
fn test_hashmap_single_entry_values() {
    ShapeTest::new(r#"HashMap().set("solo", 99).values().length"#).expect_number(1.0);
}

// =========================================================================
// map
// =========================================================================

/// Verifies map doubles values.
#[test]
fn test_hashmap_map_double_values() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 10).set("b", 20)
        let doubled = m.map(|k, v| v * 2)
        print(doubled.get("a"))
        print(doubled.get("b"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("20\n40");
}

/// Verifies map preserves keys.
#[test]
fn test_hashmap_map_preserves_keys() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", 1).set("y", 2)
        let mapped = m.map(|k, v| v + 100)
        print(mapped.has("x"))
        print(mapped.has("y"))
        print(mapped.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("true\ntrue\n2");
}

/// Verifies map preserves length.
#[test]
fn test_hashmap_map_preserves_len() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).set("c", 3).map(|k, v| v * v).len()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies map with squaring.
#[test]
fn test_hashmap_map_squared() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 2).set("b", 3)
        let sq = m.map(|k, v| v * v)
        print(sq.get("a"))
        print(sq.get("b"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("4\n9");
}

/// Verifies map to string values.
#[test]
fn test_hashmap_map_to_string() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("greeting", "hello")
        let mapped = m.map(|k, v| v + " world")
        mapped.get("greeting")
    }"#,
    )
    .expect_string("hello world");
}

/// Verifies map on empty HashMap.
#[test]
fn test_hashmap_map_on_empty() {
    ShapeTest::new(r#"HashMap().map(|k, v| v * 2).len()"#).expect_number(0.0);
}

/// Verifies map does not mutate original.
#[test]
fn test_hashmap_map_does_not_mutate_original() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 5)
        let mapped = m.map(|k, v| v * 10)
        print(m.get("a"))
        print(mapped.get("a"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("5\n50");
}

/// Verifies map to boolean values.
#[test]
fn test_hashmap_map_to_boolean() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("pos", 5).set("neg", -3)
        let mapped = m.map(|k, v| v > 0)
        print(mapped.get("pos"))
        print(mapped.get("neg"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

// =========================================================================
// filter
// =========================================================================

/// Verifies filter by value.
#[test]
fn test_hashmap_filter_by_value() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 20).set("c", 3)
        let big = m.filter(|k, v| v > 10)
        print(big.len())
        print(big.has("b"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\ntrue");
}

/// Verifies filter removes non-matching entries.
#[test]
fn test_hashmap_filter_removes_non_matching() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 20).set("c", 3)
        let big = m.filter(|k, v| v > 10)
        print(big.has("a"))
        print(big.has("c"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("false\nfalse");
}

/// Verifies filter with empty result.
#[test]
fn test_hashmap_filter_empty_result() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).filter(|k, v| v > 100).len()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies filter where all entries pass.
#[test]
fn test_hashmap_filter_all_pass() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).filter(|k, v| v > 0).len()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies filter on empty HashMap.
#[test]
fn test_hashmap_filter_on_empty() {
    ShapeTest::new(r#"HashMap().filter(|k, v| v > 0).len()"#).expect_number(0.0);
}

/// Verifies filter does not mutate original.
#[test]
fn test_hashmap_filter_does_not_mutate_original() {
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

/// Verifies filter with single entry passing.
#[test]
fn test_hashmap_filter_single_entry() {
    ShapeTest::new(
        r#"
        HashMap().set("only", 5).filter(|k, v| v == 5).len()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies filter with single entry removed.
#[test]
fn test_hashmap_filter_single_entry_removed() {
    ShapeTest::new(
        r#"
        HashMap().set("only", 5).filter(|k, v| v != 5).len()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies filter chained with len.
#[test]
fn test_hashmap_filter_chained_with_len() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 5).set("c", 10)
            .filter(|k, v| v > 3).len()
    "#,
    )
    .expect_number(2.0);
}

// =========================================================================
// forEach
// =========================================================================

/// Verifies forEach executes without error.
#[test]
fn test_hashmap_foreach_executes() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2)
        m.forEach(|k, v| v + 1)
        m.len()
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies forEach on empty HashMap.
#[test]
fn test_hashmap_foreach_on_empty() {
    ShapeTest::new(
        r#"{
        HashMap().forEach(|k, v| v)
        true
    }"#,
    )
    .expect_bool(true);
}

/// Verifies forEach with accumulator side effect.
#[test]
fn test_hashmap_foreach_with_accumulator() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 10).set("b", 20).set("c", 30)
        let mut sum = 0
        m.forEach(|k, v| { sum = sum + v })
        sum
    }"#,
    )
    .expect_number(60.0);
}

/// Verifies forEach with single entry.
#[test]
fn test_hashmap_foreach_single_entry() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", 42)
        let mut count = 0
        m.forEach(|k, v| { count = count + 1 })
        count
    }"#,
    )
    .expect_number(1.0);
}

// =========================================================================
// merge
// =========================================================================

/// Verifies merging two maps.
#[test]
fn test_hashmap_merge_two_maps() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("a", 1)
        let m2 = HashMap().set("b", 2)
        let merged = m1.merge(m2)
        print(merged.len())
        print(merged.get("a"))
        print(merged.get("b"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("2\n1\n2");
}

/// Verifies merge with overlapping keys (other wins).
#[test]
fn test_hashmap_merge_overlapping_keys() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("x", 1)
        let m2 = HashMap().set("x", 99)
        m1.merge(m2).get("x")
    }"#,
    )
    .expect_number(99.0);
}

/// Verifies merge overlapping preserves correct length.
#[test]
fn test_hashmap_merge_overlapping_preserves_len() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("a", 1).set("b", 2)
        let m2 = HashMap().set("b", 99).set("c", 3)
        m1.merge(m2).len()
    }"#,
    )
    .expect_number(3.0);
}

/// Verifies merge with empty HashMap.
#[test]
fn test_hashmap_merge_with_empty() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1)
        m.merge(HashMap()).len()
    }"#,
    )
    .expect_number(1.0);
}

/// Verifies merge empty with non-empty.
#[test]
fn test_hashmap_merge_empty_with_nonempty() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1)
        HashMap().merge(m).len()
    }"#,
    )
    .expect_number(1.0);
}

/// Verifies merge both empty.
#[test]
fn test_hashmap_merge_both_empty() {
    ShapeTest::new(r#"HashMap().merge(HashMap()).len()"#).expect_number(0.0);
}

/// Verifies merge immutability.
#[test]
fn test_hashmap_merge_immutability() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("a", 1)
        let m2 = HashMap().set("b", 2)
        let merged = m1.merge(m2)
        print(m1.len())
        print(m2.len())
        print(merged.len())
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\n1\n2");
}

// =========================================================================
// reduce
// =========================================================================

/// Verifies reduce sum values.
#[test]
fn test_hashmap_reduce_sum_values() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 10).set("b", 20).set("c", 30)
        m.reduce(|acc, k, v| acc + v, 0)
    }"#,
    )
    .expect_number(60.0);
}

/// Verifies reduce count entries.
#[test]
fn test_hashmap_reduce_count_entries() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", 1).set("y", 2).set("z", 3)
        m.reduce(|acc, k, v| acc + 1, 0)
    }"#,
    )
    .expect_number(3.0);
}

/// Verifies reduce on empty HashMap.
#[test]
fn test_hashmap_reduce_empty() {
    ShapeTest::new(r#"HashMap().reduce(|acc, k, v| acc + v, 0)"#).expect_number(0.0);
}

/// Verifies reduce product.
#[test]
fn test_hashmap_reduce_product() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 2).set("b", 3).set("c", 4)
        m.reduce(|acc, k, v| acc * v, 1)
    }"#,
    )
    .expect_number(24.0);
}

/// Verifies reduce with string initial value.
#[test]
fn test_hashmap_reduce_string_initial() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1)
        m.reduce(|acc, k, v| acc + k, "keys:")
    }"#,
    )
    .expect_string("keys:a");
}

// =========================================================================
// toArray
// =========================================================================

/// Verifies toArray length.
#[test]
fn test_hashmap_to_array_length() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).toArray().length
    "#,
    )
    .expect_number(2.0);
}

/// Verifies toArray on empty.
#[test]
fn test_hashmap_to_array_empty() {
    ShapeTest::new(r#"HashMap().toArray().length"#).expect_number(0.0);
}

/// Verifies toArray produces pairs.
#[test]
fn test_hashmap_to_array_produces_pairs() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("only", 42)
        let arr = m.toArray()
        let pair = arr[0]
        pair.length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies toArray pair content.
#[test]
fn test_hashmap_to_array_pair_content() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("key", 99)
        let arr = m.toArray()
        let pair = arr[0]
        print(pair[0])
        print(pair[1])
    }"#,
    )
    .expect_run_ok()
    .expect_output("key\n99");
}

// =========================================================================
// groupBy
// =========================================================================

/// Verifies groupBy basic.
#[test]
fn test_hashmap_group_by_basic() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 1)
        let grouped = m.groupBy(|k, v| v)
        grouped.len()
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies groupBy single group.
#[test]
fn test_hashmap_group_by_single_group() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 1).set("c", 1)
        m.groupBy(|k, v| v).len()
    }"#,
    )
    .expect_number(1.0);
}

/// Verifies groupBy on empty.
#[test]
fn test_hashmap_group_by_empty() {
    ShapeTest::new(r#"HashMap().groupBy(|k, v| v).len()"#).expect_number(0.0);
}

/// Verifies groupBy all different.
#[test]
fn test_hashmap_group_by_all_different() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        m.groupBy(|k, v| v).len()
    }"#,
    )
    .expect_number(3.0);
}

// =========================================================================
// Method chaining
// =========================================================================

/// Verifies set chain then get.
#[test]
fn test_hashmap_set_chain_then_get() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).set("c", 3).get("b")
    "#,
    )
    .expect_number(2.0);
}

/// Verifies filter then map.
#[test]
fn test_hashmap_filter_then_map() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 5).set("c", 10)
        let result = m.filter(|k, v| v > 3).map(|k, v| v * 2)
        result.len()
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies set, delete, set chain.
#[test]
fn test_hashmap_set_delete_set() {
    ShapeTest::new(
        r#"
        HashMap().set("x", 1).delete("x").set("x", 42).get("x")
    "#,
    )
    .expect_number(42.0);
}

/// Verifies delete chain.
#[test]
fn test_hashmap_delete_chain() {
    ShapeTest::new(
        r#"
        HashMap().set("a", 1).set("b", 2).set("c", 3)
            .delete("a").delete("c").len()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies merge then filter.
#[test]
fn test_hashmap_merge_then_filter() {
    ShapeTest::new(
        r#"{
        let m1 = HashMap().set("a", 1).set("b", 100)
        let m2 = HashMap().set("c", 2).set("d", 200)
        m1.merge(m2).filter(|k, v| v > 50).len()
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies map then reduce.
#[test]
fn test_hashmap_map_then_reduce() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        m.map(|k, v| v * 10).reduce(|acc, k, v| acc + v, 0)
    }"#,
    )
    .expect_number(60.0);
}

// =========================================================================
// Complex scenarios
// =========================================================================

/// Verifies config pattern with merge.
#[test]
fn test_hashmap_config_pattern() {
    ShapeTest::new(
        r#"{
        let defaults = HashMap()
            .set("host", "localhost")
            .set("port", 8080)
            .set("debug", false)
        let overrides = HashMap()
            .set("port", 3000)
            .set("debug", true)
        let config = defaults.merge(overrides)
        print(config.get("host"))
        print(config.get("port"))
        print(config.get("debug"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("localhost\n3000\ntrue");
}

/// Verifies counter pattern using getOrDefault.
#[test]
fn test_hashmap_counter_pattern() {
    ShapeTest::new(
        r#"{
        let mut counts = HashMap()
        let items = ["a", "b", "a", "c", "b", "a"]
        for item in items {
            let current = counts.getOrDefault(item, 0)
            counts = counts.set(item, current + 1)
        }
        print(counts.get("a"))
        print(counts.get("b"))
        print(counts.get("c"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("3\n2\n1");
}

/// Verifies filter then keys.
#[test]
fn test_hashmap_filter_then_keys() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 1).set("b", 50).set("c", 100)
        let big = m.filter(|k, v| v >= 50)
        big.keys().length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies map then values.
#[test]
fn test_hashmap_map_then_values() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("x", 2).set("y", 3)
        let squared = m.map(|k, v| v * v)
        squared.values().length
    }"#,
    )
    .expect_number(2.0);
}

/// Verifies set in loop with string keys.
#[test]
fn test_hashmap_set_in_loop_with_string_keys() {
    ShapeTest::new(
        r#"{
        let keys = ["alpha", "beta", "gamma", "delta"]
        let mut m = HashMap()
        let mut i = 0
        for key in keys {
            m = m.set(key, i)
            i = i + 1
        }
        print(m.len())
        print(m.get("alpha"))
        print(m.get("delta"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("4\n0\n3");
}

/// Verifies HashMap set and get works with number values.
#[test]
fn test_hashmap_function_as_value() {
    ShapeTest::new(
        r#"{
        let m = HashMap()
            .set("a", 10)
            .set("b", 20)
        let val = m.get("a")
        val
    }"#,
    )
    .expect_number(10.0);
}

/// Verifies conditional set in loop.
#[test]
fn test_hashmap_conditional_set() {
    ShapeTest::new(
        r#"{
        let mut m = HashMap()
        let values = [1, 2, 3, 4, 5]
        for v in values {
            if v > 3 {
                m = m.set(v, "big")
            } else {
                m = m.set(v, "small")
            }
        }
        print(m.get(2))
        print(m.get(4))
    }"#,
    )
    .expect_run_ok()
    .expect_output("small\nbig");
}

/// Verifies reduce max value.
#[test]
fn test_hashmap_reduce_max_value() {
    ShapeTest::new(
        r#"{
        let m = HashMap().set("a", 10).set("b", 50).set("c", 30)
        m.reduce(|acc, k, v| if v > acc { v } else { acc }, 0)
    }"#,
    )
    .expect_number(50.0);
}

/// Verifies get each of many entries.
#[test]
fn test_hashmap_get_each_of_many_entries() {
    ShapeTest::new(
        r#"{
        let m = HashMap()
            .set("a", 1).set("b", 2).set("c", 3).set("d", 4).set("e", 5)
        print(m.get("a"))
        print(m.get("b"))
        print(m.get("c"))
        print(m.get("d"))
        print(m.get("e"))
    }"#,
    )
    .expect_run_ok()
    .expect_output("1\n2\n3\n4\n5");
}
