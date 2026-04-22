//! End-to-end closure dispatch regression tests.
//!
//! These tests pin the fix-jit-lead commits 1–3:
//!
//!  1. `arg_count` ABI decode as raw i64 at the JIT→FFI boundary.
//!  2. Backward propagation of slot kinds onto closure params.
//!  3. `HeapValue::ClosureRaw` / `HeapValue::Closure` decode in
//!     `jit_call_value` via `VmClosureHandle`.
//!
//! Before these commits, the bytecode-emitted closure at `closure_simple`
//! dispatched through `jit_call_value` with `arg_count` misdecoded as 0
//! (via `unbox_number` on a raw i64), the closure body failed to JIT
//! because the `|x|` param had slot kind `Unknown`, and the VM-format
//! closure pointer was unrecognised — so the whole pipeline returned
//! `Null` instead of `Integer(6)`.
//!
//! This module is intentionally NOT gated behind
//! `#[cfg(jit_v2_unstable_tests)]` so the primary regression gate
//! stays green on the default CI path. The broader
//! `mir_compiler::integration_tests` module remains gated because it
//! covers paths with separate pre-existing JIT/VM interaction
//! regressions (see the fix-jit-lead report for the outstanding
//! cell-identity issue that blocks the A.1D.2 counter tests).

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

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

fn jit_expect_number(source: &str, expected: f64) {
    match jit_eval(source) {
        WireValue::Number(n) => assert!(
            (n - expected).abs() < 1e-9,
            "Expected number {}, got {}",
            expected,
            n
        ),
        WireValue::Integer(n) => assert!(
            (n as f64 - expected).abs() < 1e-9,
            "Expected number {} (got Integer {})",
            expected,
            n
        ),
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

/// Primary fix-jit regression gate.
///
/// `|x| x + 1` applied to `5` must return `Integer(6)`. The fix-jit
/// series (commits #1–#3) is specifically motivated by this failing on
/// `jit-v2-phase1`.
#[test]
fn closure_simple_dispatch_returns_six() {
    jit_expect_int(
        r#"
let add_one = |x| x + 1
add_one(5)
"#,
        6,
    );
}

/// Integer-literal-on-rhs variant. Exercises the backward slot-kind
/// propagation from the typed constant onto the closure parameter from
/// the other side of the binop.
#[test]
fn closure_int_literal_on_rhs_propagates_param_kind() {
    jit_expect_int(
        r#"
let times_two = |x| x * 2
times_two(7)
"#,
        14,
    );
}

/// Trampoline dispatch regression: a closure that fails JIT compilation
/// (so its function_table slot stays null) must still produce the
/// correct result when invoked from JIT'd code.
///
/// Before the fix:
///   1. `execute_with_jit` never called `set_trampoline_vm`, so the
///      thread-local `TRAMPOLINE_VM` was null and
///      `dispatch_call_via_trampoline_vm` short-circuited to `TAG_NULL`.
///   2. Even with the VM wired, the trampoline constructed a bare
///      `ValueWord::from_function(fid)` — discarding the closure's
///      captures and producing `Null` / wrong values on return.
///
/// This test lowers a closure whose body takes the JIT's current
/// dynamic-arithmetic bail path (forcing a null function_table slot)
/// while still needing captures to produce the right answer. The
/// pre-fix code returned `Null`; the fix dispatches through
/// `jit_trampoline_call_closure` with the captures threaded through.
/// F4 Option / null-coalescing regression: the bytecode compiler's
/// `FrameDescriptor.slots` seeding of `MirToIR` used incompatible slot
/// numbering. MIR reserves `SlotId(0)` for the implicit return value
/// and numbers parameters starting at 1; the bytecode compiler puts
/// the first parameter at slot 0. Seeding via the bytecode layout
/// mis-declared MIR's return slot with the first param's `SlotKind`,
/// so e.g. a `bool` parameter forced slot 0 (the F64 return) to be
/// declared as `Bool` in Cranelift. Writing `return 7.0` then went
/// through `ensure_kind(F64, Bool) → ireduce I8` which truncated the
/// F64 bit pattern to zero, and `result ?? 42.0` evaluated to 42.0 for
/// every branch of the caller.
///
/// Case 1: function returns `number?`, two `return` paths (literal F64
/// and `None`). Pre-fix: returns 42.0 (None-ness was forced). Post-fix:
/// returns 7.0.
#[test]
fn option_return_conditional_number_some() {
    jit_expect_number(
        r#"
fn get_val(flag: bool) -> number? {
    if flag {
        return 7.0
    }
    return None
}
let x = get_val(true)
x ?? 42.0
"#,
        7.0,
    );
}

/// F4 Option / null-coalescing regression (None branch). Pre-fix the
/// None branch also returned 42.0 — but so did the Some branch, so this
/// passes both before and after the fix. Kept for symmetry with the
/// Some-returning case above so the two sides of the conditional stay
/// pinned together if one regresses in isolation.
#[test]
fn option_return_conditional_number_none() {
    jit_expect_number(
        r#"
fn get_val(flag: bool) -> number? {
    if flag {
        return 7.0
    }
    return None
}
let x = get_val(false)
x ?? 42.0
"#,
        42.0,
    );
}

#[test]
fn closure_non_jit_compiled_dispatches_through_trampoline_vm() {
    // `|| { x = x + base; x }` with `let base` (immutable capture) and
    // `let mut x` (OwnedMutable capture) exercises the exact shape from
    // the original bug report. Calling it twice sums base twice into x.
    jit_expect_int(
        r#"
fn main() -> int {
    let base: int = 10
    let mut x: int = 0
    let f = || { x = x + base; x }
    f()
    f()
}
main()
"#,
        20,
    );
}
