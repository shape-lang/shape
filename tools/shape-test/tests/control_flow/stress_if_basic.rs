//! Stress tests for basic if/else: true/false branches, comparisons, if without else,
//! if/else-if/else chains, truthiness, complex conditions, sequential if blocks.

use shape_test::shape_test::ShapeTest;

// ===========================================================================
// Section 1: Basic if/else
// ===========================================================================

/// Verifies if true takes true branch.
#[test]
fn test_if_true_branch() {
    ShapeTest::new("function test() {\n  if true { return 1; }\n  return 0;\n}\ntest()").expect_number(1.0);
}

/// Verifies if false skips body.
#[test]
fn test_if_false_branch() {
    ShapeTest::new("function test() {\n  if false { return 1; }\n  return 0;\n}\ntest()").expect_number(0.0);
}

/// Verifies if-else true branch.
#[test]
fn test_if_else_true_branch() {
    ShapeTest::new("function test() {\n  if true { return 10; } else { return 20; }\n}\ntest()").expect_number(10.0);
}

/// Verifies if-else false branch.
#[test]
fn test_if_else_false_branch() {
    ShapeTest::new("function test() {\n  if false { return 10; } else { return 20; }\n}\ntest()").expect_number(20.0);
}

/// Verifies if with greater-than comparison.
#[test]
fn test_if_comparison_greater() {
    ShapeTest::new("function test() {\n  if 5 > 3 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with less-than comparison.
#[test]
fn test_if_comparison_less() {
    ShapeTest::new("function test() {\n  if 3 < 5 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with equality comparison.
#[test]
fn test_if_comparison_equal() {
    ShapeTest::new("function test() {\n  if 7 == 7 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with not-equal comparison.
#[test]
fn test_if_comparison_not_equal() {
    ShapeTest::new("function test() {\n  if 7 != 8 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with greater-than-or-equal.
#[test]
fn test_if_comparison_gte() {
    ShapeTest::new("function test() {\n  if 5 >= 5 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with less-than-or-equal.
#[test]
fn test_if_comparison_lte() {
    ShapeTest::new("function test() {\n  if 3 <= 5 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

// ===========================================================================
// Section 2: If without else
// ===========================================================================

/// Verifies if without else, condition true.
#[test]
fn test_if_without_else_true() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if true { x = 1; }\n  return x;\n}\ntest()").expect_number(1.0);
}

/// Verifies if without else, condition false.
#[test]
fn test_if_without_else_false() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if false { x = 1; }\n  return x;\n}\ntest()").expect_number(0.0);
}

/// Verifies if without else side effects.
#[test]
fn test_if_without_else_side_effect() {
    ShapeTest::new("function test() {\n  let sum = 0;\n  if 1 > 0 { sum = sum + 10; }\n  if 1 < 0 { sum = sum + 100; }\n  return sum;\n}\ntest()").expect_number(10.0);
}

// ===========================================================================
// Section 3: If/else-if/else chains
// ===========================================================================

/// Verifies else-if first branch.
#[test]
fn test_else_if_first_branch() {
    ShapeTest::new("function test() {\n  let x = 10;\n  if x > 5 { return 1; }\n  else if x > 0 { return 2; }\n  else { return 3; }\n}\ntest()").expect_number(1.0);
}

/// Verifies else-if second branch.
#[test]
fn test_else_if_second_branch() {
    ShapeTest::new("function test() {\n  let x = 3;\n  if x > 5 { return 1; }\n  else if x > 0 { return 2; }\n  else { return 3; }\n}\ntest()").expect_number(2.0);
}

/// Verifies else-if else branch.
#[test]
fn test_else_if_else_branch() {
    ShapeTest::new("function test() {\n  let x = -1;\n  if x > 5 { return 1; }\n  else if x > 0 { return 2; }\n  else { return 3; }\n}\ntest()").expect_number(3.0);
}

/// Verifies three else-if branches.
#[test]
fn test_three_else_if_branches() {
    ShapeTest::new("function test() {\n  let x = 50;\n  if x > 100 { return 1; }\n  else if x > 75 { return 2; }\n  else if x > 25 { return 3; }\n  else { return 4; }\n}\ntest()").expect_number(3.0);
}

/// Verifies four else-if branches.
#[test]
fn test_four_else_if_branches() {
    ShapeTest::new("function test() {\n  let x = 5;\n  if x == 1 { return 10; }\n  else if x == 2 { return 20; }\n  else if x == 3 { return 30; }\n  else if x == 4 { return 40; }\n  else if x == 5 { return 50; }\n  else { return 60; }\n}\ntest()").expect_number(50.0);
}

/// Verifies else-if chain last resort.
#[test]
fn test_else_if_chain_last_resort() {
    ShapeTest::new("function test() {\n  let x = 99;\n  if x == 1 { return 10; }\n  else if x == 2 { return 20; }\n  else if x == 3 { return 30; }\n  else { return -1; }\n}\ntest()").expect_number(-1.0);
}

// ===========================================================================
// Section 10: Truthiness in conditions
// ===========================================================================

/// Verifies truthiness of true.
#[test]
fn test_truthiness_true() {
    ShapeTest::new("function test() {\n  if true { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies truthiness of false.
#[test]
fn test_truthiness_false() {
    ShapeTest::new("function test() {\n  if false { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies truthiness of zero int (falsy).
#[test]
fn test_truthiness_zero_int() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies truthiness of nonzero int (truthy).
#[test]
fn test_truthiness_nonzero_int() {
    ShapeTest::new("function test() {\n  let x = 42;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies truthiness of negative int (truthy).
#[test]
fn test_truthiness_negative_int() {
    ShapeTest::new("function test() {\n  let x = -1;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies truthiness of None (falsy).
#[test]
fn test_truthiness_none_is_falsy() {
    ShapeTest::new("function test() {\n  let x = None;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies truthiness of nonempty string (truthy).
#[test]
fn test_truthiness_nonempty_string() {
    ShapeTest::new("function test() {\n  let x = \"hello\";\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies truthiness of float zero (falsy).
#[test]
fn test_truthiness_float_zero() {
    ShapeTest::new("function test() {\n  let x = 0.0;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies truthiness of float nonzero (truthy).
#[test]
fn test_truthiness_float_nonzero() {
    ShapeTest::new("function test() {\n  let x = 0.1;\n  if x { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

// ===========================================================================
// Section 11: Complex conditions
// ===========================================================================

/// Verifies AND with both true.
#[test]
fn test_and_both_true() {
    ShapeTest::new("function test() {\n  if true && true { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies AND with one false.
#[test]
fn test_and_one_false() {
    ShapeTest::new("function test() {\n  if true && false { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies OR with both false.
#[test]
fn test_or_both_false() {
    ShapeTest::new("function test() {\n  if false || false { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies OR with one true.
#[test]
fn test_or_one_true() {
    ShapeTest::new("function test() {\n  if false || true { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies NOT true.
#[test]
fn test_not_true() {
    ShapeTest::new("function test() {\n  if !true { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies NOT false.
#[test]
fn test_not_false() {
    ShapeTest::new("function test() {\n  if !false { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies compound AND with comparisons.
#[test]
fn test_compound_and_or() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = 10;\n  if a > 0 && b > 0 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies compound condition mixed AND/OR.
#[test]
fn test_compound_condition_mixed() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = -1;\n  if a > 0 && b > 0 { return 1; }\n  else if a > 0 || b > 0 { return 2; }\n  else { return 3; }\n}\ntest()").expect_number(2.0);
}

/// Verifies negated compound condition.
#[test]
fn test_negated_compound_condition() {
    ShapeTest::new("function test() {\n  let x = true;\n  let y = false;\n  if x && !y { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies three-way AND.
#[test]
fn test_three_way_and() {
    ShapeTest::new("function test() {\n  if 1 > 0 && 2 > 1 && 3 > 2 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies three-way OR.
#[test]
fn test_three_way_or() {
    ShapeTest::new("function test() {\n  if false || false || true { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

// ===========================================================================
// Section 12: Conditional with functions
// ===========================================================================

/// Verifies function call in condition.
#[test]
fn test_function_call_in_condition() {
    ShapeTest::new("function is_positive(n) { return n > 0; }\nfunction test() {\n  if is_positive(5) { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies function call in condition false.
#[test]
fn test_function_call_in_condition_false() {
    ShapeTest::new("function is_positive(n) { return n > 0; }\nfunction test() {\n  if is_positive(-5) { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies function result in else-if.
#[test]
fn test_function_result_in_else_if() {
    ShapeTest::new("function classify(n) {\n  if n > 100 { return \"big\"; }\n  else if n > 0 { return \"small\"; }\n  else { return \"zero_or_neg\"; }\n}\nfunction test() {\n  return classify(50);\n}\ntest()").expect_string("small");
}

// ===========================================================================
// Section 13: Conditional assignment
// ===========================================================================

/// Verifies conditional assignment true.
#[test]
fn test_conditional_assignment_true() {
    ShapeTest::new("function test() {\n  let x = if 5 > 3 { 100 } else { 200 }\n  return x\n}\ntest()").expect_number(100.0);
}

/// Verifies conditional assignment false.
#[test]
fn test_conditional_assignment_false() {
    ShapeTest::new("function test() {\n  let x = if 1 > 3 { 100 } else { 200 }\n  return x\n}\ntest()").expect_number(200.0);
}

/// Verifies conditional reassignment.
#[test]
fn test_conditional_reassignment() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if true { x = 10; }\n  if false { x = 20; }\n  return x;\n}\ntest()").expect_number(10.0);
}

// ===========================================================================
// Section 14: Early return in branches
// ===========================================================================

/// Verifies early return in if.
#[test]
fn test_early_return_in_if() {
    ShapeTest::new("function test() {\n  if true { return 42; }\n  return 0;\n}\ntest()").expect_number(42.0);
}

/// Verifies early return skipped.
#[test]
fn test_early_return_skipped() {
    ShapeTest::new("function test() {\n  if false { return 42; }\n  return 0;\n}\ntest()").expect_number(0.0);
}

/// Verifies early return in loop.
#[test]
fn test_early_return_in_loop() {
    ShapeTest::new("function test() {\n  for i in [1, 2, 3, 4, 5] {\n    if i == 3 { return i; }\n  }\n  return -1;\n}\ntest()").expect_number(3.0);
}

/// Verifies early return in nested if.
#[test]
fn test_early_return_in_nested_if() {
    ShapeTest::new("function test() {\n  let a = 10;\n  let b = 20;\n  if a > 5 {\n    if b > 15 {\n      return a + b;\n    }\n  }\n  return 0;\n}\ntest()").expect_number(30.0);
}

/// Verifies guard clause pattern.
#[test]
fn test_guard_clause_pattern() {
    ShapeTest::new("function process(x) {\n  if x < 0 { return -1; }\n  if x == 0 { return 0; }\n  return x * 2;\n}\nfunction test() {\n  return process(5);\n}\ntest()").expect_number(10.0);
}

/// Verifies guard clause early exit.
#[test]
fn test_guard_clause_early_exit() {
    ShapeTest::new("function process(x) {\n  if x < 0 { return -1; }\n  if x == 0 { return 0; }\n  return x * 2;\n}\nfunction test() {\n  return process(-5);\n}\ntest()").expect_number(-1.0);
}

// ===========================================================================
// Section 20: Complex conditional patterns
// ===========================================================================

/// Verifies if-else with mutation.
#[test]
fn test_if_else_with_mutation() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if true { x = x + 1; }\n  if true { x = x + 2; }\n  if false { x = x + 100; }\n  return x;\n}\ntest()").expect_number(3.0);
}

/// Verifies conditional accumulator.
#[test]
fn test_conditional_accumulator() {
    ShapeTest::new("function test() {\n  let sum = 0;\n  for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] {\n    if i > 5 { sum = sum + i; }\n  }\n  return sum;\n}\ntest()").expect_number(40.0);
}

/// Verifies if with string comparison.
#[test]
fn test_if_with_string_comparison() {
    ShapeTest::new("function test() {\n  let s = \"hello\";\n  if s == \"hello\" { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with string comparison false.
#[test]
fn test_if_with_string_comparison_false() {
    ShapeTest::new("function test() {\n  let s = \"world\";\n  if s == \"hello\" { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies chained if all false.
#[test]
fn test_chained_if_all_false() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if false { x = 1; }\n  if false { x = 2; }\n  if false { x = 3; }\n  return x;\n}\ntest()").expect_number(0.0);
}

/// Verifies chained if last true.
#[test]
fn test_chained_if_last_true() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if false { x = 1; }\n  if false { x = 2; }\n  if true { x = 3; }\n  return x;\n}\ntest()").expect_number(3.0);
}

/// Verifies if with arithmetic condition.
#[test]
fn test_if_with_arithmetic_condition() {
    ShapeTest::new("function test() {\n  let a = 10;\n  let b = 3;\n  if a - b > 5 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with modulo condition (even).
#[test]
fn test_if_with_modulo_condition() {
    ShapeTest::new("function test() {\n  let x = 10;\n  if x % 2 == 0 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with modulo condition (odd).
#[test]
fn test_if_with_modulo_odd() {
    ShapeTest::new("function test() {\n  let x = 7;\n  if x % 2 == 0 { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

// ===========================================================================
// Section 21: Recursive conditionals
// ===========================================================================

/// Verifies recursive fibonacci.
#[test]
fn test_recursive_if_fibonacci() {
    ShapeTest::new("function fib(n) {\n  if n <= 1 { return n; }\n  return fib(n - 1) + fib(n - 2);\n}\nfunction test() {\n  return fib(10);\n}\ntest()").expect_number(55.0);
}

/// Verifies recursive factorial.
#[test]
fn test_recursive_if_factorial() {
    ShapeTest::new("function factorial(n) {\n  if n <= 1 { return 1; }\n  return n * factorial(n - 1);\n}\nfunction test() {\n  return factorial(6);\n}\ntest()").expect_number(720.0);
}

// ===========================================================================
// Section 23: Boolean conditions from variables
// ===========================================================================

/// Verifies bool variable in condition.
#[test]
fn test_bool_variable_in_condition() {
    ShapeTest::new("function test() {\n  let flag = true;\n  if flag { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies bool variable false in condition.
#[test]
fn test_bool_variable_false_in_condition() {
    ShapeTest::new("function test() {\n  let flag = false;\n  if flag { return 1; } else { return 0; }\n}\ntest()").expect_number(0.0);
}

/// Verifies bool negation in condition.
#[test]
fn test_bool_negation_in_condition() {
    ShapeTest::new("function test() {\n  let flag = false;\n  if !flag { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

// ===========================================================================
// Sections 25, 28, 29, 31, 32, 33, 34, 37, 38
// ===========================================================================

/// Verifies if with array length check.
#[test]
fn test_if_array_length_check() {
    ShapeTest::new("function test() {\n  let arr = [1, 2, 3];\n  if arr.length() > 2 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with array access.
#[test]
fn test_if_with_array_access() {
    ShapeTest::new("function test() {\n  let arr = [10, 20, 30];\n  if arr[0] == 10 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if-else chain classifying number as zero.
#[test]
fn test_if_else_chain_classify_number() {
    ShapeTest::new("function classify(n) {\n  if n > 0 { return \"positive\"; }\n  else if n < 0 { return \"negative\"; }\n  else { return \"zero\"; }\n}\nfunction test() { return classify(0); }\ntest()").expect_string("zero");
}

/// Verifies if-else chain classifying positive.
#[test]
fn test_if_else_classify_positive() {
    ShapeTest::new("function classify(n) {\n  if n > 0 { return \"positive\"; }\n  else if n < 0 { return \"negative\"; }\n  else { return \"zero\"; }\n}\nfunction test() { return classify(42); }\ntest()").expect_string("positive");
}

/// Verifies if-else chain classifying negative.
#[test]
fn test_if_else_classify_negative() {
    ShapeTest::new("function classify(n) {\n  if n > 0 { return \"positive\"; }\n  else if n < 0 { return \"negative\"; }\n  else { return \"zero\"; }\n}\nfunction test() { return classify(-7); }\ntest()").expect_string("negative");
}

/// Verifies sequential if blocks.
#[test]
fn test_sequential_if_blocks() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if 1 > 0 { x = x + 1; }\n  if 2 > 0 { x = x + 2; }\n  if 3 > 0 { x = x + 4; }\n  return x;\n}\ntest()").expect_number(7.0);
}

/// Verifies sequential if-else blocks.
#[test]
fn test_sequential_if_else_blocks() {
    ShapeTest::new("function test() {\n  let x = 0;\n  if true { x = x + 1; } else { x = x + 10; }\n  if false { x = x + 100; } else { x = x + 2; }\n  if true { x = x + 4; } else { x = x + 1000; }\n  return x;\n}\ntest()").expect_number(7.0);
}

/// Verifies if with large numbers.
#[test]
fn test_if_large_numbers() {
    ShapeTest::new("function test() {\n  let x = 1000000;\n  if x > 999999 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with float comparison.
#[test]
fn test_if_float_comparison() {
    ShapeTest::new("function test() {\n  let x = 3.14;\n  if x > 3.0 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with float equality.
#[test]
fn test_if_float_equality() {
    ShapeTest::new("function test() {\n  let x = 2.5;\n  if x == 2.5 { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies multiple return paths first.
#[test]
fn test_multiple_return_paths_first() {
    ShapeTest::new("function test() {\n  let x = 1;\n  if x == 1 { return \"first\"; }\n  if x == 2 { return \"second\"; }\n  if x == 3 { return \"third\"; }\n  return \"other\";\n}\ntest()").expect_string("first");
}

/// Verifies multiple return paths fallthrough.
#[test]
fn test_multiple_return_paths_fallthrough() {
    ShapeTest::new("function test() {\n  let x = 99;\n  if x == 1 { return \"first\"; }\n  if x == 2 { return \"second\"; }\n  if x == 3 { return \"third\"; }\n  return \"other\";\n}\ntest()").expect_string("other");
}

/// Verifies conditional over loop sum.
#[test]
fn test_conditional_over_loop_sum() {
    ShapeTest::new("function test() {\n  let sum = 0;\n  for i in [1, 2, 3, 4, 5] {\n    sum = sum + i;\n  }\n  if sum > 10 { return \"big\"; } else { return \"small\"; }\n}\ntest()").expect_string("big");
}

/// Verifies conditional break in loop.
#[test]
fn test_conditional_break_in_loop() {
    ShapeTest::new("function test() {\n  let result = 0;\n  for i in [1, 2, 3, 4, 5] {\n    if i == 3 {\n      result = i;\n      break;\n    }\n  }\n  return result;\n}\ntest()").expect_number(3.0);
}

/// Verifies if with closure-like function condition.
#[test]
fn test_if_with_closure_condition() {
    ShapeTest::new("function check(x: int) -> bool {\n  return x > 5\n}\nfunction test() {\n  if check(10) { return 1 } else { return 0 }\n}\ntest()").expect_number(1.0);
}

/// Verifies if with closure-like function condition false.
#[test]
fn test_if_with_closure_condition_false() {
    ShapeTest::new("function check(x: int) -> bool {\n  return x > 5\n}\nfunction test() {\n  if check(3) { return 1 } else { return 0 }\n}\ntest()").expect_number(0.0);
}

/// Verifies condition with equality chain.
#[test]
fn test_if_condition_with_equality_chain() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = 5;\n  let c = 5;\n  if a == b && b == c { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies condition with inequality chain.
#[test]
fn test_if_condition_inequality_chain() {
    ShapeTest::new("function test() {\n  let a = 1;\n  let b = 2;\n  let c = 3;\n  if a < b && b < c { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies conditional with multiple variables.
#[test]
fn test_conditional_with_multiple_variables() {
    ShapeTest::new("function test() {\n  let a = 2\n  let b = 3\n  let c = 4\n  let d = 10\n  if a + b > c {\n    return d\n  } else {\n    return 0\n  }\n}\ntest()").expect_number(10.0);
}
