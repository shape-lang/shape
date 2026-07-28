//! Runtime negative controls for bounds-check elision (ADR-018 §5, #191).
//!
//! The plan-level tests in `tests/bounds_elision.rs` assert *which* accesses
//! the analyzer admits. These assert the consequence that matters: an
//! out-of-range access of each widened index shape still **traps**, on the
//! JIT tier, in a function where elision is simultaneously live.
//!
//! A wrong elision is a memory-safety bug, not a performance bug, so each
//! fixture is built so the test cannot pass vacuously:
//!
//! - the same function contains an access the analyzer *does* admit, so the
//!   elided codegen path is exercised in the same compiled body;
//! - `elision_is_live` asserts the plan is non-empty for the fixture, so a
//!   future change that silently stops admitting anything fails here rather
//!   than turning every trap test green for the wrong reason;
//! - the VM's result is asserted alongside the JIT's, so a trap that came
//!   from a whole-function bail rather than from the checked path would show
//!   up as agreement on the wrong error.
//!
//! Gated behind `deep-tests` for the same reason as `witness_tripwires`: each
//! run JIT-compiles the whole prelude. Run with
//! `cargo test -p shape-jit --features deep-tests bounds_elision_traps`.

#![cfg(all(test, feature = "deep-tests"))]

use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_vm::native_witness::{self, WitnessMode, assert_native_dispatch};

use crate::executor::JITExecutor;
use crate::mir_compiler::bounds_elision;

/// Run `source` on the JIT tier; `Ok` carries the program's result value so
/// the two tiers can be compared on more than "did not error".
///
/// `func` must be natively installed and dispatched — otherwise the run
/// exercised the interpreter and proves nothing about the elided codegen.
/// This is the guard against a trap test that is green because the JIT bailed.
fn run_jit(func: &str, source: &str) -> Result<String, String> {
    let _ = initialize_shared_runtime();
    native_witness::activate(WitnessMode::JitWholeProgram);
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let outcome = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .map(|r| format!("{:?}", r.wire_value))
        .map_err(|e| e.to_string());
    let witness = native_witness::finish().expect("a session was active");
    if let Err(e) = assert_native_dispatch(&witness, func) {
        panic!(
            "`{func}` did not run natively ({e:?}); this fixture cannot speak \
             to the elided path.\nwitness: {witness:#?}"
        );
    }
    outcome
}

/// Run `source` on the bytecode interpreter.
fn run_vm(source: &str) -> Result<String, String> {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    shape_vm::BytecodeExecutor::new()
        .execute_program(&mut engine, &program)
        .map(|r| format!("{:?}", r.wire_value))
        .map_err(|e| e.to_string())
}

/// Assert the analyzer admits at least one access in `func` — the guard
/// against a vacuously-passing trap test.
#[track_caller]
fn elision_is_live(func: &str, source: &str) {
    let program = shape_vm::stdlib::compile_source("trap_fixture.shape", source)
        .expect("fixture failed to compile");
    let mir = &program
        .functions
        .iter()
        .find(|f| f.name == func)
        .unwrap_or_else(|| panic!("fixture has no function `{func}`"))
        .mir_data
        .as_ref()
        .expect("function carries no MIR")
        .mir;
    let plan = bounds_elision::analyze(mir);
    assert!(
        !plan.is_empty(),
        "fixture `{func}` admits no elision, so its trap proves nothing about \
         the elided path",
    );
}

/// Both tiers must reject the access, and with an out-of-bounds diagnostic —
/// not a bail, a crash, or a wrong answer.
#[track_caller]
fn assert_both_tiers_trap(func: &str, source: &str) {
    elision_is_live(func, source);
    let vm = run_vm(source);
    let jit = run_jit(func, source);
    let vm_err = vm.expect_err("the VM must reject the out-of-range access");
    let jit_err = jit.expect_err("the JIT must reject the out-of-range access");
    for (tier, err) in [("vm", &vm_err), ("jit", &jit_err)] {
        assert!(
            err.to_lowercase().contains("out of bounds")
                || err.to_lowercase().contains("out-of-bounds"),
            "{tier} rejected the access but not as out-of-bounds: {err}",
        );
    }
}

// ── Shape (a): constant index ───────────────────────────────────────────

/// `arr[0]` is admitted inside the guarded body; `arr[k]` for `k` past the
/// proven length is not, and must trap on a short array.
#[test]
fn constant_index_beyond_the_proven_length_still_traps() {
    let src = r#"
fn head(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        acc = acc + arr[0] + arr[3]
        i = i + 1
    }
    return acc
}
print(head([7]))
"#;
    assert_both_tiers_trap("head", src);
}

// ── Shape (b): `iv ± constant` ──────────────────────────────────────────

/// The bound gives one element of slack, so `arr[i + 1]` is admitted and
/// `arr[i + 2]` is not; the latter runs off the end on the last iteration.
#[test]
fn iv_offset_beyond_the_slack_still_traps() {
    let src = r#"
fn window(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    let n = arr.length - 1
    while i < n {
        acc = acc + arr[i + 1] + arr[i + 2]
        i = i + 1
    }
    return acc
}
print(window([1, 2, 3]))
"#;
    assert_both_tiers_trap("window", src);
}

