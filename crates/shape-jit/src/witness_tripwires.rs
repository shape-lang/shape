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
    // Tripwire 1, POST-FLIP (#188). Force the fallback and confirm no native
    // claim survives for the refused function, that it carries its exact
    // reason, and that its SIBLING keeps a real native dispatch count.
    //
    // #117 wrote this to flip when #187 landed per-function granularity. #187
    // flipped it and was reverted at `841f92f7`, because letting the direct
    // call to the non-compiled `cold` lower through the trampoline turned two
    // corpus programs into silent-wrong-output. #188 found the actual cause —
    // the trampoline discarded per-ARGUMENT kinds, not the return kind — fixed
    // it in `dispatch_call_via_trampoline_vm`, confirmed both corpus programs
    // are MATCH again, and restored the flip. See the history block at the
    // refusal site in `mir_compiler/terminators.rs`.
    //
    // The load-bearing half is `hot`: top-level calls the demoted `cold`
    // directly and the program STAYS native, so `hot`'s 200 dispatches are a
    // count no installation-only record could produce.
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

    // The flip: one unsupported construct no longer costs the whole program its
    // native code, even when top-level calls the demoted function directly.
    assert!(
        witness.program_fallback.is_none(),
        "a direct call to a demoted callee must no longer bail the whole \
         program — got {:?}",
        witness.program_fallback
    );
    let hot = assert_native_dispatch(&witness, "hot")
        .expect("`hot` must be a native claim once the program stays native");
    assert_eq!(
        hot.native_dispatches, 200,
        "the loop calls `hot` 200 times; the count comes from inside the \
         emitted body, so it cannot be produced without running it"
    );
    assert_eq!(hot.interpreter_dispatches, 0);
    assert_eq!(hot.disposition, Disposition::NativeDispatched);

    // And the demoted callee's own execution is recorded as what it is: the
    // trampoline hands it to the interpreter, once, for the single call.
    assert!(
        cold.interpreter_dispatches > 0,
        "`cold` ran somewhere — the trampoline dispatch must be recorded \
         rather than leaving the unit looking unreached"
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
// Tripwire 1b — #188 slice 2: the closure BODY runs native
// ---------------------------------------------------------------------------

/// A capturing closure called through a VARIABLE and through a PARAMETER, plus
/// a reassignment so one callee slot holds two different closures.
const CLOSURE_DISPATCH_FIXTURE: &str = r#"
fn apply(g: (int) => int, n: int) -> int { return g(n) }

let k = 3
let mut f = |x: int| x * k
let mut total = 0

for i in 0..200 {
    total = total + f(i) + apply(f, i)
}

f = |x: int| x + k

for i in 0..200 {
    total = total + f(i)
}

print(total)
"#;

#[test]
fn a_capturing_closure_body_is_natively_dispatched() {
    // The #188 acceptance claim, and the one that cannot be made from the
    // compiler's classification alone: `native_installed` says the closure was
    // COMPILED, which was already true before slice 2 while every call still
    // ran on the interpreter. Only the dispatch COUNT, recorded from inside the
    // emitted body, distinguishes the two.
    //
    // Both call shapes are covered, and they take different routes:
    //   * `f(i)` — the MIR emitter's guarded direct dispatch, which loads the
    //     captures out of the closure block and calls the body natively;
    //   * `apply(f, i)` — inside `apply`, `g` is a parameter with no
    //     `MakeClosureHeap` to speculate from, so the native call comes from
    //     the trampoline's own dispatch in `ffi/control/mod.rs`.
    //
    // The reassignment is the soundness half: two closures share one callee
    // slot, so the emitter's recorded `function_id` is wrong for half the
    // calls. Both bodies must still be natively dispatched AND the program must
    // still print the right answer — a guard that silently called the wrong
    // body would keep the counts and corrupt the result.
    let witness = jit_witness(CLOSURE_DISPATCH_FIXTURE);

    assert!(
        witness.program_fallback.is_none(),
        "the fixture must stay on the native path, got {:?}",
        witness.program_fallback
    );

    let closures: Vec<_> = witness
        .functions
        .iter()
        .filter(|f| f.function_identity.starts_with("__closure"))
        .collect();
    assert_eq!(
        closures.len(),
        2,
        "both closure literals must be registered units, got {:?}",
        closures
            .iter()
            .map(|f| &f.function_identity)
            .collect::<Vec<_>>()
    );

    for closure in &closures {
        let unit = assert_native_dispatch(&witness, &closure.function_identity)
            .unwrap_or_else(|e| panic!("{} must be a native claim: {e:?}", closure.function_identity));
        assert!(
            unit.native_dispatches > 0,
            "{} announced no native entry — installation alone is not a nativity \
             claim (R15)",
            closure.function_identity
        );
        assert_eq!(
            unit.interpreter_dispatches, 0,
            "{} still reached the interpreter trampoline",
            closure.function_identity
        );
    }

    // The first closure is called through both the variable and the parameter
    // route (200 each = 400); the second only through the variable (200). 600
    // native closure entries, none of them interpreted.
    let mut counts: Vec<u64> = closures.iter().map(|f| f.native_dispatches).collect();
    counts.sort_unstable();
    assert_eq!(
        counts,
        vec![200, 400],
        "expected one closure entered 400 times (variable + parameter) and one \
         entered 200 times (variable only); got {counts:?}"
    );

    // `apply` itself stays native too — the closure call inside it must not
    // cost it its own compilation.
    let apply = assert_native_dispatch(&witness, "apply").expect("`apply` must be native");
    assert_eq!(apply.native_dispatches, 200);
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
