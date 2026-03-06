//! Stress tests for nested if statements: 2-level, 3-level, 5-level deep nesting.

use shape_test::shape_test::ShapeTest;

/// Verifies nested if both conditions true.
#[test]
fn test_nested_if_both_true() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = 10;\n  if a > 0 {\n    if b > 0 { return 1; }\n    else { return 2; }\n  } else {\n    return 3;\n  }\n}\ntest()").expect_number(1.0);
}

/// Verifies nested if outer true inner false.
#[test]
fn test_nested_if_outer_true_inner_false() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let b = -1;\n  if a > 0 {\n    if b > 0 { return 1; }\n    else { return 2; }\n  } else {\n    return 3;\n  }\n}\ntest()").expect_number(2.0);
}

/// Verifies nested if outer false.
#[test]
fn test_nested_if_outer_false() {
    ShapeTest::new("function test() {\n  let a = -1;\n  let b = 10;\n  if a > 0 {\n    if b > 0 { return 1; }\n    else { return 2; }\n  } else {\n    return 3;\n  }\n}\ntest()").expect_number(3.0);
}

/// Verifies triple nested if.
#[test]
fn test_triple_nested_if() {
    ShapeTest::new("function test() {\n  let a = 1;\n  let b = 2;\n  let c = 3;\n  if a > 0 {\n    if b > 0 {\n      if c > 0 { return 100; }\n      else { return 99; }\n    } else { return 98; }\n  } else { return 97; }\n}\ntest()").expect_number(100.0);
}

/// Verifies deeply nested if five levels.
#[test]
fn test_deeply_nested_if_five_levels() {
    ShapeTest::new("function test() {\n  if true {\n    if true {\n      if true {\n        if true {\n          if true {\n            return 42;\n          } else { return 0; }\n        } else { return 0; }\n      } else { return 0; }\n    } else { return 0; }\n  } else { return 0; }\n}\ntest()").expect_number(42.0);
}

/// Verifies ternary-style nested if-else expression.
#[test]
fn test_ternary_style_nested() {
    ShapeTest::new("function test() {\n  let a = 5;\n  let result = if a > 10 { \"big\" } else { if a > 0 { \"small\" } else { \"zero\" } }\n  return result\n}\ntest()").expect_string("small");
}
