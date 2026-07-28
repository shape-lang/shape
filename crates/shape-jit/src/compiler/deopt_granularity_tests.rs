//! ADR-018 §2 / #187 — per-function deopt granularity.
//!
//! Before #187, one unsupported construct anywhere in a program cost EVERY
//! function its native code: `JITExecutor::execute_with_jit` read a
//! program-wide `has_*_residual` flag and refused to compile at all. These
//! tests pin the replacement contract at the JIT's own classification seam —
//! the `MixedFunctionTable` that decides, per function, between a native
//! pointer and an interpreter entry.
//!
//! What is asserted here is the JIT's dispatch DECISION, read from the
//! compiler's own output rather than parsed from log prose. Per R15 that is
//! not a nativity claim: a `FunctionEntry::Native` entry proves the function
//! was compiled and installed, not that a native frame executed. The
//! execution-side assertion belongs to `NativeExecutionWitness` (#117) and is
//! the one pending edit on this fixture — see
//! `two_function_fixture_pending_native_execution_witness`.

#[cfg(feature = "deep-tests")]
use crate::mixed_table::FunctionEntry;
use shape_vm::bytecode::{BytecodeProgram, JitResidual, ResidualScope};

/// Compile Shape source to bytecode. No JIT compilation, so this is cheap
/// enough for the ungated tier.
fn compile_to_bytecode(source: &str) -> BytecodeProgram {
    use shape_vm::BytecodeCompiler;

    shape_runtime::initialize_shared_runtime().ok();
    let program = shape_ast::parse_program(source).expect("parse failed");

    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        shape_vm::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .expect("module graph construction failed");

    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    compiler.set_source(source);
    compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .expect("bytecode compilation failed")
}

/// Run the JIT's selective compiler over `bytecode`, returning the
/// per-function dispatch table.
///
/// Callers are `deep-tests`-gated: JIT-compiling a program pulls in ~118
/// stdlib functions per test, which is both slow and subject to the known
/// SIGILL race at default parallelism (see the crate-level gate rationale in
/// `mir_compiler/mod.rs`).
#[cfg(feature = "deep-tests")]
fn mixed_table_for(
    bytecode: &BytecodeProgram,
) -> Result<crate::mixed_table::MixedFunctionTable, String> {
    let mut jit =
        crate::compiler::JITCompiler::new(crate::JITConfig::default()).expect("JIT init failed");
    jit.compile_program_selective("main", bytecode)
        .map(|(_jit_fn, table)| table)
}

