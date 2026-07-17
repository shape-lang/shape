//! ADR-009 C2 #13 — install-atomicity pins.
//!
//! Slice 0 wrote these as EXECUTE-the-assumption pins that DOCUMENTED the debt:
//! the generated-body install path was non-atomic, so a rejected install left a
//! registered ghost function plus a live generated-symbol reservation behind.
//! Slice 1 lands the atomic install transaction
//! ([`checked_body`](crate::compiler::checked_body)) and FLIPS those pins to the
//! atomic assertion they were designed to detect: a rejected generated install
//! now publishes NOTHING.
//!
//! Both reject modes are pinned, because the install spans the whole driver and
//! a rejection can surface in either phase (see `checked_body` module docs):
//!
//! 1. `install_is_atomic_generated_extend_body_failure_leaves_nothing` — the
//!    ANALYSIS-time reject. The generated body calls an undefined function; the
//!    shared analyzer (`analyze_program_full`, `FailFast`/`Strict`) infers it
//!    and rejects BEFORE pass-2 runs. The surviving ghost was the pre-pass
//!    registration; the transaction rolls it back.
//! 2. `install_is_atomic_generated_extend_body_pass2_failure_leaves_nothing` —
//!    the PASS-2 reject. The generated body reassigns an immutable binding: it
//!    passes analysis (a mutability violation is not a type error) but
//!    `analyze_function_body` rejects it during pass-2 body compile, AFTER it
//!    has already published the MIR fact bundle. The transaction rolls back the
//!    registration AND that fact bundle.
//! 3. `successful_generated_install_publishes_and_runs` — the success-path
//!    invariance: a generated install that succeeds still publishes everything
//!    exactly as before (compiled body, reservation, fact bundle), so the
//!    transaction's commit is a no-op on the happy path.
//!
//! The query-session retain mode (a rolled-back install keeps the generated-
//! query reservation tables for LSP tooling) is exercised by the lsp-lib suite,
//! not here — those tests are its arbiter (commit `4dce9471`).
//!
//! `solver_reentry_is_idempotent_over_same_semantic_identity` is unchanged from
//! slice 0: the per-function body-analysis bundle re-runs over the same semantic
//! identity idempotently, the property slice 2 depends on.

use super::BytecodeCompiler;
use shape_ast::ast::{FunctionDef, Item};

/// The known-good generated-extend fixture shape (mirrors the S2 provenance
/// test at this file's parent module), but the generated method body calls an
/// undefined function so it is REJECTED at analysis time. The annotation
/// handler itself succeeds (it only emits source text); the pre-pass registers
/// the method SIGNATURE; the shared analyzer then infers the generated body and
/// rejects the undefined callee before pass-2 body compile.
const FAILING_ANALYSIS_BODY_PROGRAM: &str = r#"
annotation genfail() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ nonexistent() \} \}")
  }
}

@genfail()
type Ghost { id: int }
"#;

/// The analysis-time fixture's generated method name (Type.method), the key
/// under which the function table and the generated-symbol table record it.
const ANALYSIS_METHOD_NAME: &str = "Ghost.answer";

/// A generated-extend fixture whose body PASSES analysis but is REJECTED during
/// pass-2 body compile: reassigning the immutable `let x` is a mutability
/// violation, which the shared analyzer does not flag (it is not a type error)
/// but `analyze_function_body` rejects — AFTER publishing the MIR fact bundle
/// (see `functions.rs`'s `mir_borrow_analyses` retention on a reassignment
/// error). This drives the pass-2 branch of the install transaction, including
/// fact-bundle rollback.
const FAILING_PASS2_BODY_PROGRAM: &str = r#"
annotation genmut() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ let x = 1; x = 2; x \} \}")
  }
}

@genmut()
type Mut { id: int }
"#;

/// The pass-2 fixture's generated method name.
const PASS2_METHOD_NAME: &str = "Mut.answer";

/// A generated-extend fixture that installs SUCCESSFULLY (mirrors the S2
/// provenance sibling test): the generated method returns a literal, passing
/// both phases, so the transaction commits and every publication survives.
const SUCCEEDING_BODY_PROGRAM: &str = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }
"#;

