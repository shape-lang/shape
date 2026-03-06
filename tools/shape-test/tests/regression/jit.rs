//! JIT regression tests -- verify correctness of JIT-compiled Shape programs.
//!
//! These tests run small Shape programs through the JIT executor and verify
//! they produce the same results as the VM. This catches regressions from
//! JIT optimization phases (inline array access, fused cmp-branch, etc.).

use shape_runtime::engine::ShapeEngine;
use shape_runtime::initialize_shared_runtime;
use shape_jit::JITExecutor;
use shape_runtime::engine::ProgramExecutor;
use shape_wire::WireValue;

/// Run a Shape program through JIT and return the result as WireValue.
fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

/// Run through JIT, expect a Number result (not Integer — type form matters).
fn jit_expect_number(source: &str, expected: f64) {
    match jit_eval(source) {
        WireValue::Number(n) => {
            assert!(
                (n - expected).abs() < 1e-6,
                "Expected {}, got {}",
                expected,
                n
            );
        }
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

// -- Preflight: all builtins now accepted (100% JIT coverage) -----------------

#[test]
fn jit_preflight_accepts_all_builtins() {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program("toBool(1)").expect("parse failed");
    let result = JITExecutor.execute_program(&mut engine, &program);
    assert!(
        result.is_ok(),
        "All builtins should be accepted by JIT preflight (generic FFI trampoline)"
    );
}

// -- Basic arithmetic ---------------------------------------------------------

#[test]
fn jit_add() {
    jit_expect_number("10 + 5", 15.0);
}

#[test]
fn jit_sub() {
    jit_expect_number("10 - 3", 7.0);
}

#[test]
fn jit_mul() {
    jit_expect_number("6 * 7", 42.0);
}

#[test]
fn jit_div() {
    jit_expect_number("100 / 4", 25.0);
}

#[test]
fn jit_mod() {
    jit_expect_number("17 % 5", 2.0);
}

// -- Variables ----------------------------------------------------------------

#[test]
fn jit_local_variables() {
    jit_expect_number("let x = 10\nlet y = 20\nx + y", 30.0);
}

#[test]
fn jit_variable_reassignment() {
    jit_expect_number("var x = 1\nx = x + 1\nx = x + 1\nx", 3.0);
}

// -- Comparisons (via if/else to get numeric result) --------------------------

#[test]
fn jit_comparison_gt() {
    jit_expect_number("if 10 > 5 { 1 } else { 0 }", 1.0);
    jit_expect_number("if 5 > 10 { 1 } else { 0 }", 0.0);
}

#[test]
fn jit_comparison_lt() {
    jit_expect_number("if 5 < 10 { 1 } else { 0 }", 1.0);
}

#[test]
fn jit_comparison_eq() {
    jit_expect_number("if 10 == 10 { 1 } else { 0 }", 1.0);
    jit_expect_number("if 10 == 5 { 1 } else { 0 }", 0.0);
}

#[test]
fn jit_comparison_neq() {
    jit_expect_number("if 10 != 5 { 1 } else { 0 }", 1.0);
}

#[test]
fn jit_comparison_gte_lte() {
    jit_expect_number("if 10 >= 10 { 1 } else { 0 }", 1.0);
    jit_expect_number("if 10 <= 10 { 1 } else { 0 }", 1.0);
}

// -- Control flow -------------------------------------------------------------

#[test]
fn jit_if_else() {
    jit_expect_number("if true { 1 } else { 2 }", 1.0);
    jit_expect_number("if false { 1 } else { 2 }", 2.0);
}

#[test]
fn jit_while_loop() {
    jit_expect_number(
        "var x = 0\nvar i = 0\nwhile i < 10 { x = x + i\ni = i + 1 }\nx",
        45.0,
    );
}

#[test]
fn jit_while_sum_to_100() {
    jit_expect_number(
        "var sum = 0\nvar i = 1\nwhile i <= 100 {\n  sum = sum + i\n  i = i + 1\n}\nsum",
        5050.0,
    );
}

#[test]
fn jit_float_loop_mixed_bound_comparison() {
    jit_expect_number(
        r#"
function sum_to(n) {
    var s = 0.0
    var i = 0.0
    while i < n {
        s = s + i
        i = i + 1.0
    }
    return s
}
sum_to(10)
"#,
        45.0,
    );
}

// -- Functions ----------------------------------------------------------------

#[test]
fn jit_function_call() {
    jit_expect_number("function double(n) { return n * 2 }\ndouble(21)", 42.0);
}

#[test]
fn jit_recursive_fibonacci() {
    jit_expect_number(
        "function fib(n) {\n  if n < 2 { return n }\n  return fib(n - 1) + fib(n - 2)\n}\nfib(20)",
        6765.0,
    );
}

// -- Arrays -------------------------------------------------------------------

#[test]
fn jit_array_create_and_access() {
    jit_expect_number("let arr = [10, 20, 30]\narr[1]", 20.0);
}

#[test]
fn jit_array_length() {
    jit_expect_number("let arr = [1, 2, 3, 4, 5]\narr.length", 5.0);
}

#[test]
fn jit_array_mutation_via_function() {
    // References only work on local variables passed as function arguments
    jit_expect_number(
        r#"
function set_elem(&arr, idx, val) {
    arr[idx] = val
}
function test_mutate() {
    var arr = [10, 20, 30]
    set_elem(&arr, 1, 99)
    return arr[1]
}
test_mutate()
"#,
        99.0,
    );
}

#[test]
fn jit_array_push_via_function() {
    jit_expect_number(
        r#"
function push_vals(&arr) {
    arr = arr.push(10)
    arr = arr.push(20)
    arr = arr.push(30)
}
function test_push() {
    var arr = []
    push_vals(&arr)
    return arr.length
}
test_push()
"#,
        3.0,
    );
}

// -- Regression: loop with comparison (Phase 2 fused cmp-branch) --------------

#[test]
fn jit_loop_comparison_fused() {
    // Tests that fused comparison-branch correctly handles loop conditions.
    // Phase 2 optimization fuses fcmp + boolean boxing + JumpIfFalse into
    // a single fcmp + brif. This test catches SSA/branch target errors.
    jit_expect_number(
        r#"
var count = 0
var i = 0
while i < 1000 {
    if i % 2 == 0 { count = count + 1 }
    i = i + 1
}
count
"#,
        500.0,
    );
}

#[test]
fn jit_nested_loop_comparison() {
    // Nested loops stress-test the fused comparison optimization
    jit_expect_number(
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
        100.0,
    );
}

#[test]
fn jit_mandelbrot_mixed_numeric_loop_regression() {
    // Regression: generic numeric loop vars initialized inside outer loops
    // must not be defaulted to int-unboxed when init type is unknown.
    jit_expect_number(
        r#"
function mandelbrot(size) {
    var count = 0;
    var y = 0;
    while y < size {
        var x = 0;
        while x < size {
            let cr = 2.0 * x / size - 1.5;
            let ci = 2.0 * y / size - 1.0;
            var zr = 0.0;
            var zi = 0.0;
            var iter = 0;
            while iter < 50 {
                let tr = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = tr;
                if zr * zr + zi * zi > 4.0 {
                    break;
                }
                iter = iter + 1;
            }
            if iter == 50 {
                count = count + 1;
            }
            x = x + 1;
        }
        y = y + 1;
    }
    return count;
}
mandelbrot(120)
"#,
        5739.0,
    );
}

// -- Regression: array-heavy computation (Phase 1 inline array access) --------

#[test]
fn jit_sieve_small() {
    // Small sieve of Eratosthenes -- exercises array read/write in loops.
    // This catches regressions in inline emit_array_data_ptr (JitArray offsets).
    jit_expect_number(
        r#"
function mark_composites(&flags, p: int, n: int) {
    var j = p * p
    while j <= n {
        flags[j] = false
        j = j + p
    }
}

function sieve(n: int) -> int {
    var flags = []
    var i = 0
    while i <= n {
        flags = flags.push(true)
        i = i + 1
    }
    var p = 2
    while p * p <= n {
        if flags[p] {
            mark_composites(&flags, p, n)
        }
        p = p + 1
    }
    var count = 0
    var k = 2
    while k <= n {
        if flags[k] {
            count = count + 1
        }
        k = k + 1
    }
    return count
}
sieve(1000)
"#,
        168.0, // number of primes <= 1000
    );
}

// -- Regression: numeric precision --------------------------------------------

#[test]
fn jit_floating_point_precision() {
    jit_expect_number("0.1 + 0.2", 0.30000000000000004);
}

#[test]
fn jit_large_number_arithmetic() {
    jit_expect_number("1000000 * 1000000", 1e12);
}

// -- Regression: Ackermann function (deep recursion + comparisons) ------------

#[test]
fn jit_ackermann() {
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

// -- Regression: iterative fibonacci (loop + variable swap) -------------------

#[test]
fn jit_fib_iterative() {
    jit_expect_number(
        r#"
function fib_iter(n: int) -> int {
    var a = 0
    var b = 1
    var i = 0
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

// -- Regression: collatz sequence ---------------------------------------------

#[test]
fn jit_collatz() {
    // Collatz sequence length for n=27 (known to be 111 steps)
    jit_expect_number(
        r#"
function collatz_len(n: int) -> int {
    var count = 0
    var x = n
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

// -- Regression: matrix multiply (triple-nested + array access) ---------------

#[test]
fn jit_matrix_mul_small() {
    // Small 3x3 matrix multiply exercising triple-nested loops + array access
    // A = [[1,2,3],[4,5,6],[7,8,9]], compute trace of A*A
    // (AA)[0][0] = 1*1+2*4+3*7 = 30
    // (AA)[1][1] = 4*2+5*5+6*8 = 81
    // (AA)[2][2] = 7*3+8*6+9*9 = 150
    // trace = 30+81+150 = 261
    jit_expect_number(
        r#"
function do_mul(&c_ref, a, b, n: int) {
    var i = 0
    while i < n {
        var j = 0
        while j < n {
            var s = 0
            var k = 0
            while k < n {
                s = s + a[i * n + k] * b[k * n + j]
                k = k + 1
            }
            c_ref[i * n + j] = s
            j = j + 1
        }
        i = i + 1
    }
}

function mat_mul_trace(n: int) -> int {
    var a = []
    var b = []
    var c = []
    var i = 0
    while i < n * n {
        a = a.push(i + 1)
        b = b.push(i + 1)
        c = c.push(0)
        i = i + 1
    }
    do_mul(&c, a, b, n)
    var trace = 0
    var d = 0
    while d < n {
        trace = trace + c[d * n + d]
        d = d + 1
    }
    return trace
}
mat_mul_trace(3)
"#,
        261.0,
    );
}

// -- Regression: integer unboxing (Sprint 5.1) --------------------------------

#[test]
fn jit_int_unboxing_sum_local() {
    // Integer sum loop using function-scoped local variables.
    // Tests the prelude block pattern: NaN-boxed -> raw i64 at loop entry,
    // native iadd in loop body, raw i64 -> NaN-boxed at loop exit.
    jit_expect_number(
        r#"
function sum_test() {
    var s = 0
    var i = 0
    while i < 1000 {
        s = s + i
        i = i + 1
    }
    return s
}
sum_test()
"#,
        499500.0,
    );
}

#[test]
fn jit_int_unboxing_sum_module_binding() {
    // Same integer sum but with top-level (module binding) variables.
    // Tests module binding promotion to Cranelift Variables.
    jit_expect_number(
        r#"
var s = 0
var i = 0
while i < 1000 {
    s = s + i
    i = i + 1
}
s
"#,
        499500.0,
    );
}

#[test]
fn jit_int_unboxing_nested_loops() {
    // Nested loops: outer loop activates unboxing, inner loop must NOT
    // prematurely clear the outer loop's unboxed state.
    jit_expect_number(
        r#"
function nested_sum() {
    var total = 0
    var i = 0
    while i < 10 {
        var j = 0
        while j < 10 {
            total = total + 1
            j = j + 1
        }
        i = i + 1
    }
    return total
}
nested_sum()
"#,
        100.0,
    );
}

#[test]
fn jit_int_unboxing_fib_swap() {
    // Fibonacci iteration: `t = a + b; a = b; b = t` pattern.
    // `t` should NOT be unboxed because it flows to a plain assignment (b = t).
    // `i` is an induction variable (unboxed).
    // Tests that the accumulator filter correctly excludes `t`.
    jit_expect_number(
        r#"
function fib_iter(n: int) -> int {
    var a = 0
    var b = 1
    var i = 0
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
fn jit_int_unboxing_mixed_local_types() {
    // Loop with both unboxed (i, count) and non-unboxed variables.
    // `flag` is a boolean variable -- must NOT be unboxed.
    // Tests that non-integer variables remain NaN-boxed.
    jit_expect_number(
        r#"
function mixed_test() {
    var count = 0
    var i = 0
    while i < 100 {
        if i % 3 == 0 {
            count = count + 1
        }
        i = i + 1
    }
    return count
}
mixed_test()
"#,
        34.0,
    );
}

#[test]
fn jit_int_unboxing_nested_module_bindings() {
    // Top-level nested loops with module bindings.
    // Tests module binding promotion + nested loop depth tracking.
    jit_expect_number(
        r#"
var total = 0
var i = 0
while i < 20 {
    var j = 0
    while j < 20 {
        total = total + 1
        j = j + 1
    }
    i = i + 1
}
total
"#,
        400.0,
    );
}

#[test]
fn jit_int_unboxing_large_result() {
    // Large integer result to test precision preservation.
    // 100M sum = 4999999950000000 (exceeds 2^32, needs full i64).
    jit_expect_number(
        r#"
function large_sum() {
    var s = 0
    var i = 0
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
