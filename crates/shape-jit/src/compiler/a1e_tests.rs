//! Track A.1E — end-to-end JIT tests for `var` closures (Shared captures).
//!
//! Mirrors the A.1D.2 test structure. Non-gated tests cover the
//! compile-path contracts (preflight acceptance + layout classification
//! + Native dispatch assignment). Gated tests (`jit_v2_unstable_tests`
//! cfg) exercise full runtime behaviour and currently fail because of
//! the pre-existing branch-wide JIT closure dispatch regression
//! (`project_jit_closure_fix.md`). The gated tests exist as a regression
//! net for when that dispatch bug is fixed in a follow-up session.

use crate::compiler::JITCompiler;
use crate::mixed_table::FunctionEntry;
use crate::JITConfig;

/// Compile Shape source end-to-end through the JIT. Returns the program
/// + mixed table so tests can assert on per-function dispatch.
fn compile_to_mixed_table(
    source: &str,
) -> (
    shape_vm::bytecode::BytecodeProgram,
    crate::mixed_table::MixedFunctionTable,
) {
    use shape_vm::BytecodeCompiler;

    shape_runtime::initialize_shared_runtime().ok();
    let program = shape_ast::parse_program(source).expect("parse failed");

    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        shape_vm::module_resolution::build_graph_and_stdlib_names(
            &program,
            &mut loader,
            &[],
        )
        .expect("module graph construction failed");

    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    compiler.set_source(source);
    let bytecode = compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .expect("bytecode compilation failed");

    let jit_config = JITConfig::default();
    let mut jit = JITCompiler::new(jit_config).expect("JIT init failed");
    let (_jit_fn, mixed_table) = jit
        .compile_program_selective("main", &bytecode)
        .expect("JIT compilation failed");
    (bytecode, mixed_table)
}

/// Returns `true` when `mixed_table` has at least one closure function
/// (flagged by the bytecode compiler's `is_closure`) dispatched as
/// `FunctionEntry::Native`.
fn has_native_closure(
    program: &shape_vm::bytecode::BytecodeProgram,
    table: &crate::mixed_table::MixedFunctionTable,
) -> bool {
    program
        .functions
        .iter()
        .enumerate()
        .any(|(idx, func)| {
            func.is_closure
                && matches!(table.get(idx), Some(FunctionEntry::Native(_)))
        })
}

/// End-to-end runner used only by the gated tests — same shape as
/// `a1d2_tests::jit_run`.
#[cfg(jit_v2_unstable_tests)]
fn jit_run(source: &str) -> shape_wire::WireValue {
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

// ---------------------------------------------------------------------------
// 1. Preflight + compile-path tests (non-gated, always-run).
// ---------------------------------------------------------------------------

#[test]
fn a1e_jit_var_closure_body_is_natively_compiled() {
    // Minimal `var` counter closure. After A.1E, the closure body's
    // `Load/StoreSharedCapture` opcodes pass JIT preflight, so the
    // closure lands in the mixed table as `FunctionEntry::Native`.
    // Under A.1D.2 (pre-A.1E) this would appear as `Interpreted(...)`.
    let source = r#"
        fn main() -> int {
            var x: int = 0
            let f = || { x = x + 1 }
            f()
            x
        }
        main()
    "#;
    let (program, table) = compile_to_mixed_table(source);
    assert!(
        has_native_closure(&program, &table),
        "A.1E: closure body containing Load/StoreSharedCapture must \
         JIT-compile to FunctionEntry::Native. native={} interpreted={}",
        table.native_count(),
        table.interpreted_count()
    );
}

#[test]
fn a1e_closure_layout_shared_kind_classifies() {
    // Pins `ClosureLayout::capture_storage_kind` for Shared — the
    // classification the A.1E side-table reads to decide which capture
    // param slots drive lock-gated pointer-deref lowering.
    use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
    use shape_value::v2::ConcreteType;

    let capture_types = vec![
        ConcreteType::I64,
        ConcreteType::I64,
        ConcreteType::F64,
    ];
    let kinds = vec![
        CaptureKind::Immutable,
        CaptureKind::Shared,
        CaptureKind::OwnedMutable,
    ];
    let layout = ClosureLayout::from_capture_types(&capture_types, &kinds);

    assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
    assert_eq!(layout.capture_storage_kind(1), CaptureKind::Shared);
    assert_eq!(layout.capture_storage_kind(2), CaptureKind::OwnedMutable);
    // shared_capture_mask has exactly bit 1 set.
    assert_eq!(
        layout.shared_capture_mask, 0b010,
        "shared_capture_mask must have exactly bit 1 set for the \
         single Shared capture at index 1"
    );
    // The three masks are disjoint.
    assert_eq!(layout.heap_capture_mask, 0);
    assert_eq!(layout.owned_mutable_capture_mask, 0b100);
    assert_eq!(
        layout.shared_capture_mask & layout.owned_mutable_capture_mask,
        0,
        "shared and owned_mutable masks must be disjoint"
    );
    assert!(layout.is_shared_capture(1));
    assert!(!layout.is_shared_capture(0));
    assert!(!layout.is_shared_capture(2));
}

// ---------------------------------------------------------------------------
// 2. End-to-end execution tests (gated).
//
// These are gated behind `jit_v2_unstable_tests` matching A.1D.2's
// pattern. On `jit-v2-phase1` today they ALL fail the same way as
// the pre-existing `closure_simple` test (closure dispatch returns
// `Null` instead of the computed result — see memory note
// `project_jit_closure_fix.md`). The fail-mode is not A.1E-specific;
// A.1D.2's gated tests fail with the same symptom. These are net
// regression tests for when the dispatch fix lands in a follow-up.
// ---------------------------------------------------------------------------

#[cfg(jit_v2_unstable_tests)]
#[test]
fn a1e_jit_var_counter_e2e() {
    // Three calls to an incrementing `var` closure; final read should be 3.
    let source = r#"
        fn main() -> int {
            var x: int = 0
            let inc = || { x = x + 1 }
            inc()
            inc()
            inc()
            x
        }
        main()
    "#;
    match jit_run(source) {
        shape_wire::WireValue::Integer(n) => {
            assert_eq!(n, 3, "counter must reach 3 after three inc() calls");
        }
        shape_wire::WireValue::Number(n) => {
            assert!(
                (n - 3.0).abs() < 1e-9,
                "counter must reach 3 (got Number {})",
                n
            );
        }
        other => panic!("expected Integer(3), got {:?}", other),
    }
}

#[cfg(jit_v2_unstable_tests)]
#[test]
fn a1e_jit_var_two_closures_share_cell() {
    // Two closures capture the SAME `var x`. Both Arcs point at the
    // same SharedCell; mutations through `inc` must be visible via
    // `dec` and vice versa.
    let source = r#"
        fn main() -> int {
            var x: int = 10
            let inc = || { x = x + 1 }
            let dec = || { x = x - 1 }
            inc()
            inc()
            dec()
            x
        }
        main()
    "#;
    match jit_run(source) {
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 11),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 11.0).abs() < 1e-9, "expected 11, got Number {}", n)
        }
        other => panic!("expected Integer(11), got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 3. cell-identity fix regression tests — exercise the outer-scope Shared
//    local lifecycle now that preflight accepts AllocSharedLocal /
//    LoadSharedLocal / StoreSharedLocal / DropSharedLocal. Previously these
//    programs triggered a SIGSEGV because the JIT's `main` ran without an
//    Arc<SharedCell> allocation and the closure's `jit_arc_shared_retain`
//    dereferenced a plain scalar as a cell pointer.
// ---------------------------------------------------------------------------

#[test]
fn a1e_jit_var_no_closure_runs() {
    // `var` binding without any closure capture. MIR storage planner
    // classifies this slot as Direct (no promotion). The JIT compiles
    // the function as a plain integer — no Arc<SharedCell>. Exercise
    // the path that must remain unaffected by the cell-identity fix.
    let source = r#"
        fn main() -> int {
            var x: int = 0
            x = x + 1
            x = x + 1
            x
        }
        main()
    "#;
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    match result.wire_value {
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 2),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 2.0).abs() < 1e-9, "expected 2, got Number {}", n)
        }
        other => panic!("expected Integer(2), got {:?}", other),
    }
}

