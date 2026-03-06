//! Basic loop tests: for, while, break, continue, ranges.
//!
//! Covers:
//! - For loop with range (exclusive, inclusive, reverse, large, step)
//! - For loop over array and string
//! - For loop destructuring (BUG-CF-005)
//! - For loop variable mutation
//! - While loop (basic, while-true-break, while-false, compound condition)
//! - Break and continue (simple, in while)
//!
//! See also: `loops_nested` for nested loops, loop keyword, and loop-as-expression.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// For loop with range
// =========================================================================

/// For loop iterates over an exclusive range.
#[test]
fn cf_04_for_range() {
    let code = r#"
// Test 04: For loops with ranges
for i in 0..5 {
  print(i)
}
// Expected: 0 1 2 3 4
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n1\n2\n3\n4");
}

#[test]
fn for_loop_with_range() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 0..5 {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

/// Reverse range (5..0) produces no iterations; loop body is skipped.
#[test]
fn cf_13_reverse_range() {
    let code = r#"
// Test 13: Reverse range
for i in 5..0 {
  print(i)
}
print("end")
// Expected: either 5 4 3 2 1 or nothing (if reverse not supported)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("end");
}

/// Inclusive range `0..=5` includes the upper bound.
#[test]
fn cf_14_inclusive_range() {
    let code = r#"
// Test 14: Inclusive range
for i in 0..=5 {
  print(i)
}
// Expected: 0 1 2 3 4 5 (if ..= syntax supported)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n1\n2\n3\n4\n5");
}

/// BUG-CF-029: Range step syntax is not implemented.
/// The `step` keyword after a range literal is not recognized by the parser,
/// which reports an unexpected `}` token.
#[test]
fn cf_29_range_step() {
    let code = r#"
// Test 29: Range with step (if supported)
for i in 0..10 step 2 {
  print(i)
}
// Expected: 0 2 4 6 8 or syntax error
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n2\n4\n6\n8");
}

/// Fixed: Range iteration now produces i64 values, so accumulation
/// over large ranges works correctly.
#[test]
fn cf_35_large_range() {
    let code = r#"
// Test 35: Large range (performance check)
let sum = 0
for i in 0..10000 {
  sum = sum + i
}
print(sum)
// Expected: 49995000
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("49995000");
}

// =========================================================================
// For loop over array
// =========================================================================

/// For loop iterates over array elements.
#[test]
fn cf_10_for_array() {
    let code = r#"
// Test 10: For loop over an array
for item in [1, 2, 3] {
  print(item)
}
// Expected: 1 2 3
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n2\n3");
}

#[test]
fn for_loop_over_array_print() {
    ShapeTest::new(
        r#"
        for x in [1, 2, 3] {
            print(x)
        }
    "#,
    )
    .expect_output("1\n2\n3");
}

#[test]
fn for_loop_accumulator() {
    ShapeTest::new(
        r#"
        var sum = 0
        for x in [10, 20, 30] {
            sum = sum + x
        }
        sum
    "#,
    )
    .expect_number(60.0);
}

#[test]
fn for_loop_over_string_array() {
    ShapeTest::new(
        r#"
        var result = ""
        for s in ["a", "b", "c"] {
            result = result + s
        }
        result
    "#,
    )
    .expect_string("abc");
}

#[test]
fn for_loop_empty_array() {
    // Iterating over empty array should not execute body
    ShapeTest::new(
        r#"
        var x = 42
        for item in [] {
            x = 0
        }
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn for_loop_counting_elements() {
    ShapeTest::new(
        r#"
        var count = 0
        for x in [1, 2, 3, 4, 5] {
            count = count + 1
        }
        count
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn for_loop_with_conditional_accumulation() {
    // Sum only positive values
    ShapeTest::new(
        r#"
        var sum = 0
        for x in [1, -2, 3, -4, 5] {
            if x > 0 { sum = sum + x }
        }
        sum
    "#,
    )
    .expect_number(9.0);
}

#[test]
fn for_loop_with_if_else_body() {
    ShapeTest::new(
        r#"
        var evens = 0
        var odds = 0
        for x in [1, 2, 3, 4, 5, 6] {
            if x % 2 == 0 {
                evens = evens + 1
            } else {
                odds = odds + 1
            }
        }
        evens
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn for_loop_building_result_array() {
    ShapeTest::new(
        r#"
        var result = []
        for x in [1, 2, 3] {
            result = result.push(x * 2)
        }
        result.length
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// For loop over string
// =========================================================================

/// For loop iterates over characters of a string.
#[test]
fn cf_26_for_string_iteration() {
    let code = r#"
// Test 26: For loop over string characters
for ch in "hello" {
  print(ch)
}
// Expected: h e l l o
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("h\ne\nl\nl\no");
}

// =========================================================================
// For loop destructuring (BUG)
// =========================================================================

/// BUG-CF-005: For-loop destructuring is not implemented.
/// Destructuring bindings (`for {x, y} in items`) should work but the
/// semantic analyzer reports "Undefined variable 'x'" because destructuring
/// patterns in for-loop heads are not yet supported.
#[test]
fn cf_05_for_destructuring() {
    let code = r#"
// Test 05: For loops with destructuring
let points = [{x: 1, y: 2}, {x: 3, y: 4}]
for {x, y} in points {
  print(f"({x}, {y})")
}
// Expected: (1, 2) then (3, 4)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("(1, 2)\n(3, 4)");
}

// =========================================================================
// For loop variable mutation
// =========================================================================

/// Fixed: Range iteration now produces i64 values instead of f64,
/// so int arithmetic in for-loop bodies works correctly.
#[test]
fn cf_20_for_var_mutation() {
    let code = r#"
// Test 20: For loop variable mutation inside body
for i in 0..5 {
  i = i + 10
  print(i)
}
// Expected: error or 10 11 12 13 14 (depending on if loop var is mutable)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("10\n11\n12\n13\n14");
}

// =========================================================================
// While loop
// =========================================================================

/// Basic while loop with a counter.
#[test]
fn cf_06_while_loop() {
    let code = r#"
// Test 06: While loops
let i = 0
while i < 3 {
  print(i)
  i = i + 1
}
// Expected: 0 1 2
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n1\n2");
}

#[test]
fn while_loop_basic_counter() {
    ShapeTest::new(
        r#"
        var i = 0
        var sum = 0
        while i < 5 {
            sum = sum + i
            i = i + 1
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn while_loop_sum_to_100() {
    ShapeTest::new(
        r#"
        var sum = 0
        var i = 1
        while i <= 100 {
            sum = sum + i
            i = i + 1
        }
        sum
    "#,
    )
    .expect_number(5050.0);
}

#[test]
fn while_loop_never_enters() {
    ShapeTest::new(
        r#"
        var x = 0
        while false {
            x = 99
        }
        x
    "#,
    )
    .expect_number(0.0);
}

/// While-false loop body never executes.
#[test]
fn cf_34_while_false() {
    let code = r#"
// Test 34: while false body should never execute
while false {
  print("should not print")
}
print("done")
// Expected: done
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("done");
}

#[test]
fn while_loop_decrementing() {
    ShapeTest::new(
        r#"
        var n = 10
        var result = 1
        while n > 0 {
            result = result * n
            n = n - 1
        }
        result
    "#,
    )
    .expect_number(3628800.0);
}

#[test]
fn while_loop_with_compound_condition() {
    ShapeTest::new(
        r#"
        var i = 0
        var sum = 0
        while i < 20 and sum < 50 {
            sum = sum + i
            i = i + 1
        }
        sum
    "#,
    )
    .expect_number(55.0);
}

#[test]
fn while_loop_simulating_do_while() {
    // Execute body at least once, then check condition
    ShapeTest::new(
        r#"
        var i = 10
        var ran = false
        while true {
            ran = true
            if i < 5 {
                i = i + 1
            } else {
                break
            }
        }
        ran
    "#,
    )
    .expect_bool(true);
}

// =========================================================================
// While true with break
// =========================================================================

/// Infinite while-true loop guarded by break.
#[test]
fn cf_12_while_true_break() {
    let code = r#"
// Test 12: while true with break (infinite loop guard)
let count = 0
while true {
  if count >= 5 { break }
  print(count)
  count = count + 1
}
print("done")
// Expected: 0 1 2 3 4 done
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n1\n2\n3\n4\ndone");
}

#[test]
fn while_loop_with_break() {
    ShapeTest::new(
        r#"
        var i = 0
        while true {
            if i >= 5 { break }
            i = i + 1
        }
        i
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn while_loop_early_break_on_condition() {
    // Find first element > 5
    ShapeTest::new(
        r#"
        let arr = [1, 3, 7, 2, 9]
        var found = -1
        var i = 0
        while i < arr.length {
            if arr[i] > 5 {
                found = arr[i]
                break
            }
            i = i + 1
        }
        found
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// Break and continue
// =========================================================================

/// Break exits the loop; continue skips the current iteration.
#[test]
fn cf_07_break_continue() {
    let code = r#"
// Test 07: Break and continue
for i in 0..10 {
  if i == 2 { continue }
  if i == 6 { break }
  print(i)
}
// Expected: 0 1 3 4 5
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("0\n1\n3\n4\n5");
}

/// While loop combining continue and break.
#[test]
fn cf_19_while_break_continue() {
    let code = r#"
// Test 19: Continue and break in while loops
let i = 0
while i < 10 {
  i = i + 1
  if i == 3 { continue }
  if i == 7 { break }
  print(i)
}
// Expected: 1 2 4 5 6
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n2\n4\n5\n6");
}

#[test]
fn while_loop_with_continue() {
    // Sum only even numbers from 0 to 9
    ShapeTest::new(
        r#"
        var sum = 0
        var i = 0
        while i < 10 {
            i = i + 1
            if i % 2 != 0 { continue }
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(30.0);
}