/// The success fixture's generated method name.
const SUCCESS_METHOD_NAME: &str = "Point.answer";

fn parse_fn(source: &str) -> FunctionDef {
    shape_ast::parse_program(source)
        .expect("preflight probe parses")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(definition, _) => Some(definition),
            _ => None,
        })
        .expect("probe source contains a function")
}

/// Assert that NONE of the name-keyed install publications for `method_name`
/// survive: the function-table entry, every side table `register_function`
/// touches, and the `analyze_function_body` fact bundle. Shared by both reject
/// pins so the atomic guarantee is asserted identically in both phases.
fn assert_no_install_publication_survives(compiler: &BytecodeCompiler, method_name: &str) {
    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == method_name),
        "rejected generated install must leave NO function-table entry (was a registered ghost before slice 1)"
    );
    assert!(
        !compiler.generated_symbols.contains_name(method_name),
        "rejected generated install must leave NO generated-symbol reservation"
    );
    // Side tables `register_function` publishes.
    assert!(
        !compiler.function_defs.contains_key(method_name),
        "rejected generated install must leave NO function_defs entry"
    );
    assert!(
        !compiler.function_arity_bounds.contains_key(method_name),
        "rejected generated install must leave NO function_arity_bounds entry"
    );
    assert!(
        !compiler.function_const_params.contains_key(method_name),
        "rejected generated install must leave NO function_const_params entry"
    );
    assert!(
        compiler
            .type_tracker
            .get_function_return_concrete_type(method_name)
            .is_none(),
        "rejected generated install must leave NO type_tracker return type"
    );
    // The `analyze_function_body` fact bundle — ALL SEVEN name-keyed maps (the
    // subset hazard the review flagged: a partial set lets a table re-hide the
    // ghost).
    assert!(
        !compiler.mir_functions.contains_key(method_name),
        "rejected generated install must leave NO mir_functions fact"
    );
    assert!(
        !compiler.mir_borrow_analyses.contains_key(method_name),
        "rejected generated install must leave NO mir_borrow_analyses fact"
    );
    assert!(
        !compiler.mir_storage_plans.contains_key(method_name),
        "rejected generated install must leave NO mir_storage_plans fact"
    );
    assert!(
        !compiler.mir_field_analyses.contains_key(method_name),
        "rejected generated install must leave NO mir_field_analyses fact"
    );
    assert!(
        !compiler.mir_span_to_point.contains_key(method_name),
        "rejected generated install must leave NO mir_span_to_point fact"
    );
    assert!(
        !compiler.function_borrow_summaries.contains_key(method_name),
        "rejected generated install must leave NO function_borrow_summaries fact"
    );
    assert!(
        !compiler
            .function_return_reference_summaries
            .contains_key(method_name),
        "rejected generated install must leave NO function_return_reference_summaries fact"
    );
}

/// FLIP OF THE SLICE-0 PIN — the ANALYSIS-time reject is atomic.
///
/// Routes a generated `extend` method with an undefined-callee body through the
/// REAL install path (`compile_in_place`, now the atomic-transaction wrapper).
/// The compile rejects at analysis time. Slice 0 pinned that the pre-pass
/// registration + generated-symbol reservation SURVIVED; slice 1's transaction
/// rolls them back, so nothing is observable. This is the documented
/// true-positive flip slice 0 was designed to trip.
#[test]
fn install_is_atomic_generated_extend_body_failure_leaves_nothing() {
    let program =
        shape_ast::parse_program(FAILING_ANALYSIS_BODY_PROGRAM).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();

    // The install must be REJECTED (undefined-function body error). If this ever
    // returns Ok, the fixture stopped exercising a rejection.
    let outcome = compiler.compile_in_place(&program);
    assert!(
        outcome.is_err(),
        "generated body calling an undefined function must reject at install"
    );

    // ATOMIC: the rejected install published NOTHING (was a ghost before slice 1).
    assert_no_install_publication_survives(&compiler, ANALYSIS_METHOD_NAME);

    // The call-site soft-fail set is not this path's mechanism and stays empty
    // (the transaction, not the soft-fail counter, is what makes the install
    // atomic — see CLAUDE.md §Forbidden "soft-fail counter for now").
    assert!(
        compiler.failed_call_site_specializations.is_empty(),
        "the annotation-generated path records nothing in the call-site soft-fail set"
    );
}