#[test]
fn a1e_jit_var_captured_then_outer_observe() {
    // Closure captures `x`, single call updates it, outer reads the
    // post-call value. Pins the canonical cell-identity contract: the
    // same Arc<SharedCell> is seen by the JIT'd outer function and
    // the JIT'd closure body.
    let source = r#"
        fn main() -> int {
            var x: int = 5
            let f = || { x = x + 10 }
            f()
            x
        }
        main()
    "#;
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    match result.wire_value {
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 15),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 15.0).abs() < 1e-9, "expected 15, got Number {}", n)
        }
        other => panic!("expected Integer(15), got {:?}", other),
    }
}

#[test]
fn a1e_jit_two_closures_share_cell() {
    // Two closures capture the same `var x`. Both share the Arc
    // strong count; mutations via one are visible from the other.
    let source = r#"
        fn main() -> int {
            var x: int = 10
            let inc = || { x = x + 1 }
            let dec = || { x = x - 1 }
            inc()
            inc()
            dec()
            x
        }
        main()
    "#;
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    match result.wire_value {
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 11),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 11.0).abs() < 1e-9, "expected 11, got Number {}", n)
        }
        other => panic!("expected Integer(11), got {:?}", other),
    }
}

#[test]
fn a1e_jit_var_counter_repeated_calls() {
    // Three successive calls accumulate through the shared cell.
    // Mirrors `a1e_jit_var_counter_e2e` but un-gated — the cell-
    // identity fix makes this stable without `jit_v2_unstable_tests`.
    let source = r#"
        fn main() -> int {
            var x: int = 0
            let inc = || { x = x + 1 }
            inc()
            inc()
            inc()
            x
        }
        main()
    "#;
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    match result.wire_value {
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 3),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 3.0).abs() < 1e-9, "expected 3, got Number {}", n)
        }
        other => panic!("expected Integer(3), got {:?}", other),
    }
}

#[cfg(jit_v2_unstable_tests)]
#[test]
fn a1e_jit_mixed_let_letmut_var() {
    // Closure captures one of each kind — immutable `base` (let),
    // OwnedMutable `accum` (let mut, moved into closure), Shared
    // `shared` (var). All three lowering paths must coexist in a
    // single closure layout. Under Rust-move semantics, outer reads
    // of `accum` after capture are a compile error — so the closure
    // folds accum into shared, which stays outer-accessible.
    let source = r#"
        fn main() -> int {
            let base: int = 100
            let mut accum: int = 0
            var shared: int = 7
            let f = || {
                accum = accum + base
                shared = shared + accum
            }
            f()
            f()
            shared
        }
        main()
    "#;
    match jit_run(source) {
        // f() #1: accum = 0 + 100 = 100, shared = 7 + 100 = 107
        // f() #2: accum = 100 + 100 = 200, shared = 107 + 200 = 307
        // outer returns shared = 307.
        shape_wire::WireValue::Integer(n) => assert_eq!(n, 307),
        shape_wire::WireValue::Number(n) => {
            assert!((n - 307.0).abs() < 1e-9, "expected 307, got Number {}", n)
        }
        other => panic!("expected Integer(307), got {:?}", other),
    }
}
