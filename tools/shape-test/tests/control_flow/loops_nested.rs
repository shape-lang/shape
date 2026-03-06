//! Nested loop, loop keyword, and loop-as-expression tests.
//!
//! Covers:
//! - Nested loops (for-for, while-while, for-while)
//! - Break in nested loops (inner vs outer)
//! - Continue in nested loops
//! - Loop keyword (infinite loop with break)
//! - Loop as expression / break with value

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Nested loops with break
// =========================================================================

/// Nested for loops where break in inner loop affects iteration.
#[test]
fn cf_11_nested_loop_break() {
    let code = r#"
// Test 11: Nested loops with break
for i in 0..3 {
  for j in 0..3 {
    if j == 2 { break }
    print(f"i={i} j={j}")
  }
}
// Expected: i=0 j=0, i=0 j=1, i=1 j=0, i=1 j=1, i=2 j=0, i=2 j=1
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=0 j=1\ni=1 j=0\ni=1 j=1\ni=2 j=0\ni=2 j=1");
}

/// Debug test for nested loop break -- single loop with break works correctly.
#[test]
fn cf_11b_nested_loop_break_debug() {
    let code = r#"
// Test 11b: Debug nested loop break issue
// The error was: "expected array, string, or range, got option"
// This suggests break turns the range into an option type

// Simple single loop with break works:
for i in 0..5 {
  if i == 3 { break }
  print(f"single: {i}")
}

print("---")

// Nested: outer loop re-enters after inner break
// The outer range seems corrupted after inner break
for i in 0..2 {
  print(f"outer i={i}")
  for j in 0..3 {
    if j == 1 { break }
    print(f"  inner j={j}")
  }
}
"#;
    ShapeTest::new(code).expect_run_ok().expect_output(
        "single: 0\nsingle: 1\nsingle: 2\n---\nouter i=0\n  inner j=0\nouter i=1\n  inner j=0",
    );
}

/// Nested for loops without break (sanity check).
#[test]
fn cf_11c_nested_loop_no_break() {
    let code = r#"
// Test 11c: Nested loops without break (sanity check)
for i in 0..2 {
  for j in 0..2 {
    print(f"i={i} j={j}")
  }
}
// Expected: i=0 j=0, i=0 j=1, i=1 j=0, i=1 j=1
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=0 j=1\ni=1 j=0\ni=1 j=1");
}

/// Nested while loops with break in inner loop.
#[test]
fn cf_11d_nested_while_break() {
    let code = r#"
// Test 11d: Nested while loops with break (does the same bug occur?)
let i = 0
while i < 3 {
  let j = 0
  while j < 3 {
    if j == 1 { break }
    print(f"i={i} j={j}")
    j = j + 1
  }
  i = i + 1
}
// Expected: i=0 j=0, i=1 j=0, i=2 j=0
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=1 j=0\ni=2 j=0");
}

/// Outer for loop with inner while loop containing break.
#[test]
fn cf_11e_for_while_nested_break() {
    let code = r#"
// Test 11e: Outer for, inner while with break
for i in 0..3 {
  let j = 0
  while j < 3 {
    if j == 1 { break }
    print(f"i={i} j={j}")
    j = j + 1
  }
}
// Expected: i=0 j=0, i=1 j=0, i=2 j=0
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=1 j=0\ni=2 j=0");
}

#[test]
fn nested_for_loops() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in [1, 2, 3] {
            for j in [10, 20] {
                sum = sum + i * j
            }
        }
        sum
    "#,
    )
    .expect_number(180.0);
}

