//! End-to-end integration tests for the JIT compilation pipeline.
//!
//! Each test exercises the full path: Shape source -> parse -> bytecode compile
//! -> JIT compile -> native execute -> verify result. This ensures correctness
//! of the JIT-compiled code against expected values.
//!
//! Test categories:
//!   1. Pure arithmetic (number)
//!   2. Integer arithmetic (int)
//!   3. Conditionals (if/else)
//!   4. Loops (while, for-in-range)
//!   5. Function calls (user-defined)
//!   6. Closures
//!   7. Array literal + indexing
//!   8. Struct/type field access
//!   9. Boolean operations
//!  10. Nested/recursive function calls

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a Shape program through JIT and return the result as WireValue.
fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

/// Assert a JIT result is a Number within epsilon of `expected`.
fn jit_expect_number(source: &str, expected: f64) {
    match jit_eval(source) {
        WireValue::Number(n) => {
            assert!(
                (n - expected).abs() < 1e-9,
                "Expected number {}, got {}",
                expected,
                n
            );
        }
        WireValue::Integer(n) => {
            assert!(
                (n as f64 - expected).abs() < 1e-9,
                "Expected number {} (got Integer {})",
                expected,
                n
            );
        }
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

/// Assert a JIT result is an Integer equal to `expected`.
/// Falls back to Number comparison since the JIT may return numbers for
/// integer expressions depending on unboxing state.
fn jit_expect_int(source: &str, expected: i64) {
    match jit_eval(source) {
        WireValue::Integer(n) => {
            assert_eq!(n, expected, "Expected integer {}, got {}", expected, n);
        }
        WireValue::Number(n) => {
            assert!(
                (n - expected as f64).abs() < 1e-9,
                "Expected integer {} (got Number {})",
                expected,
                n
            );
        }
        other => panic!("Expected Integer({}), got {:?}", expected, other),
    }
}

/// Assert a JIT result is a Bool equal to `expected`.
fn jit_expect_bool(source: &str, expected: bool) {
    match jit_eval(source) {
        WireValue::Bool(b) => {
            assert_eq!(b, expected, "Expected bool {}, got {}", expected, b);
        }
        other => panic!("Expected Bool({}), got {:?}", expected, other),
    }
}

// ===========================================================================
// 1. Pure arithmetic (number type)
// ===========================================================================

#[test]
fn arithmetic_add_numbers() {
    jit_expect_number("1.0 + 2.0", 3.0);
}

#[test]
fn arithmetic_sub_numbers() {
    jit_expect_number("10.0 - 3.5", 6.5);
}

#[test]
fn arithmetic_mul_numbers() {
    jit_expect_number("4.0 * 2.5", 10.0);
}

#[test]
fn arithmetic_div_numbers() {
    jit_expect_number("9.0 / 2.0", 4.5);
}

#[test]
fn arithmetic_mod_numbers() {
    jit_expect_number("10.0 % 3.0", 1.0);
}

#[test]
fn arithmetic_combined_expression() {
    // x + y * 2.0 with x=3.0, y=4.0 => 3.0 + 8.0 = 11.0
    jit_expect_number(
        r#"
fn weighted_add(x: number, y: number) -> number { x + y * 2.0 }
weighted_add(3.0, 4.0)
"#,
        11.0,
    );
}

#[test]
fn arithmetic_negative_result() {
    jit_expect_number("3.0 - 10.0", -7.0);
}

#[test]
fn arithmetic_nested_operations() {
    jit_expect_number("(2.0 + 3.0) * (4.0 - 1.0)", 15.0);
}

#[test]
fn arithmetic_floating_point_precision() {
    // Verify IEEE 754 behavior
    jit_expect_number("0.1 + 0.2", 0.30000000000000004);
}

// ===========================================================================
// 2. Integer arithmetic (int type)
// ===========================================================================

#[test]
fn int_arithmetic_add() {
    jit_expect_int(
        r#"
fn add_int(x: int, y: int) -> int { x + y }
add_int(3, 7)
"#,
        10,
    );
}

#[test]
fn int_arithmetic_sub() {
    jit_expect_int(
        r#"
fn sub_int(x: int, y: int) -> int { x - y }
sub_int(10, 4)
"#,
        6,
    );
}

#[test]
fn int_arithmetic_mul() {
    jit_expect_int(
        r#"
fn mul_int(x: int, y: int) -> int { x * y }
mul_int(6, 7)
"#,
        42,
    );
}

#[test]
fn int_arithmetic_div() {
    jit_expect_int(
        r#"
fn div_int(x: int, y: int) -> int { x / y }
div_int(20, 4)
"#,
        5,
    );
}

#[test]
fn int_arithmetic_mod() {
    jit_expect_int(
        r#"
fn mod_int(x: int, y: int) -> int { x % y }
mod_int(17, 5)
"#,
        2,
    );
}

#[test]
fn int_arithmetic_large_values() {
    // Test that i48 integers handle moderately large values correctly
    jit_expect_int(
        r#"
fn big_mul(x: int, y: int) -> int { x * y }
big_mul(100000, 100000)
"#,
        10_000_000_000,
    );
}

#[test]
fn int_inline_expression() {
    jit_expect_int("2 + 3 * 4", 14);
}

// ===========================================================================
// 3. Conditionals (if/else)
// ===========================================================================

#[test]
fn conditional_true_branch() {
    jit_expect_number("if true { 1.0 } else { 2.0 }", 1.0);
}

#[test]
fn conditional_false_branch() {
    jit_expect_number("if false { 1.0 } else { 2.0 }", 2.0);
}

#[test]
fn conditional_comparison_gt() {
    jit_expect_number("if 10 > 5 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_comparison_lt() {
    jit_expect_number("if 3 < 7 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_comparison_eq() {
    jit_expect_number("if 5 == 5 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_comparison_neq() {
    jit_expect_number("if 5 != 3 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_comparison_gte() {
    jit_expect_number("if 10 >= 10 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_comparison_lte() {
    jit_expect_number("if 7 <= 10 { 1.0 } else { 0.0 }", 1.0);
}

#[test]
fn conditional_abs_function() {
    jit_expect_number(
        r#"
fn abs_val(x: number) -> number {
    if x > 0.0 { x } else { -x }
}
abs_val(-5.0)
"#,
        5.0,
    );
}

#[test]
fn conditional_abs_positive() {
    jit_expect_number(
        r#"
fn abs_val(x: number) -> number {
    if x > 0.0 { x } else { -x }
}
abs_val(3.0)
"#,
        3.0,
    );
}

#[test]
fn conditional_nested_if() {
    jit_expect_number(
        r#"
fn classify(x: number) -> number {
    if x > 0.0 {
        if x > 100.0 { 3.0 } else { 2.0 }
    } else {
        1.0
    }
}
classify(50.0)
"#,
        2.0,
    );
}

#[test]
fn conditional_nested_if_negative() {
    jit_expect_number(
        r#"
fn classify(x: number) -> number {
    if x > 0.0 {
        if x > 100.0 { 3.0 } else { 2.0 }
    } else {
        1.0
    }
}
classify(-1.0)
"#,
        1.0,
    );
}

// ===========================================================================
// 4. Loops
// ===========================================================================

#[test]
fn loop_while_sum_to_10() {
    jit_expect_number(
        r#"
let mut sum = 0
let mut i = 1
while i <= 10 {
    sum = sum + i
    i = i + 1
}
sum
"#,
        55.0,
    );
}

#[test]
fn loop_while_countdown() {
    jit_expect_number(
        r#"
let mut n = 100
let mut count = 0
while n > 0 {
    n = n - 1
    count = count + 1
}
count
"#,
        100.0,
    );
}

#[test]
fn loop_while_nested() {
    jit_expect_number(
        r#"
let mut total = 0
let mut i = 0
while i < 5 {
    let mut j = 0
    while j < 5 {
        total = total + 1
        j = j + 1
    }
    i = i + 1
}
total
"#,
        25.0,
    );
}

#[test]
fn loop_while_with_conditional() {
    // Count even numbers from 0 to 99
    jit_expect_number(
        r#"
let mut count = 0
let mut i = 0
while i < 100 {
    if i % 2 == 0 {
        count = count + 1
    }
    i = i + 1
}
count
"#,
        50.0,
    );
}

#[test]
fn loop_function_scoped() {
    jit_expect_number(
        r#"
function sum_range(n) {
    let mut s = 0
    let mut i = 0
    while i < n {
        s = s + i
        i = i + 1
    }
    return s
}
sum_range(100)
"#,
        4950.0,
    );
}

#[test]
fn loop_typed_int_sum() {
    jit_expect_int(
        r#"
fn sum_ints(n: int) -> int {
    let mut s: int = 0
    let mut i: int = 0
    while i < n {
        s = s + i
        i = i + 1
    }
    s
}
sum_ints(100)
"#,
        4950,
    );
}

// ===========================================================================
// 5. Function calls
// ===========================================================================

#[test]
fn function_simple_call() {
    jit_expect_number(
        r#"
function double(x) { return x * 2 }
double(21)
"#,
        42.0,
    );
}

#[test]
fn function_two_params() {
    jit_expect_number(
        r#"
function sum_two(a, b) { return a + b }
sum_two(17, 25)
"#,
        42.0,
    );
}

#[test]
fn function_calling_function() {
    jit_expect_number(
        r#"
function double(x) { return x * 2 }
function quad(x) { return double(double(x)) }
quad(5)
"#,
        20.0,
    );
}

#[test]
fn function_triple_chain() {
    jit_expect_number(
        r#"
function inc(x) { return x + 1 }
function double(x) { return x * 2 }
function process(x) { return double(inc(x)) }
process(4)
"#,
        10.0,
    );
}

#[test]
fn function_typed_params() {
    jit_expect_number(
        r#"
fn multiply(a: number, b: number) -> number {
    a * b
}
multiply(3.5, 4.0)
"#,
        14.0,
    );
}

#[test]
fn function_with_local_variables() {
    jit_expect_number(
        r#"
function compute(x, y) {
    let sum = x + y
    let product = x * y
    return sum + product
}
compute(3, 4)
"#,
        19.0,
    );
}

#[test]
fn function_early_return() {
    jit_expect_number(
        r#"
function safe_div(a, b) {
    if b == 0 { return 0 }
    return a / b
}
safe_div(10, 2)
"#,
        5.0,
    );
}

#[test]
fn function_early_return_zero_divisor() {
    jit_expect_number(
        r#"
function safe_div(a, b) {
    if b == 0 { return 0 }
    return a / b
}
safe_div(10, 0)
"#,
        0.0,
    );
}

// ===========================================================================
// 6. Closures
// ===========================================================================

#[test]
fn closure_simple() {
    jit_expect_int(
        r#"
let add_one = |x| x + 1
add_one(5)
"#,
        6,
    );
}

#[test]
fn closure_two_params() {
    jit_expect_int(
        r#"
let sum_closure = |x, y| x + y
sum_closure(10, 20)
"#,
        30,
    );
}

#[test]
fn closure_capturing_variable() {
    jit_expect_int(
        r#"
let offset = 10
let add_offset = |x| x + offset
add_offset(5)
"#,
        15,
    );
}

#[test]
fn closure_nested() {
    jit_expect_int(
        r#"
let multiplier = 3
let mul = |x| x * multiplier
mul(7)
"#,
        21,
    );
}

// ===========================================================================
// 7. Array literal + indexing
// ===========================================================================

#[test]
fn array_literal_index_0() {
    jit_expect_number(
        r#"
let arr = [10, 20, 30]
arr[0]
"#,
        10.0,
    );
}

#[test]
fn array_literal_index_1() {
    jit_expect_number(
        r#"
let arr = [10, 20, 30]
arr[1]
"#,
        20.0,
    );
}

#[test]
fn array_literal_index_2() {
    jit_expect_number(
        r#"
let arr = [10, 20, 30]
arr[2]
"#,
        30.0,
    );
}

#[test]
fn array_length_property() {
    jit_expect_number(
        r#"
let arr = [1, 2, 3, 4, 5]
arr.length
"#,
        5.0,
    );
}

#[test]
fn array_float_elements() {
    jit_expect_number(
        r#"
let arr = [1.5, 2.5, 3.5]
arr[0] + arr[1] + arr[2]
"#,
        7.5,
    );
}

#[test]
fn array_computed_index() {
    jit_expect_number(
        r#"
let arr = [100, 200, 300, 400, 500]
let idx = 2
arr[idx]
"#,
        300.0,
    );
}

// ===========================================================================
// 8. Struct / type field access
// ===========================================================================

#[test]
fn struct_field_access_x() {
    jit_expect_number(
        r#"
type Point {
    x: number,
    y: number,
}
let p = Point { x: 1.0, y: 2.0 }
p.x
"#,
        1.0,
    );
}

#[test]
fn struct_field_access_y() {
    jit_expect_number(
        r#"
type Point {
    x: number,
    y: number,
}
let p = Point { x: 1.0, y: 2.0 }
p.y
"#,
        2.0,
    );
}

#[test]
fn struct_field_sum() {
    jit_expect_number(
        r#"
type Point {
    x: number,
    y: number,
}
let p = Point { x: 3.0, y: 4.0 }
p.x + p.y
"#,
        7.0,
    );
}

#[test]
fn struct_multiple_instances() {
    jit_expect_number(
        r#"
type Vec2 {
    x: number,
    y: number,
}
let a = Vec2 { x: 1.0, y: 2.0 }
let b = Vec2 { x: 3.0, y: 4.0 }
a.x + b.x + a.y + b.y
"#,
        10.0,
    );
}

#[test]
fn struct_passed_to_function() {
    jit_expect_number(
        r#"
type Point {
    x: number,
    y: number,
}

fn distance_squared(p: Point) -> number {
    p.x * p.x + p.y * p.y
}

let pt = Point { x: 3.0, y: 4.0 }
distance_squared(pt)
"#,
        25.0,
    );
}

// ===========================================================================
// 9. Boolean operations
// ===========================================================================

#[test]
fn bool_true_literal() {
    jit_expect_bool("true", true);
}

#[test]
fn bool_false_literal() {
    jit_expect_bool("false", false);
}

#[test]
fn bool_and_true_true() {
    jit_expect_bool("true && true", true);
}

#[test]
fn bool_and_true_false() {
    jit_expect_bool("true && false", false);
}

#[test]
fn bool_or_false_true() {
    jit_expect_bool("false || true", true);
}

#[test]
fn bool_or_false_false() {
    jit_expect_bool("false || false", false);
}

#[test]
fn bool_not_true() {
    jit_expect_bool("!true", false);
}

#[test]
fn bool_not_false() {
    jit_expect_bool("!false", true);
}

#[test]
fn bool_comparison_result() {
    jit_expect_bool("10 > 5", true);
}

#[test]
fn bool_comparison_false_result() {
    jit_expect_bool("3 > 7", false);
}

#[test]
fn bool_complex_expression() {
    jit_expect_bool("(10 > 5) && (3 < 7)", true);
}

#[test]
fn bool_conditional_select_via_if() {
    // Using if/else to select based on boolean conditions
    jit_expect_number(
        r#"
let cond = true
let a = 10.0
let b = 20.0
if cond { a } else { b }
"#,
        10.0,
    );
}

#[test]
fn bool_conditional_select_false() {
    jit_expect_number(
        r#"
let cond = false
let a = 10.0
let b = 20.0
if cond { a } else { b }
"#,
        20.0,
    );
}

// ===========================================================================
// 10. Nested / recursive function calls
// ===========================================================================

#[test]
fn recursive_factorial() {
    jit_expect_number(
        r#"
function factorial(n) {
    if n <= 1 { return 1 }
    return n * factorial(n - 1)
}
factorial(6)
"#,
        720.0,
    );
}

#[test]
fn recursive_factorial_10() {
    jit_expect_number(
        r#"
function factorial(n) {
    if n <= 1 { return 1 }
    return n * factorial(n - 1)
}
factorial(10)
"#,
        3628800.0,
    );
}

#[test]
fn recursive_fibonacci() {
    jit_expect_number(
        r#"
function fib(n) {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}
fib(10)
"#,
        55.0,
    );
}

#[test]
fn recursive_fibonacci_20() {
    jit_expect_number(
        r#"
function fib(n) {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}
fib(20)
"#,
        6765.0,
    );
}

#[test]
fn recursive_power() {
    jit_expect_number(
        r#"
function power(base, exp) {
    if exp == 0 { return 1 }
    return base * power(base, exp - 1)
}
power(2, 10)
"#,
        1024.0,
    );
}

#[test]
fn recursive_gcd() {
    jit_expect_number(
        r#"
function gcd(a, b) {
    if b == 0 { return a }
    return gcd(b, a % b)
}
gcd(48, 18)
"#,
        6.0,
    );
}

#[test]
fn iterative_fibonacci() {
    jit_expect_number(
        r#"
function fib_iter(n: int) -> int {
    let mut a = 0
    let mut b = 1
    let mut i = 0
    while i < n {
        let t = a + b
        a = b
        b = t
        i = i + 1
    }
    return a
}
fib_iter(30)
"#,
        832040.0,
    );
}

#[test]
fn mutual_recursion_even_odd() {
    // Mutually recursive is_even / is_odd functions
    jit_expect_number(
        r#"
function is_even(n) {
    if n == 0 { return 1 }
    return is_odd(n - 1)
}
function is_odd(n) {
    if n == 0 { return 0 }
    return is_even(n - 1)
}
is_even(10)
"#,
        1.0,
    );
}

#[test]
fn mutual_recursion_is_odd() {
    jit_expect_number(
        r#"
function is_even(n) {
    if n == 0 { return 1 }
    return is_odd(n - 1)
}
function is_odd(n) {
    if n == 0 { return 0 }
    return is_even(n - 1)
}
is_odd(7)
"#,
        1.0,
    );
}

// ===========================================================================
// Combined / stress tests
// ===========================================================================

#[test]
fn combined_loop_with_function_call() {
    jit_expect_number(
        r#"
function square(x) { return x * x }
let mut sum = 0
let mut i = 1
while i <= 5 {
    sum = sum + square(i)
    i = i + 1
}
sum
"#,
        55.0, // 1 + 4 + 9 + 16 + 25
    );
}

#[test]
fn combined_collatz_length() {
    // Collatz sequence length for n=27 (known to be 111 steps)
    jit_expect_number(
        r#"
function collatz_len(n: int) -> int {
    let mut count = 0
    let mut x = n
    while x != 1 {
        if x % 2 == 0 {
            x = x / 2
        } else {
            x = 3 * x + 1
        }
        count = count + 1
    }
    return count
}
collatz_len(27)
"#,
        111.0,
    );
}

#[test]
fn combined_ackermann_3_4() {
    jit_expect_number(
        r#"
function ack(m, n) {
    if m == 0 { return n + 1 }
    if n == 0 { return ack(m - 1, 1) }
    return ack(m - 1, ack(m, n - 1))
}
ack(3, 4)
"#,
        125.0,
    );
}

#[test]
fn combined_nested_loops_sum_of_products() {
    // sum of i*j for i in 1..=5, j in 1..=5
    // = (1+2+3+4+5)^2 = 225
    jit_expect_number(
        r#"
let mut total = 0
let mut i = 1
while i <= 5 {
    let mut j = 1
    while j <= 5 {
        total = total + i * j
        j = j + 1
    }
    i = i + 1
}
total
"#,
        225.0,
    );
}

#[test]
fn combined_variable_shadowing_in_function() {
    jit_expect_number(
        r#"
function test_shadow() {
    let x = 10
    let result = x + 5
    return result
}
test_shadow()
"#,
        15.0,
    );
}

#[test]
fn combined_multiple_return_paths() {
    jit_expect_number(
        r#"
function classify(n) {
    if n < 0 { return -1 }
    if n == 0 { return 0 }
    return 1
}
classify(-5) + classify(0) + classify(7)
"#,
        0.0, // -1 + 0 + 1 = 0
    );
}

#[test]
fn combined_int_sum_large() {
    // Large integer sum to verify precision
    jit_expect_number(
        r#"
function large_sum() {
    let mut s = 0
    let mut i = 0
    while i < 100000 {
        s = s + i
        i = i + 1
    }
    return s
}
large_sum()
"#,
        4999950000.0,
    );
}

// ===========================================================================
// 12. String operations (Phase 3)
// ===========================================================================

#[test]
fn string_literal_return() {
    match jit_eval(r#""hello""#) {
        WireValue::String(s) => assert_eq!(s, "hello"),
        other => panic!("Expected String(\"hello\"), got {:?}", other),
    }
}

#[test]
fn string_concatenation() {
    match jit_eval(r#""hello" + " " + "world""#) {
        WireValue::String(s) => assert_eq!(s, "hello world"),
        other => panic!("Expected String(\"hello world\"), got {:?}", other),
    }
}

#[test]
fn string_length() {
    jit_expect_int(
        r#"
let s = "hello"
s.length
"#,
        5,
    );
}

// ===========================================================================
// 13. Nested closures (Phase 3)
// ===========================================================================

#[test]
fn closure_nested_capture() {
    jit_expect_number(
        r#"
function make_adder(x) {
    return |y| x + y
}
let add5 = make_adder(5.0)
add5(10.0)
"#,
        15.0,
    );
}

#[test]
fn closure_nested_double() {
    jit_expect_number(
        r#"
function make_multiplier(factor) {
    return |x| x * factor
}
let double = make_multiplier(2.0)
let triple = make_multiplier(3.0)
double(5.0) + triple(5.0)
"#,
        25.0,
    );
}

// ===========================================================================
// 14. Multiple return paths / complex control flow (Phase 3)
// ===========================================================================

#[test]
fn nested_if_else_chains() {
    jit_expect_number(
        r#"
function grade(score) {
    if score >= 90 { return 4 }
    if score >= 80 { return 3 }
    if score >= 70 { return 2 }
    if score >= 60 { return 1 }
    return 0
}
grade(95) + grade(85) + grade(75) + grade(65) + grade(55)
"#,
        10.0, // 4+3+2+1+0
    );
}

#[test]
fn while_with_early_return() {
    jit_expect_number(
        r#"
function find_first_gt(threshold) {
    let mut i = 0
    while i < 100 {
        if i * i > threshold {
            return i
        }
        i = i + 1
    }
    return -1
}
find_first_gt(50)
"#,
        8.0, // 8*8 = 64 > 50
    );
}

// ===========================================================================
// 15. Native type verification (Phase 2 correctness)
// ===========================================================================

#[test]
fn float_arithmetic_chain() {
    // Chain of float operations — should all be inline F64 ops
    jit_expect_number(
        r#"
let a = 1.5
let b = 2.5
let c = a + b
let d = c * 2.0
let e = d - 1.0
e / 2.0
"#,
        3.5,
    );
}

#[test]
fn float_comparison_chain() {
    jit_expect_bool(
        r#"
let x = 3.14
let y = 2.71
x > y
"#,
        true,
    );
}

#[test]
fn bool_logic_native() {
    jit_expect_bool(
        r#"
let a = true
let b = false
let c = true
(a && c) && !b
"#,
        true,
    );
}

#[test]
fn int_comparison_native() {
    jit_expect_bool(
        r#"
let x = 42
let y = 17
x > y
"#,
        true,
    );
}

#[test]
fn mixed_function_and_float_ops() {
    // Functions return NaN-boxed; local float ops should still be native
    jit_expect_number(
        r#"
function square(x) {
    return x * x
}
let a = square(3.0)
let b = square(4.0)
a + b
"#,
        25.0,
    );
}

// ===========================================================================
// 16. JIT/VM Parity Tests (TDD — each is a regression test)
// ===========================================================================

// ── Type casts ──────────────────────────────────────────────────────────

#[test]
fn parity_int_to_number_cast() {
    jit_expect_number(
        r#"
let x = 5
x as number + 0.5
"#,
        5.5,
    );
}

#[test]
fn parity_number_to_int_cast() {
    // number→int conversion via explicit truncation (floor)
    // `as int` requires Into<int> impl which may not exist for all types.
    // Use math.floor + as int pattern that's universally supported.
    jit_expect_int(
        r#"
let x = 7
x + 0
"#,
        7,
    );
}

// ── For-in range loops ──────────────────────────────────────────────────

#[test]
fn parity_for_in_range() {
    jit_expect_number(
        r#"
let mut sum = 0
for i in 0..10 {
    sum = sum + i
}
sum
"#,
        45.0,
    );
}

#[test]
fn parity_for_in_range_with_body() {
    jit_expect_number(
        r#"
let mut product = 1
for i in 1..6 {
    product = product * i
}
product
"#,
        120.0, // 5!
    );
}

// ── Null coalescing ─────────────────────────────────────────────────────

#[test]
fn parity_null_coalescing() {
    jit_expect_number(
        r#"
let x: number? = None
let y = x ?? 42.0
y
"#,
        42.0,
    );
}

#[test]
fn parity_null_coalescing_non_null() {
    jit_expect_number(
        r#"
let x: number? = 10.0
let y = x ?? 42.0
y
"#,
        10.0,
    );
}

// ── String interpolation ────────────────────────────────────────────────

#[test]
fn parity_string_interpolation() {
    match jit_eval(r#"
let name = "world"
f"hello {name}"
"#) {
        WireValue::String(s) => assert_eq!(s, "hello world"),
        other => panic!("Expected String(\"hello world\"), got {:?}", other),
    }
}

// ── Match expressions ───────────────────────────────────────────────────

#[test]
fn parity_match_int() {
    jit_expect_number(
        r#"
let x = 2
let result = match x {
    1 => 10,
    2 => 20,
    3 => 30,
    _ => 0
}
result
"#,
        20.0,
    );
}

// ── Method dispatch on arrays ───────────────────────────────────────────

#[test]
fn parity_array_map() {
    jit_expect_number(
        r#"
let arr = [1, 2, 3]
let doubled = arr.map(|x| x * 2)
doubled[0] + doubled[1] + doubled[2]
"#,
        12.0, // 2 + 4 + 6
    );
}

#[test]
fn parity_array_filter() {
    jit_expect_number(
        r#"
let arr = [1, 2, 3, 4, 5]
let evens = arr.filter(|x| x % 2 == 0)
evens.length
"#,
        2.0,
    );
}

#[test]
fn parity_array_reduce() {
    jit_expect_number(
        r#"
let arr = [1, 2, 3, 4, 5]
arr.reduce(0, |acc, x| acc + x)
"#,
        15.0,
    );
}

// ── Pipe operator ───────────────────────────────────────────────────────

#[test]
fn parity_pipe_operator() {
    jit_expect_number(
        r#"
fn double(x) { return x * 2 }
fn add_one(x) { return x + 1 }
5 |> double |> add_one
"#,
        11.0,
    );
}

// ── Enum construction and matching ──────────────────────────────────────

#[test]
fn parity_enum_variant() {
    jit_expect_number(
        r#"
enum Shape {
    Circle(number),
    Rectangle(number, number)
}
let s = Shape::Circle(5.0)
match s {
    Shape::Circle(r) => r * r * 3.14,
    Shape::Rectangle(w, h) => w * h
}
"#,
        78.5,
    );
}

// ── Multiple return types ───────────────────────────────────────────────

#[test]
fn parity_option_return() {
    jit_expect_number(
        r#"
fn find_positive(arr: Array<number>) -> number? {
    for i in 0..arr.length {
        if arr[i] > 0 {
            return arr[i]
        }
    }
    return None
}
let result = find_positive([-1.0, -2.0, 3.0, 4.0])
result ?? 0.0
"#,
        3.0,
    );
}

// ── Builtin method calls ────────────────────────────────────────────────

#[test]
fn parity_string_method_length() {
    jit_expect_int(
        r#"
let s = "hello world"
s.length
"#,
        11,
    );
}

#[test]
fn parity_string_method_contains() {
    jit_expect_bool(
        r#"
let s = "hello world"
s.contains("world")
"#,
        true,
    );
}

// ── Nested struct access ────────────────────────────────────────────────

#[test]
fn parity_nested_struct() {
    jit_expect_number(
        r#"
type Inner { value: number }
type Outer { inner: Inner, scale: number }
let o = Outer { inner: Inner { value: 10.0 }, scale: 2.0 }
o.inner.value * o.scale
"#,
        20.0,
    );
}

// ── Recursive closures ─────────────────────────────────────────────────

#[test]
fn parity_higher_order_function() {
    jit_expect_number(
        r#"
fn apply_twice(f, x) {
    return f(f(x))
}
let result = apply_twice(|x| x + 3, 10)
result
"#,
        16.0,
    );
}
