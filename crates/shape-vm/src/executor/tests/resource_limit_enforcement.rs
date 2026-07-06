//! WF-3B resource-limit enforcement regression tests.
//!
//! Two HIGH-severity sandbox defects on a serve node that runs untrusted
//! transferred code:
//!
//! * **Defect A** — `--max-output-bytes` was inert: `ResourceUsage::record_output`
//!   existed but had zero callers, so untrusted code could `print` without
//!   bound. Fixed by charging the output budget at the `print` sink
//!   (`builtin_print`) and surfacing a clean `OutputLimit` `VMError`, output
//!   truncated at the cap.
//! * **Defect B** — exceeding `--max-memory-bytes` `panic!`ed inside
//!   `TypedArray::grow` (process abort, exit 101) rather than surfacing a
//!   clean error, letting untrusted code DoS the host / serve process. Fixed
//!   by recording the breach on the thread-local alloc budget (no panic),
//!   refusing the growth (buffer bounded at the ceiling), and draining the
//!   breach into a clean `VMError` at the dispatch-loop safepoint.
//!
//! These tests assert the surfaced-error behaviour AND — critically for a
//! serve node — that a second execution on the SAME thread succeeds after a
//! breach (the worker survives; the next request is unaffected).

use crate::VMConfig;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::resource_limits::ResourceLimits;
use shape_value::{KindedSlot, VMError};

/// Compile Shape source and run it under the given resource limits, mirroring
/// the production `ShapeExecutor` path in `execution.rs`: install the
/// `ResourceUsage` (drives the dispatch-loop safepoints) AND the per-execution
/// alloc-budget ceiling (bounds a single growing buffer) for the duration of
/// the run. Returns the raw execution `Result`.
fn run_with_limits(source: &str, limits: ResourceLimits) -> Result<KindedSlot, VMError> {
    let program = shape_ast::parser::parse_program(source).expect("parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler.compile(&program).expect("compile failed");

    // Mirror `execution.rs`: a memory cap installs the per-buffer ceiling.
    let _budget = shape_value::v2::alloc_budget::BudgetGuard::new(limits.max_memory_bytes);

    let mut vm = VirtualMachine::new(VMConfig::default()).with_resource_limits(limits);
    vm.load_program(bytecode);
    vm.execute(None)
}

/// Defect A: printing past `--max-output-bytes` stops at the cap with a clean
/// `OutputLimit` error (never a silent pass, never a panic).
#[test]
fn output_limit_surfaces_clean_error() {
    let src = r#"
        for i in 0..100 {
            print("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        }
    "#;
    let limits = ResourceLimits {
        max_output_bytes: Some(50),
        ..ResourceLimits::unlimited()
    };
    let err = run_with_limits(src, limits).expect_err("output cap must surface an error");
    match err {
        VMError::RuntimeError(msg) => {
            assert!(
                msg.contains("Output limit exceeded"),
                "expected OutputLimit surfacing, got: {msg}"
            );
        }
        other => panic!("expected RuntimeError(OutputLimit), got {other:?}"),
    }
}

/// Defect A companion: output that stays within the cap runs to completion.
#[test]
fn output_within_budget_ok() {
    let src = r#"print("hi")"#;
    let limits = ResourceLimits {
        max_output_bytes: Some(1024),
        ..ResourceLimits::unlimited()
    };
    run_with_limits(src, limits).expect("output within budget must succeed");
}

/// Defect B: allocating past `--max-memory-bytes` surfaces a clean
/// `VMError` — NOT a `panic!` / process abort. If the growth path still
/// panicked, this test would abort the whole test binary instead of failing
/// with an assertion, so a green result IS the "no panic on untrusted input"
/// proof.
#[test]
fn memory_limit_surfaces_clean_error_not_panic() {
    let src = r#"
        let mut a = [0]
        for i in 0..2000000 {
            a.push(i)
        }
        print(a.len())
    "#;
    let limits = ResourceLimits {
        max_memory_bytes: Some(4096),
        ..ResourceLimits::unlimited()
    };
    let err = run_with_limits(src, limits).expect_err("memory cap must surface an error");
    match err {
        VMError::RuntimeError(msg) => {
            assert!(
                msg.contains("memory limit exceeded"),
                "expected MemoryLimit surfacing, got: {msg}"
            );
        }
        other => panic!("expected RuntimeError(MemoryLimit), got {other:?}"),
    }
}

/// Serve-node survival: a worker thread that handles a memory-breaching
/// request must NOT die, and the NEXT request on the same thread must succeed.
/// Running both executions sequentially on one thread models the serve
/// per-request loop; the breach flag is cleared at each run's `BudgetGuard`
/// install so it never leaks into the following request.
#[test]
fn serve_worker_survives_breach_then_next_request_succeeds() {
    // Request 1: breaches the memory ceiling — surfaces cleanly (no panic).
    let breaching = r#"
        let mut a = [0]
        for i in 0..2000000 {
            a.push(i)
        }
        a.len()
    "#;
    let limits1 = ResourceLimits {
        max_memory_bytes: Some(4096),
        ..ResourceLimits::unlimited()
    };
    assert!(
        run_with_limits(breaching, limits1).is_err(),
        "breaching request must fail cleanly"
    );

    // Request 2: a normal request on the SAME thread runs to completion,
    // proving the worker survived and no stale breach leaked across requests.
    let normal = r#"
        let mut a = [1, 2, 3]
        a.push(4)
        a.len()
    "#;
    let limits2 = ResourceLimits {
        max_memory_bytes: Some(64 * 1024 * 1024),
        ..ResourceLimits::unlimited()
    };
    let ok = run_with_limits(normal, limits2).expect("next request must succeed");
    assert_eq!(ok.as_i64(), Some(4), "next request must compute correctly");
}
