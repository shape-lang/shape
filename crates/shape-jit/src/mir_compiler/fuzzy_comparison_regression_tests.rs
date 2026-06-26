//! W10 fuzzy-comparison VM/JIT divergence regression.
//!
//! Root cause: the bytecode VM's fuzzy-comparison compiler emits
//! tolerance-aware numeric bytecode, while the MIR consumed by the JIT lowers
//! `Expr::FuzzyComparison` to plain `Eq` / `Gt` / `Lt` and discards the
//! tolerance. The JIT executor must refuse that lossy MIR shape and run the
//! bytecode interpreter instead, preserving VM == JIT output.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_runtime::output_adapter::SharedCaptureAdapter;
use shape_vm::BytecodeExecutor;

fn run_output(executor_is_jit: bool, source: &str) -> Vec<String> {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let capture = SharedCaptureAdapter::new();
    if let Some(ctx) = engine.runtime.persistent_context_mut() {
        ctx.set_output_adapter(Box::new(capture.clone()));
    }

    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = if executor_is_jit {
        JITExecutor::new().execute_program(&mut engine, &program)
    } else {
        BytecodeExecutor::new().execute_program(&mut engine, &program)
    };
    result.expect("execution failed");
    capture.output()
}

fn assert_vm_jit_output(source: &str, expected: &[&str]) {
    let vm = run_output(false, source);
    let jit = run_output(true, source);
    let expected: Vec<String> = expected.iter().map(|line| (*line).to_string()).collect();
    assert_eq!(vm, expected, "VM output mismatch for:\n{source}");
    assert_eq!(jit, expected, "JIT output mismatch for:\n{source}");
    assert_eq!(jit, vm, "VM/JIT output divergence for:\n{source}");
}

#[test]
fn book_fuzzy_float_equality_matches_vm_output() {
    assert_vm_jit_output("print(0.1 + 0.2 ~= 0.3 within 0.0001)", &["true"]);
}

#[test]
fn neighboring_fuzzy_float_equality_false_matches_vm_output() {
    assert_vm_jit_output("print(0.1 + 0.2 ~= 0.31 within 0.0001)", &["false"]);
}
