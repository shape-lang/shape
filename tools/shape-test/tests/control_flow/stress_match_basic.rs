//! Stress tests for match expressions: literal patterns, wildcard, multiple arms,
//! match as expression, guards, nested match, identifier patterns, array/object patterns,
//! bool patterns, computed scrutinee, and match with blocks.

use shape_test::shape_test::ShapeTest;

// ===========================================================================
// Section 6: Match with literals
// ===========================================================================

/// Verifies match int literal first arm.
#[test]
fn test_match_int_literal_first_arm() {
    ShapeTest::new("function test() {\n  return match 1 { 1 => 10, 2 => 20, _ => 0 };\n}\ntest()")
        .expect_number(10.0);
}

/// Verifies match int literal second arm.
#[test]
fn test_match_int_literal_second_arm() {
    ShapeTest::new("function test() {\n  return match 2 { 1 => 10, 2 => 20, _ => 0 };\n}\ntest()")
        .expect_number(20.0);
}

/// Verifies match int literal wildcard.
#[test]
fn test_match_int_literal_wildcard() {
    ShapeTest::new("function test() {\n  return match 99 { 1 => 10, 2 => 20, _ => 0 };\n}\ntest()")
        .expect_number(0.0);
}

/// Verifies match string literal.
#[test]
fn test_match_string_literal() {
    ShapeTest::new("function test() {\n  return match \"hello\" {\n    \"hi\" => 1,\n    \"hello\" => 2,\n    _ => 0\n  };\n}\ntest()").expect_number(2.0);
}

/// Verifies match string wildcard.
#[test]
fn test_match_string_wildcard() {
    ShapeTest::new("function test() {\n  return match \"unknown\" {\n    \"hi\" => 1,\n    \"hello\" => 2,\n    _ => 0\n  };\n}\ntest()").expect_number(0.0);
}

/// Verifies match variable scrutinee.
#[test]
fn test_match_variable_scrutinee() {
    ShapeTest::new("function test() {\n  let x = 3;\n  return match x { 1 => 10, 2 => 20, 3 => 30, _ => 0 };\n}\ntest()").expect_number(30.0);
}

/// Verifies match negative literal.
#[test]
fn test_match_negative_literal() {
    ShapeTest::new("function test() {\n  let x = -1;\n  return match x { -1 => 100, 0 => 200, 1 => 300, _ => 0 };\n}\ntest()").expect_number(100.0);
}

// ===========================================================================
// Section 7: Match with wildcard
// ===========================================================================

/// Verifies match with only wildcard.
#[test]
fn test_match_only_wildcard() {
    ShapeTest::new("function test() {\n  return match 42 { _ => 99 };\n}\ntest()")
        .expect_number(99.0);
}

/// Verifies match wildcard captures all unmatched.
#[test]
fn test_match_wildcard_captures_all() {
    ShapeTest::new("function test() {\n  let mut sum = 0;\n  for i in [1, 2, 3, 4, 5] {\n    sum = sum + match i {\n      1 => 10,\n      3 => 30,\n      5 => 50,\n      _ => 0\n    };\n  }\n  return sum;\n}\ntest()").expect_number(90.0);
}

// ===========================================================================
// Section 8: Match with multiple arms
// ===========================================================================

/// Verifies match two arms.
#[test]
fn test_match_two_arms() {
    ShapeTest::new("function test() {\n  return match 1 { 1 => 10, _ => 0 };\n}\ntest()")
        .expect_number(10.0);
}

/// Verifies match five arms.
#[test]
fn test_match_five_arms() {
    ShapeTest::new("function test() {\n  let x = 4;\n  return match x {\n    1 => 10,\n    2 => 20,\n    3 => 30,\n    4 => 40,\n    _ => 0\n  };\n}\ntest()").expect_number(40.0);
}

/// Verifies match ten arms.
#[test]
fn test_match_ten_arms() {
    ShapeTest::new("function test() {\n  let x = 7;\n  return match x {\n    1 => 100,\n    2 => 200,\n    3 => 300,\n    4 => 400,\n    5 => 500,\n    6 => 600,\n    7 => 700,\n    8 => 800,\n    9 => 900,\n    _ => 0\n  };\n}\ntest()").expect_number(700.0);
}

// ===========================================================================
// Section 9: Match as expression
// ===========================================================================

