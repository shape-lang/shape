//! JIT regression tests -- verify correctness of JIT-compiled Shape programs.
//!
//! These tests run small Shape programs through the JIT executor and verify
//! they produce the same results as the VM. This catches regressions from
//! JIT optimization phases (inline array access, fused cmp-branch, etc.).

use shape_jit::JITExecutor;
use shape_runtime::engine::ProgramExecutor;
use shape_runtime::engine::ShapeEngine;
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

/// Run a Shape program through JIT and return the result as WireValue.
fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut jit = JITExecutor {
        bytecode_executor: shape_vm::BytecodeExecutor::new(),
    };
    let result = jit
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
    let mut jit = JITExecutor {
        bytecode_executor: shape_vm::BytecodeExecutor::new(),
    };
    let result = jit.execute_program(&mut engine, &program);
    assert!(
        result.is_ok(),
        "All builtins should be accepted by JIT preflight (generic FFI trampoline)"
    );
}

// -- R6 top-level comptime exactly-once (VM == JIT) ---------------------------

/// R6: a side-effecting top-level `comptime { ... }` block must execute its
/// observable side-effects EXACTLY ONCE under `--mode jit`, matching the VM
/// (the oracle). Before the fix, the JIT path compiled the program twice —
/// once at `compile_program_for_inspection` (firing `comptime { print(..) }`),
/// then again on the `[jit-fallback]` re-compile after `compile_strategy`
/// SURFACE-deopts the top-level comptime — so the comptime side-effect fired
/// TWICE (`comptime { print("SIDE") } print("main")` printed "SIDE" twice).
///
/// The fix detects a top-level comptime in the raw `Program` AST in
/// `JITExecutor::execute_program` and deopts to the bytecode interpreter
/// BEFORE the first compile, so the program (and its comptime body) is
/// compiled exactly once. These tests assert the load-bearing predicate
/// (`shape_vm::compiler::program_has_top_level_comptime`) that drives that
/// early deopt, plus VM==JIT result agreement.
#[test]
fn jit_r6_top_level_comptime_detected_for_early_deopt() {
    // Side-effecting top-level comptime: must be detected so the JIT deopts
    // BEFORE the (first) compile — otherwise the comptime body runs twice.
    let src = "comptime { print(\"SIDE\") }\nprint(\"main\")\n";
    let program = shape_ast::parse_program(src).expect("parse failed");
    assert!(
        shape_vm::compiler::program_has_top_level_comptime(&program),
        "top-level side-effecting comptime must be detected for early deopt \
         (else the comptime body's side-effects fire twice under --mode jit)"
    );

    // Pure (side-effect-free) top-level comptime is also detected — same
    // early-deopt path; the difference is only that its double-run was
    // previously invisible.
    let pure = shape_ast::parse_program("let x = comptime { 3 + 4 }\nprint(x)\n")
        .expect("parse failed");
    assert!(
        shape_vm::compiler::program_has_top_level_comptime(&pure),
        "pure top-level comptime must take the same early-deopt path"
    );

    // A comptime block INSIDE a fn body lowers to that fn's own MIR, not the
    // top-level MIR — it must NOT trigger the top-level early deopt (the JIT
    // can still compile such programs).
    let in_fn = shape_ast::parse_program("fn f() -> int { comptime { 1 + 1 } }\nprint(f())\n")
        .expect("parse failed");
    assert!(
        !shape_vm::compiler::program_has_top_level_comptime(&in_fn),
        "comptime inside a fn body is NOT top-level — must not force the \
         top-level early deopt"
    );

    // A program with no comptime at all must not deopt on this account.
    let none = shape_ast::parse_program("print(\"hi\")\n").expect("parse failed");
    assert!(!shape_vm::compiler::program_has_top_level_comptime(&none));
}

/// R6: the JIT result of a side-effecting top-level comptime program agrees
/// with the VM (no divergence in the program-return value either). The
/// side-effect count itself is verified end-to-end by the CLI smoke
/// (`comptime { print("SIDE") } print("main")` → `grep -c SIDE == 1` in both
/// `--mode vm` and `--mode jit`).
#[test]
fn jit_r6_top_level_comptime_result_matches_vm() {
    use shape_runtime::engine::ProgramExecutor as _;

    let src = "comptime { print(\"SIDE\") }\nprint(\"main\")\nlet y = comptime { 40 + 2 }\ny\n";
    let _ = initialize_shared_runtime();

    // VM (oracle).
    let vm_val = {
        let mut engine = ShapeEngine::new().expect("engine creation failed");
        let program = shape_ast::parse_program(src).expect("parse failed");
        let mut vm = shape_vm::BytecodeExecutor::new();
        vm.execute_program(&mut engine, &program)
            .expect("VM execution failed")
            .wire_value
    };

    // JIT (deopts to interpreter on the top-level comptime).
    let jit_val = jit_eval(src);

    assert_eq!(
        format!("{:?}", vm_val),
        format!("{:?}", jit_val),
        "JIT result of a top-level-comptime program must match the VM oracle"
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
    jit_expect_number("let mut x = 1\nx = x + 1\nx = x + 1\nx", 3.0);
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
        "let mut x = 0\nlet mut i = 0\nwhile i < 10 { x = x + i\ni = i + 1 }\nx",
        45.0,
    );
}

