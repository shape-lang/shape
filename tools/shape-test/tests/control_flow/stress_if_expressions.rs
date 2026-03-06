//! Stress tests for if-as-expression: assignment, return, block body, string result,
//! and if expressions as function arguments.

use shape_test::shape_test::ShapeTest;

/// Verifies if as expression true branch.
#[test]
fn test_if_as_expression_true() {
    ShapeTest::new("function test() {\n  let x = if true { 10 } else { 20 }\n  return x\n}\ntest()").expect_number(10.0);
}

/// Verifies if as expression false branch.
#[test]
fn test_if_as_expression_false() {
    ShapeTest::new("function test() {\n  let x = if false { 10 } else { 20 }\n  return x\n}\ntest()").expect_number(20.0);
}

/// Verifies if expression in return.
#[test]
fn test_if_expr_in_return() {
    ShapeTest::new("function test() {\n  return if 3 > 2 { 42 } else { 0 }\n}\ntest()").expect_number(42.0);
}

/// Verifies if expression with block body.
#[test]
fn test_if_expr_with_block_body() {
    ShapeTest::new("function test() {\n  let x = if true {\n    let a = 10;\n    let b = 20;\n    a + b\n  } else {\n    0\n  }\n  return x\n}\ntest()").expect_number(30.0);
}

/// Verifies if expression chained assignment.
#[test]
fn test_if_expr_chained_assignment() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = if a > 3 { a * 2 } else { a }\n  return b\n}\ntest()").expect_number(10.0);
}

/// Verifies if expression string result.
#[test]
fn test_if_expr_string_result() {
    ShapeTest::new("function test() {\n  let x = if true { \"yes\" } else { \"no\" }\n  return x\n}\ntest()").expect_string("yes");
}

/// Verifies if expression string false branch.
#[test]
fn test_if_expr_string_false_branch() {
    ShapeTest::new("function test() {\n  let x = if false { \"yes\" } else { \"no\" }\n  return x\n}\ntest()").expect_string("no");
}

/// Verifies if as top-level expression.
#[test]
fn test_if_as_top_level_expression() {
    ShapeTest::new("if 3 > 2 { 100 } else { 0 }").expect_number(100.0);
}

/// Verifies if-else both branches string.
#[test]
fn test_if_expr_both_branches_string() {
    ShapeTest::new("function test() {\n  let x = if 1 > 0 { \"yes\" } else { \"no\" }\n  return x\n}\ntest()").expect_string("yes");
}

/// Verifies if-else as function arg.
#[test]
fn test_if_else_as_function_arg() {
    ShapeTest::new("function double(n) { return n * 2; }\nfunction test() {\n  let flag = true;\n  return double(if flag { 5 } else { 10 });\n}\ntest()").expect_number(10.0);
}
