//! Execution-level tripwires for the `NativeExecutionWitness` (#117 / R15).
//!
//! The collector's own invariants are unit-tested in
//! `shape_vm::native_witness`. What those tests cannot show is that the
//! instrumentation is *wired to the real JIT*: that installation is recorded at
//! the site that installs, that the native-entry announcement is emitted into
//! bodies Cranelift actually finalizes, and that a refusal reaches the record
//! with its class intact. Every test here runs a real Shape program through
//! `JITExecutor::execute_program` and asserts on the resulting witness.
//!
//! Gated behind `deep-tests` for the same reason the other execution-path JIT
//! tests are: each run JIT-compiles the whole prelude, which is slow and racy
//! at default test parallelism. Run with
//! `cargo test -p shape-jit --features deep-tests witness_tripwires`.

#![cfg(all(test, feature = "deep-tests"))]

use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_vm::native_witness::{
    self, Disposition, FallbackReasonClass, NativeExecutionWitness, WitnessAssertion, WitnessMode,
    assert_fallback, assert_native_dispatch,
};

use crate::executor::JITExecutor;

/// Run `source` under `--mode jit` with a witness session collecting, and
/// return the record. The program's own success or failure is deliberately not
/// asserted: a witness must be produced either way, and several tripwires below
/// exercise programs the JIT refuses.
fn jit_witness(source: &str) -> NativeExecutionWitness {
    let _ = initialize_shared_runtime();
    native_witness::activate(WitnessMode::JitWholeProgram);
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let _ = JITExecutor::new().execute_program(&mut engine, &program);
    native_witness::finish().expect("a session was active, so a witness must exist")
}

/// A hot named function the JIT compiles, plus a loop that calls it 200 times.
const HOT_ONLY: &str = r#"
fn hot(n: int) -> int {
    return n * 2 + 1
}

let mut total = 0
for i in 0..200 {
    total = total + hot(i)
}
print(total)
"#;

/// One hot function and one whose `as` cast the JIT deliberately does not lower
/// (`OpCode::ConvertToNumber` is VM-only per `vm_only_opcode_reason`). This is
/// the two-function fixture #187's acceptance criterion is written against.
const TWO_FUNCTION_FIXTURE: &str = r#"
fn hot(n: int) -> int {
    return n * 2 + 1
}

fn cold(n: int) -> number {
    return n as number
}

let mut total = 0
for i in 0..200 {
    total = total + hot(i)
}
print(total)
print(cold(3))
"#;

// ---------------------------------------------------------------------------
// Tripwire 1 — non-vacuity
// ---------------------------------------------------------------------------

#[test]
fn a_hot_function_produces_a_real_native_dispatch_count() {
    // The positive control. Without this, every negative tripwire below could
    // pass because the instrumentation never fires at all.
    let witness = jit_witness(HOT_ONLY);
    assert!(
        witness.program_fallback.is_none(),
        "expected no whole-program deopt, got {:?}",
        witness.program_fallback
    );
    let hot = assert_native_dispatch(&witness, "hot").expect("`hot` must be a native claim");
    assert_eq!(
        hot.native_dispatches, 200,
        "the loop calls `hot` 200 times, so the native body must announce entry \
         200 times — a count that cannot be produced without running it"
    );
    assert_eq!(hot.interpreter_dispatches, 0);
    assert_eq!(hot.disposition, Disposition::NativeDispatched);
}

#[test]
fn a_deopted_function_cannot_produce_a_native_dispatch_witness() {
    // Tripwire 1. Force the fallback and confirm no native claim survives for
    // EITHER function, and that the refused one carries its exact reason.
    let witness = jit_witness(TWO_FUNCTION_FIXTURE);

    // `cold` is refused with the exact opcode class, not a vague "not native".
    let cold = assert_fallback(&witness, "cold", FallbackReasonClass::VmOnlyOpcode)
        .expect("`cold` must carry a covered fallback");
    assert!(
        cold.fallback
            .as_ref()
            .is_some_and(|f| f.detail.contains("ConvertToNumber")),
        "the fallback must name the opcode the JIT refused, got {:?}",
        cold.fallback
    );
    assert_eq!(cold.native_dispatches, 0);
    assert!(!cold.native_installed);
    assert!(
        assert_native_dispatch(&witness, "cold").is_err(),
        "a refused function must never satisfy a native claim"
    );

    // Today a direct call to a non-compiled callee is a WHOLE-PROGRAM bail
    // ("Route A surface-and-stop"), so `hot` does not stay native either. That
    // is exactly the defect #187 converts to per-function fallback. The witness
    // must say so rather than reporting a native `hot` it cannot support.
    assert!(
        witness.program_fallback.is_some(),
        "the fixture must record why the program left the native path"
    );
    assert!(
        matches!(
            assert_native_dispatch(&witness, "hot"),
            Err(WitnessAssertion::ProgramFellBack { .. })
        ),
        "with the whole program deopted, `hot` is not a native claim either"
    );
}

#[test]
fn one_program_carries_both_dispositions_at_once() {
    // The witness must be able to hold a native claim and a covered fallback
    // side by side in ONE record — otherwise #187 could never assert "one
    // unsupported construct, every other hot function still native".
    let witness = jit_witness(HOT_ONLY);
    assert_native_dispatch(&witness, "hot").expect("`hot` is native here");
    let refused: Vec<_> = witness
        .functions
        .iter()
        .filter(|f| {
            f.fallback
                .as_ref()
                .is_some_and(|r| r.reason_class == FallbackReasonClass::VmOnlyOpcode)
        })
        .collect();
    assert!(
        !refused.is_empty(),
        "this program natively dispatches `hot` while other units are refused \
         for a VM-only opcode; both must appear in the same record"
    );
    for unit in refused {
        assert_eq!(unit.native_dispatches, 0);
        assert_eq!(unit.disposition, Disposition::NotReached);
    }
}

