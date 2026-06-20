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
        let mut arr = [1, 2]
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
        let a: Array<int> = []
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
            HashMap().set("host", "localhost").set("port", "8080")
        }
        let cfg = make_config()
        cfg.get("host")
    "#,
    )
    .expect_string("localhost");
}

// =========================================================================
// ROOT-1 (F2, strict-flip 2026-06-18): arr[i] INDEX-READ element-type
// erasure for a method-returned array (split) and a nested-array loop var.
// Pre-fix, `let parts = "a,b,c".split(","); parts[0] + parts[1]` and
// `for p in [[1,2],[3,4]] { p[0]*10 + p[1] }` surfaced "Cannot infer ... :
// operand types are `unknown` and ..." because the binding never recorded
// the array element ConcreteType (split's monomorphic Array<string> return
// was lost; the nested-array literal recorded a phantom placeholder element).
// =========================================================================

#[test]
fn test_index_read_split_returned_array_element_concat() {
    // finding 2: a `.split()`-returned `Array<string>` keeps its element
    // type at the index read, so `parts[0] + parts[1]` is `string` concat.
    ShapeTest::new(
        r#"
        let parts = "a,b,c".split(",")
        parts[0] + parts[1]
    "#,
    )
    .expect_string("ab");
}

#[test]
fn test_index_read_split_chain_method_on_element() {
    // `m.split(",")[0].toUpperCase()` — split -> Array<string>, index-read
    // unwraps to string, the method resolves on the recovered element.
    ShapeTest::new(
        r#"
        let m = "hello,world"
        m.split(",")[0].toUpperCase()
    "#,
    )
    .expect_string("HELLO");
}

#[test]
fn test_index_read_nested_int_array_loop_var_arithmetic() {
    // finding 4: a nested-array literal's loop var binds the inner
    // `Array<int>`; `p[0]` / `p[1]` index-reads recover `int`, so the
    // arithmetic `p[0]*10 + p[1]` is well-typed.
    ShapeTest::new(
        r#"
        let pairs = [[1, 2], [3, 4]]
        let mut total = 0
        for p in pairs {
            total = total + (p[0] * 10 + p[1])
        }
        total
    "#,
    )
    .expect_number(46.0);
}

#[test]
fn test_index_read_map_returned_array_element_concat() {
    // A `.map()`-returned array keeps its element type at the index read.
    ShapeTest::new(
        r#"
        let xs = [1, 2, 3].map(|x| x * 2)
        xs[0] + xs[1]
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_index_read_nested_number_array_no_int_coercion() {
    // int != number must not unify: a nested `number` array stays `number`.
    ShapeTest::new(
        r#"
        let pairs = [[1.0, 2.0], [3.0, 4.0]]
        let mut total = 0.0
        for p in pairs {
            total = total + (p[0] + p[1])
        }
        total
    "#,
    )
    .expect_number(10.0);
}

// =========================================================================
// STAGE F1 (strict-flip, 2026-06-20): re-tighten the T1 any-sink.
// A field read off an element whose type is known ONLY from a `push` into an
// UNANNOTATED empty array (`[]`) is unprovable WITHOUT an annotation, so it is
// a CLEAN compile-error — NOT an `any`-typed result that would accept an
// ill-typed program or let `int`/`number` silently unify.
// =========================================================================

#[test]
fn stage_f1_unannotated_empty_push_accumulator_field_read_is_compile_error() {
    // The element type of `rs` is known only from the push into an UNANNOTATED
    // `[]`; `rs[0].n` must be a clean compile-error, not `any`.
    ShapeTest::new(
        r#"
        type Run { n: int }
        let mut rs = []
        rs = rs.push(Run { n: 1 })
        rs[0].n + 1
    "#,
    )
    .expect_run_err_contains("annotate the array");
}

#[test]
fn stage_f1_unannotated_empty_push_accumulator_field_read_not_any_sink() {
    // Before STAGE F1 this was accepted (the field resolved to `any`):
    // `bool := rs[0].n` where `n: int`. It must now be rejected.
    ShapeTest::new(
        r#"
        type Run { n: int }
        let mut rs = []
        rs = rs.push(Run { n: 1 })
        let bad: bool = rs[0].n
        bad
    "#,
    )
    .expect_run_err();
}

#[test]
fn stage_f1_annotated_empty_push_accumulator_field_read_works() {
    // The SAME accumulator with a DECLARED `Array<Run>` annotation has a proven
    // element type — the field read resolves and arithmetic works.
    ShapeTest::new(
        r#"
        type Run { n: int }
        let mut rs: Array<Run> = []
        rs = rs.push(Run { n: 4 })
        rs[0].n + 1
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn stage_f1_nonempty_struct_literal_array_field_read_works() {
    // The non-empty literal form proves the element STRUCTURALLY — stays accepted.
    ShapeTest::new(
        r#"
        type Run { n: int }
        let rs = [Run { n: 4 }]
        rs[0].n + 1
    "#,
    )
    .expect_number(5.0);
}