/// Verifies match as expression assign.
#[test]
fn test_match_as_expression_assign() {
    ShapeTest::new("function test() {\n  let x = 2;\n  let y = match x { 1 => \"one\", 2 => \"two\", _ => \"other\" };\n  return y;\n}\ntest()").expect_string("two");
}

/// Verifies match as expression return.
#[test]
fn test_match_as_expression_return() {
    ShapeTest::new("function test() {\n  return match 5 { 5 => 500, _ => 0 };\n}\ntest()")
        .expect_number(500.0);
}

/// Verifies match expression in arithmetic.
#[test]
fn test_match_expression_in_arithmetic() {
    ShapeTest::new("function test() {\n  let a = match 1 { 1 => 10, _ => 0 };\n  let b = match 2 { 2 => 20, _ => 0 };\n  return a + b;\n}\ntest()").expect_number(30.0);
}

/// Verifies match with block body.
#[test]
fn test_match_with_block_body() {
    ShapeTest::new("function test() {\n  return match 3 {\n    3 => {\n      let a = 10;\n      let b = 20;\n      a + b\n    },\n    _ => 0\n  };\n}\ntest()").expect_number(30.0);
}

// ===========================================================================
// Section 15: Match with guards
// ===========================================================================

/// Verifies match with guard true.
#[test]
fn test_match_with_guard_true() {
    ShapeTest::new("function test() {\n  return match 10 {\n    x where x > 5 => x,\n    _ => 0\n  };\n}\ntest()").expect_number(10.0);
}

/// Verifies match with guard false.
#[test]
fn test_match_with_guard_false() {
    ShapeTest::new("function test() {\n  return match 3 {\n    x where x > 5 => x,\n    _ => 0\n  };\n}\ntest()").expect_number(0.0);
}

/// Verifies match guard multiple arms.
#[test]
fn test_match_guard_multiple_arms() {
    ShapeTest::new("function test() {\n  return match 15 {\n    x where x > 100 => 1,\n    x where x > 10 => 2,\n    x where x > 0 => 3,\n    _ => 0\n  };\n}\ntest()").expect_number(2.0);
}

/// Verifies match guard with literal fallthrough.
#[test]
fn test_match_guard_with_literal_fallthrough() {
    ShapeTest::new("function test() {\n  let x = 3;\n  return match x {\n    v where v > 10 => 100,\n    v where v > 5 => 50,\n    v where v > 0 => 10,\n    _ => 0\n  };\n}\ntest()").expect_number(10.0);
}

/// Verifies match guard negative values.
#[test]
fn test_match_guard_negative_values() {
    ShapeTest::new("function test() {\n  let x = -5;\n  return match x {\n    v where v > 0 => 1,\n    v where v == 0 => 0,\n    v where v < 0 => -1,\n    _ => 999\n  };\n}\ntest()").expect_number(-1.0);
}

// ===========================================================================
// Section 16: Nested match
// ===========================================================================

/// Verifies nested match.
#[test]
fn test_nested_match() {
    ShapeTest::new("function test() {\n  let x = 2;\n  let y = 3;\n  return match x {\n    1 => match y { 1 => 11, _ => 19 },\n    2 => match y { 3 => 23, _ => 29 },\n    _ => 0\n  };\n}\ntest()").expect_number(23.0);
}

/// Verifies match inside if.
#[test]
fn test_match_inside_if() {
    ShapeTest::new("function test() {\n  let use_match = true;\n  let x = 2;\n  if use_match {\n    return match x { 1 => 10, 2 => 20, _ => 0 };\n  } else {\n    return -1;\n  }\n}\ntest()").expect_number(20.0);
}

/// Verifies if inside match.
#[test]
fn test_if_inside_match() {
    ShapeTest::new("function test() {\n  let x = 2;\n  return match x {\n    2 => {\n      if true { 200 } else { 0 }\n    },\n    _ => -1\n  };\n}\ntest()").expect_number(200.0);
}

/// Verifies match deeply nested.
#[test]
fn test_match_deeply_nested() {
    ShapeTest::new("function test() {\n  let a = 1;\n  return match a {\n    1 => {\n      let b = 2;\n      match b {\n        2 => {\n          let c = 3;\n          match c {\n            3 => 123,\n            _ => 0\n          }\n        },\n        _ => 0\n      }\n    },\n    _ => 0\n  };\n}\ntest()").expect_number(123.0);
}

// ===========================================================================
// Section 17: Match with identifier patterns
// ===========================================================================