// ---------------------------------------------------------------------------
// Tripwire 2 — the known silent-divergence hazard is never silent
// ---------------------------------------------------------------------------

#[test]
fn a_map_with_a_capturing_closure_is_never_silent() {
    // `.map()` + a capturing closure is the historical silent VM != JIT
    // territory. Whatever the JIT does with it, the witness must say something
    // truthful: either a real dispatch, or an honest fallback. What it must
    // never do is omit the unit or report a native claim it cannot support.
    let witness = jit_witness(
        r#"
fn scale(xs: Array<int>, k: int) -> Array<int> {
    return xs.map(|x| x * k)
}

let out = scale([1, 2, 3, 4], 10)
print(out)
"#,
    );

    let scale = witness.lookup("scale");
    assert_eq!(scale.len(), 1, "`scale` must appear exactly once");
    let scale = scale[0];
    assert!(
        scale.disposition == Disposition::NativeDispatched
            || scale.fallback.is_some()
            || witness.program_fallback.is_some(),
        "`scale` is neither a native dispatch nor a covered fallback: {scale:?}"
    );

    let closures: Vec<_> = witness
        .functions
        .iter()
        .filter(|f| f.function_identity.contains("closure"))
        .collect();
    assert!(
        !closures.is_empty(),
        "the closure must be a registered compilation unit, not omitted"
    );
    for closure in closures {
        // The vacuity guard that matters here: a closure the JIT installed but
        // never entered must NOT read as a native claim.
        if closure.native_installed && closure.native_dispatches == 0 {
            assert_eq!(closure.disposition, Disposition::InstalledNotDispatched);
            assert!(matches!(
                assert_native_dispatch(&witness, &closure.function_identity),
                Err(WitnessAssertion::InstalledButNeverDispatched { .. })
                    | Err(WitnessAssertion::AmbiguousFunction { .. })
                    | Err(WitnessAssertion::ProgramFellBack { .. })
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Tripwire 3 — determinism
// ---------------------------------------------------------------------------

#[test]
fn two_runs_of_the_same_program_produce_identical_witnesses() {
    let first = jit_witness(HOT_ONLY).to_canonical_json();
    let second = jit_witness(HOT_ONLY).to_canonical_json();
    assert_eq!(
        first, second,
        "the witness must be byte-identical across runs of the same program"
    );
}

#[test]
fn the_artifact_digest_changes_when_the_compiled_body_changes() {
    // The "verified artifact" half of R15's binding has to be load-bearing: a
    // digest that does not move when the body moves proves nothing about what
    // was compiled.
    let a = jit_witness(HOT_ONLY);
    let b = jit_witness(&HOT_ONLY.replace("n * 2 + 1", "n * 3 + 1"));
    let da = &a.lookup("hot")[0].artifact_digest;
    let db = &b.lookup("hot")[0].artifact_digest;
    assert_ne!(
        da, db,
        "a changed body must produce a different artifact digest"
    );
    assert_eq!(
        da,
        &jit_witness(HOT_ONLY).lookup("hot")[0].artifact_digest,
        "the same body must produce the same artifact digest"
    );
}

#[test]
fn without_a_session_the_jit_records_nothing_and_still_runs() {
    // The instrumentation is opt-in: with no session, no announcement is
    // emitted, nothing is recorded, and the program's own behaviour is
    // unchanged. A witness that leaked into ordinary runs would tax every
    // measurement #187 and #188 are supposed to take.
    let _ = initialize_shared_runtime();
    native_witness::deactivate();
    assert!(!native_witness::is_active());
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(HOT_ONLY).expect("parse failed");
    JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("the program runs normally with no session");
    assert!(
        native_witness::finish().is_none(),
        "no session means no witness — never a fabricated empty one"
    );
}

// ---------------------------------------------------------------------------
// Tripwire 4 — the interpreter tier makes no native claim
// ---------------------------------------------------------------------------

#[test]
fn the_interpreter_tier_produces_a_witness_that_claims_nothing_native() {
    use shape_vm::BytecodeExecutor;

    let _ = initialize_shared_runtime();
    native_witness::activate(WitnessMode::Vm);
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(HOT_ONLY).expect("parse failed");
    let _ = BytecodeExecutor::new().execute_program(&mut engine, &program);
    let witness = native_witness::finish().expect("a session was active");

    assert_eq!(witness.mode, WitnessMode::Vm);
    assert_eq!(witness.backend, "none");
    assert_eq!(witness.instrumentation, "none");
    assert_eq!(
        witness.program_fallback.as_ref().map(|f| f.reason_class),
        Some(FallbackReasonClass::ModeVm)
    );
    assert!(
        witness.functions.iter().all(|f| !f.native_installed),
        "the interpreter installs no native code"
    );
    assert!(
        witness
            .functions
            .iter()
            .all(|f| f.native_dispatches == 0 && f.disposition != Disposition::NativeDispatched),
        "no unit may claim a native dispatch under --mode vm"
    );
    // And the units are still enumerated, so a consumer gets a truthful record
    // rather than an empty one.
    assert!(
        !witness.lookup("hot").is_empty(),
        "`hot` must still be a named unit under --mode vm"
    );
}
