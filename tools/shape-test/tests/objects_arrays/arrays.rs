//! Array-related tests
//! Covers array creation, indexing, methods, chaining, edge cases, and destructuring.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// Basic Arrays
// =====================================================================

#[test]
fn array_literal_creation() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums)")
        .expect_run_ok()
        .expect_output_contains("[1, 2, 3, 4]");
}

#[test]
fn array_indexing_zero_based() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums[0])")
        .expect_run_ok()
        .expect_output("1");
}

#[test]
fn array_indexing_second_element() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums[1])")
        .expect_run_ok()
        .expect_output("2");
}

#[test]
fn array_indexing_last_element_by_index() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums[3])")
        .expect_run_ok()
        .expect_output("4");
}

#[test]
fn array_negative_indexing_last() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums[-1])")
        .expect_run_ok()
        .expect_output("4");
}

#[test]
fn array_negative_indexing_second_last() {
    ShapeTest::new("let nums = [1, 2, 3, 4]\nprint(nums[-2])")
        .expect_run_ok()
        .expect_output("3");
}

// =====================================================================
// Array Methods - map, filter, reduce
// =====================================================================

#[test]
fn array_map_doubles() {
    ShapeTest::new(
        r#"let nums = [1, 2, 3, 4]
let doubled = nums.map(|x| x * 2)
print(doubled)"#,
    )
    .expect_run_ok()
    .expect_output_contains("2");
}