/// Verifies match identifier binding.
#[test]
fn test_match_identifier_binding() {
    ShapeTest::new("function test() {\n  return match 42 {\n    x => x + 1\n  };\n}\ntest()")
        .expect_number(43.0);
}

/// Verifies match identifier with prior literal.
#[test]
fn test_match_identifier_with_prior_literal() {
    ShapeTest::new("function test() {\n  return match 5 {\n    1 => 100,\n    2 => 200,\n    x => x * 10\n  };\n}\ntest()").expect_number(50.0);
}

// ===========================================================================
// Section 18: Match with array patterns
// ===========================================================================

/// Verifies match array pattern basic.
#[test]
fn test_match_array_pattern_basic() {
    ShapeTest::new("function test() {\n  return match [1, 2] {\n    [a, b] => a + b,\n    _ => 0\n  };\n}\ntest()").expect_number(3.0);
}

/// Verifies match array pattern three elements.
#[test]
fn test_match_array_pattern_three_elements() {
    ShapeTest::new("function test() {\n  return match [10, 20, 30] {\n    [a, b, c] => a + b + c,\n    _ => 0\n  };\n}\ntest()").expect_number(60.0);
}

// ===========================================================================
// Section 19: Match with object patterns
// ===========================================================================

/// Verifies match object pattern basic.
#[test]
fn test_match_object_pattern_basic() {
    ShapeTest::new("function test() {\n  return match { a: 1, b: 2 } {\n    { a: x, b: y } => x + y,\n    _ => 0\n  };\n}\ntest()").expect_number(3.0);
}

/// Verifies match object pattern nested fields.
#[test]
fn test_match_object_pattern_nested_fields() {
    ShapeTest::new("function test() {\n  return match { x: 10, y: 20 } {\n    { x: a, y: b } => a * b,\n    _ => 0\n  };\n}\ntest()").expect_number(200.0);
}

// ===========================================================================
// Section 22: Match without commas
// ===========================================================================

/// Verifies match without commas.
#[test]
fn test_match_no_commas() {
    ShapeTest::new("function test() {\n  return match 2 {\n    1 => 10\n    2 => 20\n    _ => 0\n  };\n}\ntest()").expect_number(20.0);
}

// ===========================================================================
// Section 24: Match with bool pattern
// ===========================================================================

/// Verifies match bool true.
#[test]
fn test_match_bool_true() {
    ShapeTest::new("function test() {\n  return match true {\n    true => 1,\n    false => 0,\n    _ => -1\n  };\n}\ntest()").expect_number(1.0);
}

/// Verifies match bool false.
#[test]
fn test_match_bool_false() {
    ShapeTest::new("function test() {\n  return match false {\n    true => 1,\n    false => 0,\n    _ => -1\n  };\n}\ntest()").expect_number(0.0);
}

// ===========================================================================
// Section 26: Match expression top-level
// ===========================================================================

/// Verifies match as top-level expression.
#[test]
fn test_match_top_level_expression() {
    ShapeTest::new("match 5 {\n  1 => 10,\n  5 => 50,\n  _ => 0\n}").expect_number(50.0);
}

// ===========================================================================
// Section 27: Match with computed scrutinee
// ===========================================================================

/// Verifies match with computed scrutinee.
#[test]
fn test_match_computed_scrutinee() {
    ShapeTest::new("function test() {\n  let a = 2;\n  let b = 3;\n  return match (a + b) {\n    4 => 40,\n    5 => 50,\n    6 => 60,\n    _ => 0\n  };\n}\ntest()").expect_number(50.0);
}

// ===========================================================================
// Additional match tests
// ===========================================================================

/// Verifies match assignment.
#[test]
fn test_match_assignment() {
    ShapeTest::new("function test() {\n  let x = 2;\n  let label = match x {\n    1 => \"one\",\n    2 => \"two\",\n    3 => \"three\",\n    _ => \"unknown\"\n  };\n  return label;\n}\ntest()").expect_string("two");
}

/// Verifies match in loop.
#[test]
fn test_match_in_loop() {
    ShapeTest::new("function test() {\n  let mut sum = 0;\n  for i in [1, 2, 3] {\n    sum = sum + match i {\n      1 => 10,\n      2 => 20,\n      3 => 30,\n      _ => 0\n    };\n  }\n  return sum;\n}\ntest()").expect_number(60.0);
}