/// The PASS-2 reject is atomic, INCLUDING the fact bundle.
///
/// The generated body passes analysis but reassigns an immutable binding, which
/// `analyze_function_body` rejects during pass-2 body compile — after it has
/// already published the MIR fact bundle for the generated method. The
/// transaction must roll back the registration AND that fact bundle, so the
/// same nothing-survives set holds as for the analysis-time reject. This is the
/// mode a pass-2-only wrapper would have missed (C2 slice-1 supervisor ruling,
/// requirement 2).
#[test]
fn install_is_atomic_generated_extend_body_pass2_failure_leaves_nothing() {
    let program = shape_ast::parse_program(FAILING_PASS2_BODY_PROGRAM).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();

    let outcome = compiler.compile_in_place(&program);
    assert!(
        outcome.is_err(),
        "generated body reassigning an immutable binding must reject at pass-2 body compile"
    );

    // ATOMIC: nothing survives — crucially, the `analyze_function_body` fact
    // bundle that DID publish before the pass-2 mutability error is rolled back.
    assert_no_install_publication_survives(&compiler, PASS2_METHOD_NAME);
}

/// SUCCESS-PATH INVARIANCE — a successful generated install still publishes
/// everything exactly as before the transaction existed.
///
/// The transaction's commit is a no-op: on the happy path every publication
/// stays. The generated method is present in the function table with a COMPILED
/// (non-ghost, `body_length > 0`) body — the direct contrast to the reject pins,
/// where a zero-length ghost must be gone — plus its reservation and fact
/// bundle. Mirrors the S2 provenance sibling test's publication assertions.
#[test]
fn successful_generated_install_publishes_and_runs() {
    let program = shape_ast::parse_program(SUCCEEDING_BODY_PROGRAM).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("generated extend method compiles through both phases");

    let installed = compiler
        .program
        .functions
        .iter()
        .find(|f| f.name == SUCCESS_METHOD_NAME)
        .expect("successful install publishes the generated method into the function table");
    assert!(
        installed.body_length > 0,
        "successful install compiles a real (runnable) body, not a zero-length ghost"
    );
    assert!(
        compiler.generated_symbols.contains_name(SUCCESS_METHOD_NAME),
        "successful install keeps its generated-symbol reservation"
    );
    assert!(
        compiler.mir_functions.contains_key(SUCCESS_METHOD_NAME),
        "successful install keeps its analyze_function_body fact bundle"
    );
}

/// H2 — a failed install does not destroy an EARLIER successful install's
/// reservation on a reused compiler.
///
/// The pre-journal rollback reset `generated_symbols` wholesale, which would
/// have destroyed a below-watermark reservation. The undo journal removes only
/// the failed install's own `Fresh` reservations. Compiles a successful
/// generated install, then a failing one on the SAME compiler, and asserts the
/// first install's reservation survives while the second's is gone.
#[test]
fn failed_install_after_a_successful_one_preserves_the_earlier_reservation() {
    let mut compiler = BytecodeCompiler::new();

    let succeeding =
        shape_ast::parse_program(SUCCEEDING_BODY_PROGRAM).expect("success fixture parses");
    compiler
        .compile_in_place(&succeeding)
        .expect("the first generated install compiles");
    assert!(
        compiler.generated_symbols.contains_name(SUCCESS_METHOD_NAME),
        "the first install reserves its generated symbol"
    );

    let failing =
        shape_ast::parse_program(FAILING_ANALYSIS_BODY_PROGRAM).expect("failing fixture parses");
    assert!(
        compiler.compile_in_place(&failing).is_err(),
        "the second generated install rejects"
    );

    assert!(
        compiler.generated_symbols.contains_name(SUCCESS_METHOD_NAME),
        "H2: the earlier successful install's reservation must survive a later failed install"
    );
    assert!(
        !compiler.generated_symbols.contains_name(ANALYSIS_METHOD_NAME),
        "the failed install's own reservation is rolled back"
    );
}

