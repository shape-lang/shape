//! ADR-018 §2 / #187 — per-function deopt granularity.
//!
//! Before #187, one unsupported construct anywhere in a program cost EVERY
//! function its native code: `JITExecutor::execute_with_jit` read a
//! program-wide `has_*_residual` flag and refused to compile at all. These
//! tests pin the replacement contract at the JIT's own classification seam —
//! the `MixedFunctionTable` that decides, per function, between a native
//! pointer and an interpreter entry.
//!
//! Most of what is asserted here is the JIT's dispatch DECISION, read from the
//! compiler's own output rather than parsed from log prose. Per R15 that is
//! not a nativity claim on its own: a `FunctionEntry::Native` entry proves the
//! function was compiled and installed, not that a native frame executed. The
//! execution-side claim is made once, by
//! `one_unsupported_construct_keeps_every_other_function_natively_dispatched`,
//! through the `NativeExecutionWitness` collector (#117).

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
    let (graph, _stdlib_names, prelude_imports) =
        shape_vm::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .expect("module graph construction failed");

    // The prelude's name-based function-name set is deliberately left unset.
    // Sibling helpers assign it; CLAUDE.md forbids extending that surface, and
    // these fixtures do not need it — they call `print` and construct
    // `Ok`/`Err`, which resolve through the prelude imports above. All five
    // tests pass without it.
    let mut compiler = BytecodeCompiler::new();
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
///
/// Top-level calls BOTH, so the fixture exercises the call site as well as the
/// classification: a direct call to the demoted `uses_try` must lower through
/// the trampoline rather than costing top-level its own native code. (#187
/// tried this, was reverted at `841f92f7` for a silent-wrong-output the
/// differential caught, and #188 restored it after fixing the trampoline's
/// per-argument kind handoff — see the history block at the refusal site in
/// `mir_compiler/terminators.rs`.)
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

let mut total = 0
for i in 0..200 {
    total = total + hot_double(i)
}
print(total)
print(uses_try())
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

/// The R15 obligation, discharged.
///
/// The test above reads the JIT's classification, which proves installation,
/// not execution. This one runs the program under a `NativeExecutionWitness`
/// session (#117) and asserts the execution-side facts: `hot_double` announced
/// native entry from inside its own emitted body, 200 times, with zero
/// interpreter dispatches and no whole-program deopt — while `uses_try` carries
/// a covered fallback naming the `?` residual. The dispatch count cannot be
/// produced without actually running the native body, which is what makes the
/// claim non-vacuous.
#[cfg(feature = "deep-tests")]
#[test]
fn one_unsupported_construct_keeps_every_other_function_natively_dispatched() {
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    use shape_vm::native_witness::{
        self, Disposition, FallbackReasonClass, WitnessMode, assert_fallback,
        assert_native_dispatch,
    };

    shape_runtime::initialize_shared_runtime().ok();
    native_witness::activate(WitnessMode::JitWholeProgram);
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(TWO_FUNCTION_FIXTURE).expect("parse failed");
    let _ = crate::executor::JITExecutor::new().execute_program(&mut engine, &program);
    let witness = native_witness::finish().expect("a session was active");

    assert!(
        witness.program_fallback.is_none(),
        "the `?` in `uses_try` must not cost the whole program its native \
         execution; got {:?}",
        witness.program_fallback
    );

    let hot = assert_native_dispatch(&witness, "hot_double")
        .expect("`hot_double` must be natively dispatched despite its sibling's residual");
    assert_eq!(
        hot.native_dispatches, 200,
        "the loop calls `hot_double` 200 times, so its native body must announce \
         entry 200 times — a count only running it can produce"
    );
    assert_eq!(hot.interpreter_dispatches, 0);
    assert_eq!(hot.disposition, Disposition::NativeDispatched);

    let cold = assert_fallback(&witness, "uses_try", FallbackReasonClass::TryUnwrapResidual)
        .expect("`uses_try` must carry a covered fallback naming the `?` residual");
    assert_eq!(cold.native_dispatches, 0);
    assert!(
        assert_native_dispatch(&witness, "uses_try").is_err(),
        "a residual-bearing function must never satisfy a native claim"
    );
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
