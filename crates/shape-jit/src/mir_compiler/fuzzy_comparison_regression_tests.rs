//! W10 fuzzy-comparison VM/JIT divergence regression.
//!
//! Root cause: the bytecode VM's fuzzy-comparison compiler emits
//! tolerance-aware numeric bytecode. MIR used to lower `Expr::FuzzyComparison`
//! to plain `Eq` / `Gt` / `Lt` and discard the tolerance, so native JIT
//! comparison codegen compared `0.1 + 0.2 == 0.3` exactly. The JIT path now
//! receives `Rvalue::FuzzyComparison` and emits the tolerance arithmetic.

use crate::executor::JITExecutor;
use crate::mir_compiler::preflight;
use shape_ast::ast::{Item, Span, Statement};
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_vm::bytecode::MirFunctionData;
use shape_vm::mir::lowering::lower_function_detailed;
use shape_vm::mir::solver::{analyze, CalleeSummaries};
use shape_vm::mir::storage_planning::{
    collect_closure_captures, plan_storage, StoragePlannerInput,
};
use shape_vm::mir::{Rvalue, StatementKind};
use shape_vm::type_tracking::BindingSemantics;
use shape_vm::BytecodeExecutor;
use shape_wire::WireValue;
use std::collections::HashMap;

fn run_value(executor_is_jit: bool, source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = if executor_is_jit {
        JITExecutor::new().execute_program(&mut engine, &program)
    } else {
        BytecodeExecutor::new().execute_program(&mut engine, &program)
    };
    result.expect("execution failed").wire_value
}

fn assert_vm_jit_bool(source: &str, expected: bool) {
    let vm = run_value(false, source);
    let jit = run_value(true, source);
    assert_eq!(
        vm,
        WireValue::Bool(expected),
        "VM value mismatch for:\n{source}"
    );
    assert_eq!(
        jit,
        WireValue::Bool(expected),
        "JIT value mismatch for:\n{source}"
    );
    assert_eq!(jit, vm, "VM/JIT output divergence for:\n{source}");
}

fn fuzzy_expr_mir_data(source: &str) -> MirFunctionData {
    let program = shape_ast::parse_program(source).expect("parse failed");
    let (expr, span) = match program.items.as_slice() {
        [Item::Expression(expr, span)] => (expr.clone(), *span),
        [Item::Statement(Statement::Expression(expr, _), span)] => (expr.clone(), *span),
        other => panic!("expected one expression item, got {other:?}"),
    };
    let body = vec![Statement::Expression(expr, span)];
    let lowered = lower_function_detailed("main", &[], &body, Span::DUMMY);
    assert!(
        !lowered.had_fallbacks,
        "fuzzy expression lowering should not use MIR fallback spans: {:?}",
        lowered.fallback_spans
    );
    let callee_summaries = CalleeSummaries::new();
    let borrow_analysis = analyze(&lowered.mir, &callee_summaries);
    let (closure_captures, mutable_captures) = collect_closure_captures(&lowered.mir);
    let binding_semantics: HashMap<u16, BindingSemantics> = HashMap::new();
    let storage_plan = plan_storage(&StoragePlannerInput {
        mir: &lowered.mir,
        analysis: &borrow_analysis,
        binding_semantics: &binding_semantics,
        closure_captures: &closure_captures,
        mutable_captures: &mutable_captures,
        had_fallbacks: lowered.had_fallbacks,
        callee_summaries: None,
    });
    MirFunctionData {
        mir: lowered.mir,
        storage_plan,
        borrow_analysis,
    }
}

fn mir_contains_fuzzy_comparison(data: &MirFunctionData) -> bool {
    data.mir.blocks.iter().any(|block| {
        block.statements.iter().any(|stmt| {
            matches!(
                stmt.kind,
                StatementKind::Assign(_, Rvalue::FuzzyComparison { .. })
            )
        })
    })
}

#[test]
fn fuzzy_comparison_mir_is_native_jit_preflightable() {
    let mir_data = fuzzy_expr_mir_data("0.1 + 0.2 ~= 0.3 within 0.0001");
    assert!(
        mir_contains_fuzzy_comparison(&mir_data),
        "MIR should preserve fuzzy comparison as a dedicated rvalue"
    );
    let preflight = preflight(&mir_data);
    assert!(
        preflight.can_compile,
        "fuzzy comparison MIR should stay on native JIT path; blockers: {:?}",
        preflight.blockers
    );
}

#[test]
fn book_fuzzy_float_equality_matches_vm_output() {
    assert_vm_jit_bool("0.1 + 0.2 ~= 0.3 within 0.0001", true);
}

#[test]
fn neighboring_fuzzy_float_equality_false_matches_vm_output() {
    assert_vm_jit_bool("0.1 + 0.2 ~= 0.31 within 0.0001", false);
}