/// PIN #2 (slice 0, UNCHANGED) — per-function body analysis is re-entrant and
/// idempotent.
///
/// Runs the body-analysis bundle (`analyze_function_body`, which lowers to MIR
/// and drives the borrow solver, publishing the fact bundle keyed by the
/// function's semantic identity == its name) TWICE over the same generated
/// function, and pins that the second run produces NO double-publication side
/// effect and STABLE facts. This is the reachable proxy for "the solver can
/// re-run over a single body without a whole-program recompile" — the property
/// slice 2 depends on. Gap note: the probe is a plainly-parsed `FunctionDef`,
/// not one carrying `GeneratedNodeOrigin` provenance, because
/// `analyze_function_body` keys purely on `function.name` — the idempotence
/// property is provenance-independent at this seam.
#[test]
fn solver_reentry_is_idempotent_over_same_semantic_identity() {
    let probe = parse_fn("fn reentry_probe(value) { let held = value; held }");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_function(&probe)
        .expect("register the re-entry probe");

    // Baseline map sizes before any analysis publication.
    let base_functions = compiler.mir_functions.len();
    let base_borrows = compiler.mir_borrow_analyses.len();
    let base_storage = compiler.mir_storage_plans.len();

    // Run 1 — publishes the fact bundle keyed by "reentry_probe".
    compiler
        .analyze_function_body(&probe)
        .expect("first per-body analysis succeeds");
    let after_first_functions = compiler.mir_functions.len();
    let after_first_borrows = compiler.mir_borrow_analyses.len();
    let after_first_storage = compiler.mir_storage_plans.len();
    let first_num_locals = compiler
        .mir_functions
        .get("reentry_probe")
        .map(|mir| mir.num_locals);
    let first_error_counts = compiler
        .mir_borrow_analyses
        .get("reentry_probe")
        .map(|analysis| (analysis.errors.len(), analysis.mutability_errors.len()));

    // The analysis is non-vacuous: it actually published MIR + borrow facts.
    assert!(
        first_num_locals.is_some() && first_error_counts.is_some(),
        "first analysis must publish a real MIR + borrow fact bundle for the probe"
    );
    assert!(
        after_first_functions > base_functions,
        "first analysis must publish exactly one new function-fact entry"
    );

    // Run 2 — SAME FunctionDef, SAME name = SAME semantic identity.
    compiler
        .analyze_function_body(&probe)
        .expect("re-entrant per-body analysis succeeds without a whole-program recompile");

    // Idempotent publication: re-analysis must not GROW any name-keyed fact map
    // (a second insert overwrites the same key — no phantom/duplicate entry).
    assert_eq!(
        compiler.mir_functions.len(),
        after_first_functions,
        "re-analysis must not double-publish the function-fact map"
    );
    assert_eq!(
        compiler.mir_borrow_analyses.len(),
        after_first_borrows,
        "re-analysis must not double-publish the borrow-analysis map"
    );
    assert_eq!(
        compiler.mir_storage_plans.len(),
        after_first_storage,
        "re-analysis must not double-publish the storage-plan map"
    );

    // Stable facts: the derived analysis facts are identical across the two runs.
    assert_eq!(
        compiler
            .mir_functions
            .get("reentry_probe")
            .map(|mir| mir.num_locals),
        first_num_locals,
        "re-analysis must produce the same MIR local count (stable facts)"
    );
    assert_eq!(
        compiler
            .mir_borrow_analyses
            .get("reentry_probe")
            .map(|analysis| (analysis.errors.len(), analysis.mutability_errors.len())),
        first_error_counts,
        "re-analysis must produce the same borrow/mutability error counts (stable facts)"
    );
}