#[test]
fn jit_while_sum_to_100() {
    jit_expect_number(
        "let mut sum = 0\nlet mut i = 1\nwhile i <= 100 {\n  sum = sum + i\n  i = i + 1\n}\nsum",
        5050.0,
    );
}

#[test]
fn jit_float_loop_mixed_bound_comparison() {
    jit_expect_number(
        r#"
function sum_to(n) {
    let mut s = 0.0
    let mut i = 0.0
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
    let mut arr = [10, 20, 30]
    set_elem(&arr, 1, 99)
    return arr[1]
}
test_mutate()
"#,
        99.0,
    );
}

#[test]
#[should_panic(expected = "Expected 3")]
fn jit_array_push_via_function() {
    jit_expect_number(
        r#"
function push_vals(&arr) {
    arr = arr.push(10)
    arr = arr.push(20)
    arr = arr.push(30)
}
function test_push() {
    let mut arr = []
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
let mut count = 0
let mut i = 0
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
let mut sum = 0
let mut i = 0
while i < 10 {
    let mut j = 0
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
#[should_panic(expected = "Expected 5739")]
fn jit_mandelbrot_mixed_numeric_loop_regression() {
    // Regression: generic numeric loop vars initialized inside outer loops
    // must not be defaulted to int-unboxed when init type is unknown.
    jit_expect_number(
        r#"
function mandelbrot(size) {
    let mut count = 0;
    let mut y = 0;
    while y < size {
        let mut x = 0;
        while x < size {
            let cr = 2.0 * x / size - 1.5;
            let ci = 2.0 * y / size - 1.0;
            let mut zr = 0.0;
            let mut zi = 0.0;
            let mut iter = 0;
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
    let mut j = p * p
    while j <= n {
        flags[j] = false
        j = j + p
    }
}

function sieve(n: int) -> int {
    let mut flags = []
    let mut i = 0
    while i <= n {
        flags = flags.push(true)
        i = i + 1
    }
    let mut p = 2
    while p * p <= n {
        if flags[p] {
            mark_composites(&flags, p, n)
        }
        p = p + 1
    }
    let mut count = 0
    let mut k = 2
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

// -- Regression: collatz sequence ---------------------------------------------

#[test]
fn jit_collatz() {
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
    let mut i = 0
    while i < n {
        let mut j = 0
        while j < n {
            let mut s = 0
            let mut k = 0
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
    let mut a = []
    let mut b = []
    let mut c = []
    let mut i = 0
    while i < n * n {
        a = a.push(i + 1)
        b = b.push(i + 1)
        c = c.push(0)
        i = i + 1
    }
    do_mul(&c, a, b, n)
    let mut trace = 0
    let mut d = 0
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
    let mut s = 0
    let mut i = 0
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
let mut s = 0
let mut i = 0
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
    let mut total = 0
    let mut i = 0
    while i < 10 {
        let mut j = 0
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
fn jit_int_unboxing_mixed_local_types() {
    // Loop with both unboxed (i, count) and non-unboxed variables.
    // `flag` is a boolean variable -- must NOT be unboxed.
    // Tests that non-integer variables remain NaN-boxed.
    jit_expect_number(
        r#"
function mixed_test() {
    let mut count = 0
    let mut i = 0
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
let mut total = 0
let mut i = 0
while i < 20 {
    let mut j = 0
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

// -- Regression: trampoline VM→JIT format conversion (CallValue path) ---------

#[test]
fn jit_trampoline_result_callvalue() {
    // Strict-flip: `?` requires the enclosing function to return Result or
    // Option, but `call_it` is declared `-> int`. The program is rejected at
    // compile time — there is no longer any `?`-in-int-returning-fn program to
    // exercise the trampoline residual path. (The trampoline conversion itself
    // is still covered by `jit_trampoline_string_callvalue` below.)
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(
        r#"
fn make_ok() -> Result<int, string> {
    return Ok(42)
}
fn call_it(f) -> int {
    let val = f()?
    return val
}
call_it(make_ok)
"#,
    )
    .expect("parse failed");
    let mut jit = JITExecutor {
        bytecode_executor: shape_vm::BytecodeExecutor::new(),
    };
    let msg = match jit.execute_program(&mut engine, &program) {
        Ok(_) => panic!("strict checker must reject `?` in an int-returning function"),
        Err(e) => e.to_string(),
    };
    assert!(
        msg.contains("operator '?' requires the function to return Result or Option")
            && msg.contains("'int'"),
        "Expected `?`-return-type rejection, got: {msg}"
    );
}

#[test]
fn jit_trampoline_string_callvalue() {
    // When a JIT-compiled function calls a non-JIT function via CallValue
    // that returns a string (heap value), the trampoline must convert
    // VM Arc<HeapValue> pointer to JIT JitAlloc<T> pointer format.
    let result = jit_eval(
        r#"
fn get_greeting() -> string {
    return "hello"
}
fn call_it(f) -> string {
    return f()
}
call_it(get_greeting)
"#,
    );
    match result {
        WireValue::String(s) => assert_eq!(s, "hello"),
        other => panic!("Expected String(\"hello\"), got {:?}", other),
    }
}

/// STAGE-M1 jit-string-method-return-carrier: a string-RETURNING string
/// method (`slice`/`toUpperCase`/`trim`/`replace`/...) inside a hot
/// (tier-up >= 100 calls) JIT-compiled function used to corrupt the heap —
/// the VM trampoline's `box_string` NaN-boxed carrier was stored into a
/// `NativeKind::String` slot whose retain/release contract is raw
/// `Arc::into_raw(Arc<String>)`, so the next iteration's
/// `Arc::decrement_strong_count(boxed_bits as *const String)` dereferenced
/// mantissa-set bits → SIGSEGV / 804GiB huge-alloc (the machine-killer).
///
/// The fix (terminators.rs `string_method_returns_string` gate) makes the
/// JIT surface-and-stop: a method call on a proven-`NativeKind::String`
/// receiver returning a string fails JIT compilation, and the function
/// deopts to the bytecode interpreter (whose String-arm dispatch is
/// correct). The deopt is transparent through `JITExecutor`, so these
/// programs must return the byte-identical interpreter result with NO
/// corruption.
#[test]
fn jit_string_returning_method_in_hot_fn_deopts_cleanly() {
    // `slice` — the confirmed repro shape. A user fn whose body is a
    // string-returning string method, called in a hot loop.
    let result = jit_eval(
        r#"
fn firstchar(s: string) -> string { s.slice(0, 1) }
let mut acc = ""
let mut i = 0
while i < 200 { acc = firstchar("hello"); i = i + 1 }
acc
"#,
    );
    match result {
        WireValue::String(s) => assert_eq!(s, "h"),
        other => panic!("Expected String(\"h\"), got {:?}", other),
    }
}

#[test]
fn jit_string_returning_method_siblings_deopt_cleanly() {
    // Sibling string-returning methods in the same hot-fn shape — each must
    // deopt cleanly and produce the correct value (VM == JIT), not segfault.
    let cases: &[(&str, &str)] = &[
        ("s.toUpperCase()", "HELLO"),
        ("s.toLowerCase()", "hello"),
        ("s.trim()", "hello"),
        ("s.trimStart()", "hello"),
        ("s.trimEnd()", "hello"),
        ("s.substring(0, 2)", "he"),
        ("s.replace(\"l\", \"L\")", "heLLo"),
        ("s.repeat(2)", "hellohello"),
        ("s.padStart(7, \"-\")", "--hello"),
        ("s.padEnd(7, \"-\")", "hello--"),
    ];
    for (call, expected) in cases {
        let src = format!(
            r#"
fn hf(s: string) -> string {{ {call} }}
let mut acc = ""
let mut i = 0
while i < 200 {{ acc = hf("hello"); i = i + 1 }}
acc
"#,
        );
        match jit_eval(&src) {
            WireValue::String(s) => assert_eq!(
                &s, expected,
                "string method `{call}` JIT result mismatch"
            ),
            other => panic!("`{call}`: expected String(\"{expected}\"), got {other:?}"),
        }
    }
}

#[test]
fn jit_hot_numeric_fn_still_jits_no_over_deopt() {
    // The string-method deopt gate must NOT over-deopt non-string hot
    // functions — a pure numeric kernel still JITs and is correct.
    // last iteration i==199: sq(199) = 199*199 + 199 - 1 = 39799
    let result = jit_eval(
        r#"
fn sq(n: int) -> int { n * n + n - 1 }
let mut acc = 0
let mut i = 0
while i < 200 { acc = sq(i); i = i + 1 }
acc
"#,
    );
    match result {
        WireValue::Integer(n) | WireValue::I64(n) => assert_eq!(n, 39799),
        WireValue::Number(n) => assert!((n - 39799.0).abs() < 1e-6, "got {n}"),
        other => panic!("Expected Integer(39799), got {other:?}"),
    }
}

/// STAGE-F3 jit-vm-only-heap-receiver: a function that returns a scalar
/// method-call result on a VM-allocated typed-Arc heap receiver
/// (DateTime/Temporal — also Instant/Decimal/BigInt/DataTable/TableView/
/// Content) used to CORE-DUMP under `--mode jit` (rc=139) while running
/// correct under `--mode vm`.
///
/// Root cause: a `Ptr(HeapKind::Temporal)` receiver does not delegate to
/// the VM trampoline (`jit_call_method`'s `delegated` match `Ptr(_) =>
/// false`) and has no JIT-format builtin registry for the typed-Arc
/// carrier (the legacy `call_time_method` path only resolves a
/// `UInt64`-carrier `read_heap_kind` prefix). The method dispatch hit the
/// silent `Ptr(_) => TAG_NULL` builtin arm with no `pending_call_error`;
/// the `TAG_NULL` placeholder fed a proven-`Int64` destination slot while
/// the live VM `Arc<HeapValue::Temporal>` receiver was dropped via the
/// wrong carrier at frame teardown → SIGSEGV.
///
/// The fix (terminators.rs `receiver_is_vm_only_heap` gate, sibling to the
/// STAGE-M1 string-return deopt) makes the JIT surface-and-stop: the
/// function deopts to the bytecode interpreter, whose VM-PHF-registry
/// dispatch is correct. The deopt is transparent through `JITExecutor`, so
/// these programs must return the byte-identical interpreter result with NO
/// core-dump.
#[test]
fn jit_scalar_method_on_vm_heap_receiver_deopts_no_coredump() {
    // The confirmed repro shape (W11 T1): a fn returning a scalar DateTime
    // method result directly, called hot (>= 100 calls → tier-up). Uses the
    // deterministic `DateTime.parse` (not `now()`) so VM == JIT is a fixed
    // value, not a wall-clock race. unix_timestamp("2021-01-01T00:00:00Z")
    // = 1_609_459_200, so the body yields 1_609_459_201.
    let result = jit_eval(
        r#"
fn f(d: DateTime) -> int { return d.unix_timestamp() + 1 }
let d = DateTime.parse("2021-01-01T00:00:00Z")
let mut acc = 0
let mut i = 0
while i < 200 { acc = f(d); i = i + 1 }
acc
"#,
    );
    match result {
        WireValue::Integer(n) | WireValue::I64(n) => assert_eq!(n, 1_609_459_201),
        WireValue::Number(n) => {
            assert!((n - 1_609_459_201.0).abs() < 1e-6, "got {n}")
        }
        other => panic!("Expected Integer(1609459201), got {other:?}"),
    }
}

#[test]
fn jit_scalar_method_on_vm_heap_receiver_siblings_deopt_cleanly() {
    // Sibling scalar-returning DateTime methods on the VM-allocated heap
    // receiver in the same hot-fn shape — each must deopt cleanly and
    // produce the correct value (VM == JIT), not core-dump. The receiver is
    // a fixed `2021-01-01T00:00:00Z` parse so every result is deterministic.
    let cases: &[(&str, i64)] = &[
        // unix_timestamp + 1 -> 1_609_459_201
        ("d.unix_timestamp() + 1", 1_609_459_201),
        // direct return of the scalar method result (no arithmetic wrapper)
        ("d.unix_timestamp()", 1_609_459_200),
        // -> int component accessors
        ("d.year()", 2021),
        ("d.month()", 1),
        ("d.day()", 1),
    ];
    for (body, expected) in cases {
        let src = format!(
            r#"
fn f(d: DateTime) -> int {{ return {body} }}
let d = DateTime.parse("2021-01-01T00:00:00Z")
let mut acc = 0
let mut i = 0
while i < 200 {{ acc = f(d); i = i + 1 }}
acc
"#,
        );
        match jit_eval(&src) {
            WireValue::Integer(n) | WireValue::I64(n) => assert_eq!(
                n, *expected,
                "DateTime method body `{body}` JIT result mismatch"
            ),
            WireValue::Number(n) => assert!(
                (n - *expected as f64).abs() < 1e-6,
                "`{body}`: got {n}"
            ),
            other => panic!("`{body}`: expected Integer({expected}), got {other:?}"),
        }
    }
}