// Phase 4b Round 2 LANG-9 regression: inline array literal receivers
// (`[1,2,3,4,5].map(|x|x*2).sum()`) must monomorphize the method call
// the same way bound receivers (`let xs = [1,2,3,4,5]; xs.map(...).sum()`)
// do. Pre-fix, the inline form hung indefinitely (exit 124 after timeout)
// because `concrete_type_for_expr(Expr::Array)` returned `None` —
// `compile_expr_array` never recorded the array literal's element
// ConcreteType against its AST span, so the bytecode monomorphizer
// fell back to the generic `Vec.map` stub. Per ADR-006 §2.7.5
// stamp-at-compile-time, the producer's typed-kind IS the proof; the
// fix records that proof in `array_element_types[span]` at
// construction time.
#[test]
fn inline_array_literal_map_sum() {
    ShapeTest::new(r#"print([1,2,3,4,5].map(|x|x*2).sum())"#)
        .expect_run_ok()
        .expect_output("30");
}

#[test]
fn inline_array_literal_map_only() {
    ShapeTest::new(r#"print([1,2,3,4,5].map(|x| x * 2))"#)
        .expect_run_ok()
        .expect_output_contains("10");
}

#[test]
fn inline_array_literal_filter() {
    ShapeTest::new(r#"print([1,2,3,4,5].filter(|x| x > 2).sum())"#)
        .expect_run_ok()
        .expect_output("12");
}

#[test]
fn inline_array_literal_map_then_let_then_sum() {
    ShapeTest::new(
        r#"let r = [1,2,3,4,5].map(|x|x*2).sum()
print(r)"#,
    )
    .expect_run_ok()
    .expect_output("30");
}

// Phase 4b Round 4 W15 LANG-9-spin-3-first regression: VM/JIT divergence
// on `.first()` dispatch over typed-array receivers. Pre-fix VM
// surfaced `TypeError: expected object, array, string, or other heap
// value, got scalar` for inline `[1,2,3,4,5].first()` while JIT
// returned `1` (F3 exit-code AND output divergent); the chained
// `[1,2,3,4,5].map(|x|x*2).first()` form returned VM=`2` but JIT=`None`
// (F3b result-correctness bug). Per ADR-006 §2.7.5 producer-side stamp +
// §2.7.10/Q11 method-dispatch ABI: VM `dispatch_get_prop` now reads
// v2-typed-array receivers via the `UInt64` carrier + `read_element`
// projection (matching the producer's stamped element-type byte from
// `v2_handlers/array.rs`); JIT `try_emit_v2_array_method` now inlines
// the element-0 / element-(len-1) read for `first`/`last` matching the
// VM PHF `typed_int_array_methods::first` return-kind contract.
//
// Audit: `docs/cluster-audits/v0.3-w15-lang-9-spinoffs-audit.md` §4.2.
#[test]
fn inline_array_literal_first() {
    ShapeTest::new(r#"print([1,2,3,4,5].first())"#)
        .expect_run_ok()
        .expect_output("1");
}

#[test]
fn inline_array_literal_map_then_first() {
    ShapeTest::new(r#"print([1,2,3,4,5].map(|x|x*2).first())"#)
        .expect_run_ok()
        .expect_output("2");
}

#[test]
fn inline_array_literal_last() {
    ShapeTest::new(r#"print([1,2,3,4,5].last())"#)
        .expect_run_ok()
        .expect_output("5");
}

#[test]
fn inline_array_literal_map_then_last() {
    ShapeTest::new(r#"print([1,2,3,4,5].map(|x|x*2).last())"#)
        .expect_run_ok()
        .expect_output("10");
}

#[test]
fn bound_array_first_typed_int() {
    ShapeTest::new(
        r#"let arr = [1,2,3,4,5]
print(arr.first())"#,
    )
    .expect_run_ok()
    .expect_output("1");
}

#[test]
fn bound_array_last_typed_int() {
    ShapeTest::new(
        r#"let arr = [10,20,30,40,50]
print(arr.last())"#,
    )
    .expect_run_ok()
    .expect_output("50");
}

#[test]
fn array_filter_evens() {
    ShapeTest::new(
        r#"let nums = [1, 2, 3, 4]
let evens = nums.filter(|x| x % 2 == 0)
print(evens)"#,
    )
    .expect_run_ok()
    .expect_output_contains("2");
}

#[test]
fn array_reduce_sum() {
    ShapeTest::new(
        r#"let nums = [1, 2, 3, 4]
let sum = nums.reduce(|acc, x| acc + x, 0)
print(sum)"#,
    )
    .expect_run_ok()
    .expect_output("10");
}

// =====================================================================
// Additional Array Methods
// =====================================================================

#[test]
fn array_find_method() {
    let code = r#"let nums = [1, 2, 3, 4, 5]
let found = nums.find(|x| x > 3)
print(found)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("4");
}

#[test]
fn array_first_method() {
    let code = r#"let nums = [10, 20, 30]
print(nums.first())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("10");
}

#[test]
fn array_last_method() {
    let code = r#"let nums = [10, 20, 30]
print(nums.last())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

#[test]
fn array_sort_with_comparator() {
    let code = r#"let nums = [3, 1, 4, 1, 5]
let sorted = nums.sort(|a, b| a - b)
print(sorted)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_foreach() {
    // forEach output is not captured by the test harness output mechanism.
    // Just verify it runs without error.
    let code = r#"let nums = [1, 2, 3]
nums.forEach(|x| print(x))"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_some_true() {
    let code = r#"let nums = [1, 2, 3, 4]
let has_even = nums.some(|x| x % 2 == 0)
print(has_even)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

#[test]
fn array_some_false() {
    let code = r#"let nums = [1, 3, 5, 7]
let has_even = nums.some(|x| x % 2 == 0)
print(has_even)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

#[test]
fn array_every_true() {
    let code = r#"let nums = [2, 4, 6, 8]
let all_even = nums.every(|x| x % 2 == 0)
print(all_even)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

#[test]
fn array_every_false() {
    let code = r#"let nums = [2, 4, 5, 8]
let all_even = nums.every(|x| x % 2 == 0)
print(all_even)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

#[test]
fn array_flatmap() {
    let code = r#"let nums = [1, 2, 3]
let result = nums.flatMap(|x| [x, x * 10])
print(result)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_groupby() {
    let code = r#"let nums = [1, 2, 3, 4, 5, 6]
let groups = nums.groupBy(|x| x % 2)
print(groups)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_length_property() {
    let code = r#"let nums = [1, 2, 3, 4]
print(nums.length)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("4");
}

// =====================================================================
// Edge Cases - Empty and Single Element Arrays
// =====================================================================

#[test]
fn empty_array() {
    let code = r#"let a = []
print(a)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn single_element_array() {
    let code = r#"let a = [42]
print(a[0])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

// =====================================================================
// Nested Arrays
// =====================================================================

#[test]
fn nested_arrays() {
    let code = r#"let matrix = [[1, 2], [3, 4]]
print(matrix)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn nested_array_access() {
    let code = r#"let matrix = [[1, 2], [3, 4]]
print(matrix[0][0])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1");
}

#[test]
fn nested_array_second_row_access() {
    let code = r#"let matrix = [[1, 2], [3, 4]]
print(matrix[1][1])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("4");
}

// =====================================================================
// Mixed Types in Arrays
// =====================================================================

#[test]
fn array_of_mixed_types() {
    let code = r#"let mixed = [1, "hello", true]
print(mixed)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_of_objects() {
    let code = r#"let users = [
  { name: "Ada", age: 30 },
  { name: "Bob", age: 25 }
]
print(users[0].name)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("Ada");
}

#[test]
fn array_of_objects_second_element() {
    let code = r#"let users = [
  { name: "Ada", age: 30 },
  { name: "Bob", age: 25 }
]
print(users[1].age)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("25");
}

// =====================================================================
// Bounds and Errors
// =====================================================================

#[test]
fn array_negative_index_first_element() {
    // [-4] on a 4-element array should give the first element
    let code = r#"let nums = [1, 2, 3, 4]
print(nums[-4])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1");
}

#[test]
fn array_out_of_bounds_positive_returns_none() {
    // Array out-of-bounds returns None in Shape (runtime prints "None").
    let code = r#"let nums = [1, 2, 3]
print(nums[5])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("None");
}

#[test]
fn array_out_of_bounds_negative_returns_none() {
    // Array out-of-bounds returns None in Shape (runtime prints "None").
    let code = r#"let nums = [1, 2, 3]
print(nums[-5])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("None");
}

// =====================================================================
// Array Concatenation
// =====================================================================

#[test]
fn array_concatenation_with_plus() {
    let code = r#"let a = [1, 2]
let b = [3, 4]
let c = a + b
print(c)"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// Array len Function
// =====================================================================

#[test]
fn array_len_function() {
    let code = r#"let nums = [1, 2, 3, 4, 5]
print(nums.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("5");
}

#[test]
fn empty_array_len() {
    let code = r#"let a = []
print(a.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("0");
}

// =====================================================================
// Array Chaining
// =====================================================================

#[test]
fn array_method_chaining() {
    let code = r#"let nums = [1, 2, 3, 4, 5, 6]
let result = nums.filter(|x| x % 2 == 0).map(|x| x * 10)
print(result)"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// Array String Elements
// =====================================================================

#[test]
fn array_of_strings() {
    let code = r#"let names = ["Alice", "Bob", "Charlie"]
print(names[1])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("Bob");
}

#[test]
fn array_of_strings_negative_index() {
    let code = r#"let names = ["Alice", "Bob", "Charlie"]
print(names[-1])"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Charlie");
}

// =====================================================================
// Array Reduce with Different Accumulators
// =====================================================================

#[test]
fn array_reduce_product() {
    let code = r#"let nums = [1, 2, 3, 4]
let product = nums.reduce(|acc, x| acc * x, 1)
print(product)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("24");
}

#[test]
fn array_reduce_string_concat() {
    let code = r#"let words = ["hello", " ", "world"]
let sentence = words.reduce(|acc, w| acc + w, "")
print(sentence)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("hello world");
}

// =====================================================================
// Array with Boolean Elements
// =====================================================================

#[test]
fn array_of_booleans() {
    let code = r#"let flags = [true, false, true]
print(flags[0])
print(flags[1])"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("true\nfalse");
}

// =====================================================================
// Complex Compositions (Array-focused)
// =====================================================================

#[test]
fn array_of_objects_with_map() {
    let code = r#"let users = [
  { name: "Ada", score: 90 },
  { name: "Bob", score: 80 }
]
let names = users.map(|u| u.name)
print(names)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn array_of_objects_with_filter() {
    let code = r#"let users = [
  { name: "Ada", score: 90 },
  { name: "Bob", score: 80 },
  { name: "Charlie", score: 95 }
]
let top = users.filter(|u| u.score > 85)
print(top.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
}

// =====================================================================
// Array Destructuring in Function Parameters
// =====================================================================

#[test]
fn array_destructuring_in_function_param() {
    let code = r#"fn sum_pair([a, b]) {
    return a + b
}
print(sum_pair([10, 20]))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}