fn function_index(program: &BytecodeProgram, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|f| f.name == name)
        .unwrap_or_else(|| {
            panic!(
                "fixture function `{name}` not found; program declares {:?}",
                program
                    .functions
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

/// The two-function fixture: one function holds the unsupported construct
/// (`?`, an `Owner`-scoped residual), the other is ordinary hot arithmetic.
const TWO_FUNCTION_FIXTURE: &str = r#"
fn make_ok() -> Result<int, string> {
    return Ok(42)
}

fn uses_try() -> Result<int, string> {
    let v = make_ok()?
    return Ok(v + 1)
}

fn hot_double(n: int) -> int {
    return n * 2
}

print(hot_double(21))
"#;

#[test]
fn try_operator_is_attributed_to_its_enclosing_function_not_the_program() {
    let bytecode = compile_to_bytecode(TWO_FUNCTION_FIXTURE);

    let uses_try = function_index(&bytecode, "uses_try");
    let hot_double = function_index(&bytecode, "hot_double");

    assert!(
        bytecode
            .jit_residuals
            .for_function(uses_try)
            .any(|r| r == JitResidual::TryUnwrap),
        "the `?` residual must be attributed to `uses_try`, which holds it"
    );
    assert!(
        !bytecode
            .jit_residuals
            .function_is_residual_bearing(hot_double),
        "`hot_double` contains no residual construct and must carry no attribution"
    );
    assert!(
        !bytecode.jit_residuals.top_level_is_residual_bearing(),
        "the `?` is inside a function, so top-level code must carry no residual"
    );
    assert!(
        bytecode.has_try_unwrap_residual,
        "the program-level summary flag must still be set — `record_jit_residual` \
         is the single writer for both"
    );
    assert!(
        bytecode.jit_residuals_agree_with_summary_flags(),
        "per-owner attribution and the summary flags must not drift"
    );
}

/// Tripwire 1 (witness-free form): one unsupported construct costs its own
/// function native code and nothing else.
#[cfg(feature = "deep-tests")]
#[test]
fn one_unsupported_construct_keeps_every_other_function_native() {
    let bytecode = compile_to_bytecode(TWO_FUNCTION_FIXTURE);
    let table = mixed_table_for(&bytecode).expect(
        "a program whose only residual is Owner-scoped and inside a function must \
         still compile — the whole-program refusal is what #187 removed",
    );

    let uses_try = function_index(&bytecode, "uses_try");
    let hot_double = function_index(&bytecode, "hot_double");

    assert!(
        matches!(table.get(uses_try), Some(FunctionEntry::Interpreted(_))),
        "`uses_try` holds the `?` residual and must never run native; got {:?}",
        table.get(uses_try)
    );
    assert!(
        matches!(table.get(hot_double), Some(FunctionEntry::Native(_))),
        "`hot_double` must keep its native code — a sibling's unsupported \
         construct is exactly what #187 stops charging to it; got {:?}",
        table.get(hot_double)
    );
}

/// The R15 obligation this fixture does not yet discharge.
///
/// `FunctionEntry::Native` proves installation, not execution. Asserting that
/// `hot_double` actually ran in a native frame requires the
/// `NativeExecutionWitness` collector from #117. When that lands, this test
/// gains the witness assertion (tier-up event, native dispatch on the covered
/// path, zero covered fallback events) and stops being a placeholder. Until
/// then #187 makes no nativity claim.
#[cfg(feature = "deep-tests")]
#[test]
fn two_function_fixture_pending_native_execution_witness() {
    let bytecode = compile_to_bytecode(TWO_FUNCTION_FIXTURE);
    let table = mixed_table_for(&bytecode).expect("fixture must compile");
    let hot_double = function_index(&bytecode, "hot_double");

    // The installation-side fact, which is all this slice may claim.
    assert!(matches!(
        table.get(hot_double),
        Some(FunctionEntry::Native(_))
    ));
}

#[test]
fn a_program_scoped_residual_still_refuses_the_whole_program() {
    // `ReferenceEscapePromotion` is recorded against the CONSUMER of the
    // returned reference while the `PromotedCell` carrier is created by the
    // producer, so demoting only the consumer would leave a native producer
    // handing a raw stack address to an interpreted consumer. The refusal
    // stays whole-program until that producer/consumer split is closed.
    assert_eq!(
        JitResidual::ReferenceEscapePromotion.scope(),
        ResidualScope::Program
    );
    assert_eq!(
        JitResidual::ModuleFnMarshalReturn.scope(),
        ResidualScope::Program
    );
    assert_eq!(JitResidual::TryUnwrap.scope(), ResidualScope::Owner);
    assert_eq!(JitResidual::NullCoalesce.scope(), ResidualScope::Owner);
    assert_eq!(
        JitResidual::ImportedConstInline.scope(),
        ResidualScope::Owner
    );
}

#[cfg(feature = "deep-tests")]
#[test]
fn a_program_with_no_residual_leaves_every_function_eligible() {
    let bytecode = compile_to_bytecode(
        r#"
fn hot_double(n: int) -> int {
    return n * 2
}

fn hot_triple(n: int) -> int {
    return n * 3
}

print(hot_double(21) + hot_triple(7))
"#,
    );
    let table = mixed_table_for(&bytecode).expect("a residual-free program must compile");

    assert!(bytecode.jit_residuals.is_empty());
    assert!(!bytecode.has_any_jit_residual_summary());
    for name in ["hot_double", "hot_triple"] {
        let idx = function_index(&bytecode, name);
        assert!(
            matches!(table.get(idx), Some(FunctionEntry::Native(_))),
            "`{name}` must be native in a residual-free program; got {:?}",
            table.get(idx)
        );
    }
}