/// A negative offset below the induction variable's proven start value must
/// trap rather than reading backwards out of the buffer — the unchecked path
/// skips index normalisation, so this is the shape that would silently read
/// out-of-object memory if it were ever admitted.
#[test]
fn negative_iv_offset_below_the_start_value_still_traps() {
    let src = r#"
fn back(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 1
    let n = arr.length - 1
    while i < n {
        acc = acc + arr[i + 1] + arr[i - 2]
        i = i + 1
    }
    return acc
}
print(back([1, 2, 3, 4]))
"#;
    assert_both_tiers_trap("back", src);
}

// ── Shape (c): field-projected receiver ─────────────────────────────────

/// Shape (c) is proven at the plan level (see `tests/bounds_elision.rs`) but
/// **cannot reach the native tier today**, so it has no runtime trap control:
/// a read of an `Array`-typed struct field has no projected `NativeKind`, and
/// `read_place`'s `Place::Field` arm surfaces-and-stops before any indexing
/// happens. That blocker is independent of bounds-checking — it blocks the
/// checked path too.
///
/// This is written as a tripwire rather than a disabled test so it fails the
/// moment the blocker lifts, at which point the real trap control below it
/// should be enabled.
#[test]
fn field_projected_receiver_is_blocked_from_the_native_tier() {
    let src = r#"
type Buf { data: Array<int> }
fn total(b: Buf) -> int {
    let mut acc = 0
    let mut i = 0
    while i < b.data.length {
        acc = acc + b.data[i]
        i = i + 1
    }
    return acc
}
print(total(Buf { data: [1, 2, 3] }))
"#;
    // The analyzer does admit the access — the widening itself is done.
    elision_is_live("total", src);

    let _ = initialize_shared_runtime();
    native_witness::activate(WitnessMode::JitWholeProgram);
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(src).expect("parse failed");
    let _ = JITExecutor::new().execute_program(&mut engine, &program);
    let witness = native_witness::finish().expect("a session was active");
    let record = witness.program_fallback.as_ref().expect(
        "field-projected array receivers still deopt on the `.data` field-read \
         gap; if this program now compiles natively, enable \
         `field_projected_receiver_out_of_range_still_traps` below and delete \
         this tripwire",
    );
    assert!(
        record.detail.contains("unresolved direct field read"),
        "expected the `.data` field-read blocker, got: {}",
        record.detail,
    );
}

/// The trap control this blocker is holding back, recorded here rather than
/// added as a disabled test — the R14 ignored-test ratchet is shrink-only,
/// and a disabled test is legacy authority, not evidence. When the tripwire
/// above starts failing, add this as a real test alongside the other three:
///
/// ```text
/// #[test]
/// fn field_projected_receiver_out_of_range_still_traps() {
///     let src = r#"
/// type Buf { data: Array<int> }
/// fn total(b: Buf) -> int {
///     let mut acc = 0
///     let mut i = 0
///     while i < b.data.length {
///         acc = acc + b.data[i] + b.data[i + 1]
///         i = i + 1
///     }
///     return acc
/// }
/// print(total(Buf { data: [1, 2, 3] }))
/// "#;
///     assert_both_tiers_trap("total", src);
/// }
/// ```
const _SHAPE_C_TRAP_CONTROL_PENDING: () = ();

// ── Positive controls: the admitted shapes still compute correctly ──────

/// The elided accesses must produce the same answer the checked path does.
/// Without this, "nothing traps" and "nothing is computed" look alike.
#[test]
fn admitted_shapes_agree_with_the_interpreter() {
    let src = r#"
fn kernel(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 1
    let n = arr.length - 2
    while i < n {
        acc = acc + arr[i - 1] + arr[i] + arr[i + 1] + arr[i + 2]
        i = i + 1
    }
    return acc
}
let src: Array<int> = [1, 2, 3, 4, 5, 6, 7, 8]
kernel(src)
"#;
    elision_is_live("kernel", src);
    let vm = run_vm(src).expect("the interpreter must accept the kernel");
    let jit = run_jit("kernel", src).expect("the JIT must accept the kernel");
    assert_eq!(vm, jit, "elided JIT result diverges from the interpreter");
}

/// The same agreement check for a plain local receiver with a write-side
/// elision, which takes the v2 `TypedArray<T>` carrier.
#[test]
fn elided_writes_agree_with_the_interpreter() {
    let src = r#"
fn fill(arr: Array<int>) -> int {
    let mut i = 0
    while i < arr.length {
        arr[i] = i * 3
        i = i + 1
    }
    let mut acc = 0
    let mut k = 0
    while k < arr.length {
        acc = acc + arr[k]
        k = k + 1
    }
    return acc
}
fill([0, 0, 0, 0, 0, 0])
"#;
    elision_is_live("fill", src);
    let vm = run_vm(src).expect("the interpreter must accept the fixture");
    let jit = run_jit("fill", src).expect("the JIT must accept the fixture");
    assert_eq!(vm, jit, "elided JIT result diverges from the interpreter");
}
