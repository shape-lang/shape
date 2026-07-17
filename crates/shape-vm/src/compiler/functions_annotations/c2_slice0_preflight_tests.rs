//! ADR-009 C2 #13 — slice-0 preflight PINS (execute-the-assumption, not reason).
//!
//! These tests do not change any production behavior. They EXECUTE the two
//! riskiest assumptions the C2 slice plan rests on and pin the observed reality
//! as committed evidence, exactly as C1's slice-0 preflight earned its keep.
//!
//! 1. `install_is_non_atomic_*` pins that the CURRENT generated-body install
//!    path is NON-ATOMIC: a generated `extend` method whose body fails pass-2
//!    body compilation leaves a registered ghost function in the program's
//!    function table AND a surviving reservation in the generated-symbol table,
//!    even though `compile_in_place` returned `Err`. The `// C2 SLICE-0 PIN`
//!    assertions document the debt slice 1 must flip: when the atomic
//!    staging/commit transaction lands, a rejected install publishes NOTHING
//!    and these assertions go RED — that flip is the intended detector.
//!
//! 2. `solver_reentry_*` pins the slice plan's secondary assumption: the
//!    per-function body-analysis bundle (`analyze_function_body`, which drives
//!    the MIR borrow solver over one body) can re-run over the SAME semantic
//!    identity idempotently — no double-publication side effect, stable facts —
//!    so slice 2 can route it through the transaction without a whole-program
//!    recompile.

use super::BytecodeCompiler;
use shape_ast::ast::{FunctionDef, Item};

/// The known-good generated-extend fixture shape (mirrors the S2 provenance
/// test at this file's parent module), but the generated method body calls an
/// undefined function so pass-2 body compilation FAILS. The annotation handler
/// itself succeeds (it only emits source text); the pre-pass registers the
/// method SIGNATURE; only the pass-2 body compile trips, after registration.
const FAILING_GENERATED_BODY_PROGRAM: &str = r#"
annotation genfail() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ nonexistent() \} \}")
  }
}

@genfail()
type Ghost { id: int }
"#;

/// The generated method's derived name (Type.method), the key under which both
/// the function table and the generated-symbol table record it.
const GENERATED_METHOD_NAME: &str = "Ghost.answer";

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

/// PIN #1 — the non-atomicity of the generated-body install path.
///
/// Routes a generated `extend` method with a failing body through the REAL
/// install path (`compile_in_place` → `materialize_computed_comptime_extends`
/// pre-pass register + pass-2 `apply_comptime_extend` body compile). The
/// compile fails; we then inventory exactly what ghost state survives.
#[test]
fn install_is_non_atomic_generated_extend_body_failure_leaves_ghost_registration() {
    let program =
        shape_ast::parse_program(FAILING_GENERATED_BODY_PROGRAM).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();

    // The install must be REJECTED (undefined-function body error surfaced with
    // generated-decl provenance). If this ever returns Ok, the fixture stopped
    // exercising a rejection and the rest of the pin is meaningless.
    let outcome = compiler.compile_in_place(&program);
    assert!(
        outcome.is_err(),
        "generated body calling an undefined function must reject at install"
    );

    // C2 SLICE-0 PIN: current install is non-atomic — the pre-pass registered
    // the generated method SIGNATURE into the program function table (positional
    // push, no rollback), so the rejected install leaves a GHOST function behind.
    // Slice 1's atomic commit flips this to `!any(...)`.
    let ghost = compiler
        .program
        .functions
        .iter()
        .find(|f| f.name == GENERATED_METHOD_NAME);
    assert!(
        ghost.is_some(),
        "PIN: rejected generated body still leaves its registered function index in program.functions"
    );

    // C2 SLICE-0 PIN: the ghost is a registered-but-never-compiled ZERO-length
    // body (the exact hazard the mod.rs:1932 doc names — dispatching a
    // zero-instruction body). `register_function` seeds body_length = 0 and the
    // pass-2 body compile fails before the emission back-patch runs.
    assert_eq!(
        ghost.expect("ghost present").body_length,
        0,
        "PIN: rejected generated body is registered with an uncompiled (zero-length) body"
    );

    // C2 SLICE-0 PIN: the generated-symbol reservation ALSO survives. C1's
    // poison machinery (`poison_annotation_compiler`) fires only on the
    // annotation-DECLARATION install transaction, never on this generated-extend
    // body-compile path — so the reservation is not cleaned. Slice 1 must fold
    // this reservation into the same atomic rollback.
    assert!(
        compiler
            .generated_symbols
            .contains_name(GENERATED_METHOD_NAME),
        "PIN: rejected generated body still leaves its generated-symbol reservation"
    );

    // C2 SLICE-0 PIN: unlike the call-site-specialization path
    // (`__w24_method_*` / `__w27_implicit_*`), the annotation-generated install
    // path has NO soft-fail guard at all — `failed_call_site_specializations`
    // is never touched here, so a second consumer has zero signal that the
    // ghost body is uncompiled. This documents that the annotation path is even
    // LESS protected than the documented call-site hole.
    assert!(
        !compiler
            .failed_call_site_specializations
            .contains(GENERATED_METHOD_NAME),
        "PIN: annotation-generated install path does not record the failure in the call-site soft-fail set"
    );
    assert!(
        compiler.failed_call_site_specializations.is_empty(),
        "PIN: the call-site soft-fail set stays empty for the annotation-generated path"
    );
}

/// PIN #2 — per-function body analysis is re-entrant and idempotent.
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