/// Verifies match returns string from int.
#[test]
fn test_match_returns_string_from_int() {
    ShapeTest::new("function test() {\n  let code = 2;\n  return match code {\n    1 => \"error\",\n    2 => \"warning\",\n    3 => \"info\",\n    _ => \"unknown\"\n  };\n}\ntest()").expect_string("warning");
}

/// Verifies match with zero.
#[test]
fn test_match_with_zero() {
    ShapeTest::new("function test() {\n  return match 0 {\n    0 => \"zero\",\n    _ => \"nonzero\"\n  };\n}\ntest()").expect_string("zero");
}

/// Verifies match all arms return same type.
#[test]
fn test_match_all_arms_return_same_type() {
    ShapeTest::new("function test() {\n  let x = 3;\n  return match x {\n    1 => 100,\n    2 => 200,\n    3 => 300,\n    _ => 999\n  };\n}\ntest()").expect_number(300.0);
}

/// Verifies match first arm matches immediately.
#[test]
fn test_match_first_arm_matches_immediately() {
    ShapeTest::new("function test() {\n  return match 1 {\n    1 => 999,\n    2 => 0,\n    3 => 0,\n    _ => 0\n  };\n}\ntest()").expect_number(999.0);
}

/// Verifies match last literal arm matches.
#[test]
fn test_match_last_literal_arm_matches() {
    ShapeTest::new("function test() {\n  return match 4 {\n    1 => 100,\n    2 => 200,\n    3 => 300,\n    4 => 400\n  };\n}\ntest()").expect_number(400.0);
}

/// Verifies if then match.
#[test]
fn test_if_then_match() {
    ShapeTest::new("function test() {\n  let x = 5;\n  if x > 0 {\n    return match x {\n      5 => 500,\n      _ => 0\n    };\n  } else {\n    return -1;\n  }\n}\ntest()").expect_number(500.0);
}

/// Verifies match then if.
#[test]
fn test_match_then_if() {
    ShapeTest::new("function test() {\n  let code = match 3 {\n    1 => \"a\",\n    2 => \"b\",\n    3 => \"c\",\n    _ => \"x\"\n  };\n  if code == \"c\" { return 1; } else { return 0; }\n}\ntest()").expect_number(1.0);
}

/// Verifies match string equality.
#[test]
fn test_match_string_equality() {
    ShapeTest::new("function test() {\n  let cmd = \"run\";\n  return match cmd {\n    \"help\" => 0,\n    \"run\" => 1,\n    \"test\" => 2,\n    _ => -1\n  };\n}\ntest()").expect_number(1.0);
}

/// Verifies match with block and locals.
#[test]
fn test_match_with_block_and_locals() {
    ShapeTest::new("function test() {\n  let x = 3;\n  return match x {\n    1 => {\n      let r = 10;\n      r\n    },\n    2 => {\n      let r = 20;\n      r\n    },\n    3 => {\n      let r = 30;\n      r\n    },\n    _ => 0\n  };\n}\ntest()").expect_number(30.0);
}

/// Verifies function call in match scrutinee.
#[test]
fn test_function_call_in_match_scrutinee() {
    ShapeTest::new("function double(n) { return n * 2; }\nfunction test() {\n  return match double(3) {\n    4 => \"four\",\n    6 => \"six\",\n    _ => \"other\"\n  };\n}\ntest()").expect_string("six");
}

/// Verifies function call in match arm body.
#[test]
fn test_function_call_in_match_arm_body() {
    ShapeTest::new("function square(n) { return n * n; }\nfunction test() {\n  return match 3 {\n    1 => square(1),\n    2 => square(2),\n    3 => square(3),\n    _ => 0\n  };\n}\ntest()").expect_number(9.0);
}

/// Verifies match some constructor.
#[test]
fn test_match_some_constructor() {
    ShapeTest::new("function test() {\n  return match { value: 42 } {\n    Ok(x) => x,\n    _ => 0\n  };\n}\ntest()").expect_number(0.0);
}

/// Verifies match expression sum.
#[test]
fn test_match_expression_sum() {
    ShapeTest::new("function test() {\n  let a = match 1 { 1 => 10, _ => 0 };\n  let b = match 2 { 2 => 20, _ => 0 };\n  let c = match 3 { 3 => 30, _ => 0 };\n  return a + b + c;\n}\ntest()").expect_number(60.0);
}