#[test]
fn nested_while_loops() {
    ShapeTest::new(
        r#"
        var sum = 0
        var i = 0
        while i < 10 {
            var j = 0
            while j < 10 {
                sum = sum + 1
                j = j + 1
            }
            i = i + 1
        }
        sum
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn break_from_inner_loop_does_not_affect_outer() {
    ShapeTest::new(
        r#"
        var r = 0
        for i in [1, 2, 3] {
            for j in [10, 20, 30] {
                if j == 20 { break }
            }
            r = r + i
        }
        r
    "#,
    )
    .expect_number(6.0);
}

// =========================================================================
// Continue in nested loops
// =========================================================================

/// Continue in inner loop skips the current inner iteration only.
#[test]
fn cf_28_continue_in_nested() {
    let code = r#"
// Test 28: Continue in nested for loops
for i in 0..3 {
  for j in 0..3 {
    if j == 1 { continue }
    print(f"i={i} j={j}")
  }
}
// Expected: i=0 j=0, i=0 j=2, i=1 j=0, i=1 j=2, i=2 j=0, i=2 j=2
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=0 j=2\ni=1 j=0\ni=1 j=2\ni=2 j=0\ni=2 j=2");
}

/// Continue in inner loop does not affect the outer loop.
#[test]
fn cf_30_nested_for_continue_outer() {
    let code = r#"
// Test 30: Does continue only affect innermost loop?
for i in 0..3 {
  for j in 0..3 {
    if j == 1 { continue }
    print(f"i={i} j={j}")
  }
  print(f"end outer i={i}")
}
// Expected: i=0 j=0, i=0 j=2, end outer i=0, i=1 j=0, i=1 j=2, end outer i=1, ...
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("i=0 j=0\ni=0 j=2\nend outer i=0\ni=1 j=0\ni=1 j=2\nend outer i=1\ni=2 j=0\ni=2 j=2\nend outer i=2");
}

#[test]
fn nested_for_with_continue_in_inner() {
    // Skip even j values
    ShapeTest::new(
        r#"
        var sum = 0
        for i in [1, 2] {
            for j in [1, 2, 3, 4] {
                if j % 2 == 0 { continue }
                sum = sum + i * j
            }
        }
        sum
    "#,
    )
    .expect_number(12.0);
}

// =========================================================================
// Loop keyword (infinite loop)
// =========================================================================

// BUG: `break expr` inside loop does not propagate as the loop's return value.
// The loop always evaluates to Null regardless of break value.
#[test]
fn loop_with_break_value_is_null_bug() {
    ShapeTest::new(
        r#"
        var i = 0
        loop {
            i = i + 1
            if i == 5 { break }
        }
        i
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn loop_with_counter_and_break() {
    ShapeTest::new(
        r#"
        var count = 0
        loop {
            count = count + 1
            if count >= 10 { break }
        }
        count
    "#,
    )
    .expect_number(10.0);
}

// BUG: `break expr` does not propagate value from loop (see loop_with_break_value_is_null_bug).
// Workaround: use a mutable variable to capture the result before breaking.
#[test]
fn loop_break_with_result_workaround() {
    ShapeTest::new(
        r#"
        var i = 0
        let items = ["apple", "banana", "cherry"]
        var result = "not found"
        loop {
            if items[i] == "banana" {
                result = "found banana"
                break
            }
            i = i + 1
            if i >= items.length { break }
        }
        result
    "#,
    )
    .expect_string("found banana");
}

// =========================================================================
// Loop as expression / break with value
// =========================================================================

/// A for loop used as an expression returns the last iteration value.
#[test]
fn cf_17_loop_as_expression() {
    let code = r#"
// Test 17: Does for/while return a value?
let result = for i in 0..3 { i }
print(result)
// Expected: some value, () , or error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
}

/// For and while loop return values in various scenarios.
#[test]
fn cf_17b_for_loop_value() {
    let code = r#"
// Test 17b: What exactly does a for loop return?
let r1 = for i in 0..3 { i }
print(f"for result: {r1}")

let r2 = for i in [10, 20, 30] { i * 2 }
print(f"for array result: {r2}")

// Does while also return a value?
let r3 = while false { 42 }
print(f"while false result: {r3}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("for result: 2\nfor array result: 60\nwhile false result: None");
}

/// A while-false loop used as an expression returns None.
#[test]
fn cf_21_while_loop_expression() {
    let code = r#"
// Test 21: While loop as expression
let result = while false { 42 }
print(result)
// Expected: () or error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("None");
}

/// Break with a value returns that value from the loop expression.
#[test]
fn cf_38_break_with_value() {
    let code = r#"
// Test 38: Break with value (Rust-style)
let result = while true {
  break 42
}
print(result)
// Expected: 42 or syntax error
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}
