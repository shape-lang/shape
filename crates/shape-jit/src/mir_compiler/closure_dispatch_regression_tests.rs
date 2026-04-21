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
