//! Array operations and HashMap tests.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 2. Array Operations (25 tests)
// =========================================================================

#[test]
fn test_array_literal_basic() {
    ShapeTest::new(
        r#"
        let a = [1, 2, 3]
        a.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_array_index_access() {
    ShapeTest::new(
        r#"
        [10, 20, 30][1]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_array_index_first_element() {
    ShapeTest::new(
        r#"
        let arr = [100, 200, 300]
        arr[0]
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_array_index_last_element() {
    ShapeTest::new(
        r#"
        let arr = [100, 200, 300]
        arr[2]
    "#,
    )
    .expect_number(300.0);
}

#[test]
fn test_array_length_property() {
    ShapeTest::new(
        r#"
        [10, 20, 30, 40, 50].length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_array_push_immutable() {
    // .push returns a new array with the element appended
    ShapeTest::new(
        r#"
        var arr = [1, 2]
        arr = arr.push(3)
        arr.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_array_map_double() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3].map(|x| x * 2)
        result[0] + result[1] + result[2]
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_array_map_to_bool() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3].map(|x| x > 1)
        result[0]
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_array_filter_greater_than() {
    ShapeTest::new(
        r#"
        let result = [1, 2, 3, 4, 5].filter(|x| x > 3)
        result.length
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_array_filter_even() {
    ShapeTest::new(
        r#"
        let evens = [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
        evens[0] + evens[1] + evens[2]
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_array_reduce_sum() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4, 5].reduce(|acc, x| acc + x, 0)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_array_reduce_product() {
    ShapeTest::new(
        r#"
        [1, 2, 3, 4].reduce(|acc, x| acc * x, 1)
    "#,
    )
    .expect_number(24.0);
}

#[test]
fn test_array_includes_true() {
    ShapeTest::new(
        r#"
        [10, 20, 30].includes(20)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_array_includes_false() {
    ShapeTest::new(
        r#"
        [10, 20, 30].includes(99)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_array_slice_basic() {
    ShapeTest::new(
        r#"
        let result = [10, 20, 30, 40, 50].slice(1, 4)
        result.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_array_slice_values() {
    ShapeTest::new(
        r#"
        let result = [10, 20, 30, 40, 50].slice(1, 3)
        result[0]
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_array_sort_ascending() {
    ShapeTest::new(
        r#"
        let sorted = [3, 1, 4, 1, 5].sort(|a, b| a - b)
        sorted[0]
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_array_sort_descending() {
    ShapeTest::new(
        r#"
        let sorted = [3, 1, 4, 1, 5].sort(|a, b| b - a)
        sorted[0]
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_array_reverse() {
    ShapeTest::new(
        r#"
        let rev = [1, 2, 3].reverse()
        rev[0]
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_array_reverse_length_preserved() {
    ShapeTest::new(
        r#"
        let rev = [1, 2, 3, 4, 5].reverse()
        rev.length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_array_empty_length() {
    ShapeTest::new(
        r#"
        let a = []
        a.length
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn test_array_nested_access() {
    ShapeTest::new(
        r#"
        let nested = [[1, 2], [3, 4], [5, 6]]
        nested[1][0]
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_array_find_first_match() {
    ShapeTest::new(
        r#"
        [10, 20, 30].find(|x| x > 15)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_array_some_true() {
    ShapeTest::new(
        r#"
        [1, 2, 3].some(|x| x > 2)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_array_every_true() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 0)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_array_every_false() {
    ShapeTest::new(
        r#"
        [2, 4, 6].every(|x| x > 3)
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// 5. HashMap (10 tests)
// =========================================================================

#[test]
fn test_hashmap_construction_and_get() {
    ShapeTest::new(
        r#"
        HashMap().set("key", "val").get("key")
    "#,
    )
    .expect_string("val");
}

#[test]
fn test_hashmap_set_multiple_keys() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        m.get("b")
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_hashmap_has_existing_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("x", 10)
        m.has("x")
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hashmap_has_missing_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("x", 10)
        m.has("y")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hashmap_delete_key() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        let m2 = m.delete("a")
        m2.has("a")
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hashmap_delete_preserves_other_keys() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2)
        let m2 = m.delete("a")
        m2.get("b")
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_hashmap_length() {
    ShapeTest::new(
        r#"
        let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
        m.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_hashmap_is_empty_true() {
    ShapeTest::new(
        r#"
        HashMap().isEmpty()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_hashmap_is_empty_false() {
    ShapeTest::new(
        r#"
        HashMap().set("x", 1).isEmpty()
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_hashmap_in_function() {
    ShapeTest::new(
        r#"
        fn make_config() {
            HashMap().set("host", "localhost").set("port", 8080)
        }
        let cfg = make_config()
        cfg.get("host")
    "#,
    )
    .expect_string("localhost");
}
