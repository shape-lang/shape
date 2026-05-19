//! JIT-compiled function produces same result as interpreter.
//!
//! Many tests are TDD since the JIT is not directly accessible through
//! the ShapeTest builder. We verify correctness by running code through
//! the interpreter and trusting the JIT must match.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic arithmetic correctness
// =========================================================================

#[test]
fn jit_addition_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(17, 25)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_subtraction_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn sub(a, b) { a - b }
        sub(100, 58)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_multiplication_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn mul(a, b) { a * b }
        mul(6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_division_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn div(a, b) { a / b }
        div(84.0, 2.0)
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Function calls
// =========================================================================

#[test]
fn jit_nested_function_calls() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        fn quad(x) { double(double(x)) }
        quad(10)
    "#,
    )
    .expect_number(40.0);
}

#[test]
fn jit_recursive_function() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        factorial(6)
    "#,
    )
    .expect_number(720.0);
}

// =========================================================================
// Comparison and branching
// =========================================================================

#[test]
fn jit_conditional_branch() {
    ShapeTest::new(
        r#"
        fn max_val(a, b) {
            if a > b { a } else { b }
        }
        max_val(10, 20)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn jit_loop_accumulator() {
    ShapeTest::new(
        r#"
        fn sum_to(n) {
            let mut total = 0
            for i in range(1, n + 1) {
                total = total + i
            }
            total
        }
        sum_to(10)
    "#,
    )
    .expect_number(55.0);
}

// =========================================================================
// Phase 4b Round 5 W14.2-E1 — Trait-method arity matrix (VM regression pins)
//
// Per `docs/cluster-audits/v0.3-w14-test-coverage-audit.md` §4 W10:
// > W10 JIT call-method user-trait | (b) PARTIAL | ... missing:
// > per-trait-method-arity (n=0..6+) coverage matrix.
//
// `ShapeTest` evaluates via the bytecode VM (per `eval_with_output` at
// tools/shape-test/src/shape_test.rs:212 — uses BytecodeExecutor). These
// pins document the VM-side correct behavior across the arity matrix.
// JIT-side byte-equal coverage is in
// crates/shape-jit/src/mir_compiler/closure_dispatch_regression_tests.rs
// (deep-tests-gated due to stdlib JIT-cache parallel-execution SIGILL —
// CLAUDE.md "Known Constraints"). Trait-method dispatch with non-bool
// non-self args at JIT level is currently DIVERGENT — surfaced as
// W14.2-E-SURFACE-A in the W14.2-E close report.
// =========================================================================

#[test]
fn vm_trait_method_arity_n0_int() {
    ShapeTest::new(
        r#"
        trait Greet {
            fn say() -> int
        }
        type Hi {}
        impl Greet for Hi {
            fn say() -> int {
                42
            }
        }
        let h = Hi {}
        h.say()
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n1_int() {
    ShapeTest::new(
        r#"
        trait Doubler {
            fn dbl(x: int) -> int
        }
        type D {}
        impl Doubler for D {
            fn dbl(x: int) -> int {
                x * 2
            }
        }
        let d = D {}
        d.dbl(21)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n2_int() {
    ShapeTest::new(
        r#"
        trait Adder {
            fn add(a: int, b: int) -> int
        }
        type A {}
        impl Adder for A {
            fn add(a: int, b: int) -> int {
                a + b
            }
        }
        let a = A {}
        a.add(20, 22)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n3_int() {
    ShapeTest::new(
        r#"
        trait Tri {
            fn sum3(a: int, b: int, c: int) -> int
        }
        type T3 {}
        impl Tri for T3 {
            fn sum3(a: int, b: int, c: int) -> int {
                a + b + c
            }
        }
        let t = T3 {}
        t.sum3(10, 15, 17)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n4_int() {
    ShapeTest::new(
        r#"
        trait Quad {
            fn sum4(a: int, b: int, c: int, d: int) -> int
        }
        type Q4 {}
        impl Quad for Q4 {
            fn sum4(a: int, b: int, c: int, d: int) -> int {
                a + b + c + d
            }
        }
        let q = Q4 {}
        q.sum4(1, 2, 3, 36)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n5_int() {
    ShapeTest::new(
        r#"
        trait Pent {
            fn sum5(a: int, b: int, c: int, d: int, e: int) -> int
        }
        type P5 {}
        impl Pent for P5 {
            fn sum5(a: int, b: int, c: int, d: int, e: int) -> int {
                a + b + c + d + e
            }
        }
        let p = P5 {}
        p.sum5(1, 2, 3, 4, 32)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n6_int() {
    ShapeTest::new(
        r#"
        trait Hex {
            fn sum6(a: int, b: int, c: int, d: int, e: int, f: int) -> int
        }
        type H6 {}
        impl Hex for H6 {
            fn sum6(a: int, b: int, c: int, d: int, e: int, f: int) -> int {
                a + b + c + d + e + f
            }
        }
        let h = H6 {}
        h.sum6(1, 2, 3, 4, 5, 27)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n7_int_plus() {
    // n=6+ shape — 7-arg method dispatch (above the typical Rust 6-arg
    // calling-convention reg pressure boundary). VM should handle these
    // via stack-passed args.
    ShapeTest::new(
        r#"
        trait Sept {
            fn sum7(a: int, b: int, c: int, d: int, e: int, f: int, g: int) -> int
        }
        type S7 {}
        impl Sept for S7 {
            fn sum7(a: int, b: int, c: int, d: int, e: int, f: int, g: int) -> int {
                a + b + c + d + e + f + g
            }
        }
        let s = S7 {}
        s.sum7(1, 2, 3, 4, 5, 6, 21)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n1_number() {
    ShapeTest::new(
        r#"
        trait NumTrait {
            fn nfn(x: number) -> number
        }
        type N {}
        impl NumTrait for N {
            fn nfn(x: number) -> number {
                x * 2.0
            }
        }
        let n = N {}
        n.nfn(21.0)
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_arity_n1_bool() {
    ShapeTest::new(
        r#"
        trait BoolTrait {
            fn bfn(x: bool) -> bool
        }
        type B {}
        impl BoolTrait for B {
            fn bfn(x: bool) -> bool {
                !x
            }
        }
        let b = B {}
        b.bfn(false)
        "#,
    )
    .expect_bool(true);
}

#[test]
fn vm_trait_method_arity_n1_string() {
    ShapeTest::new(
        r#"
        trait StrTrait {
            fn sfn(x: string) -> string
        }
        type S {}
        impl StrTrait for S {
            fn sfn(x: string) -> string {
                x + "!"
            }
        }
        let s = S {}
        s.sfn("hi")
        "#,
    )
    .expect_string("hi!");
}

#[test]
fn vm_trait_method_self_field_access_n0() {
    // Receiver with field; method body reads `self.value`. JIT diverges
    // here even at n=0 (W14.2-E-SURFACE-A2) — VM is correct.
    ShapeTest::new(
        r#"
        trait Operate {
            fn op() -> int
        }
        type Wrapper { value: int }
        impl Operate for Wrapper {
            fn op() -> int {
                self.value * 2
            }
        }
        let w = Wrapper { value: 21 }
        w.op()
        "#,
    )
    .expect_number(42.0);
}

#[test]
fn vm_trait_method_self_and_arg_n1() {
    // Receiver with field; method body reads `self.value` AND takes an arg.
    // Combined receiver+arg shape exercises both classes simultaneously.
    ShapeTest::new(
        r#"
        trait Adder {
            fn add(other: int) -> int
        }
        type Wrapper { value: int }
        impl Adder for Wrapper {
            fn add(other: int) -> int {
                self.value + other
            }
        }
        let w = Wrapper { value: 10 }
        w.add(32)
        "#,
    )
    .expect_number(42.0);
}
