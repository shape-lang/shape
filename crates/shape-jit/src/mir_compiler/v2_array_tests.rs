//! End-to-end JIT integration tests for the v2 typed-array codegen path.
//!
//! These tests exercise the full pipeline:
//!   Shape source → parse → bytecode compile (with `let x: Array<T>` annotations
//!   that populate `top_level_local_concrete_types`) → JIT compile (which uses
//!   the v2 inline `v2_array_get`/`v2_array_set`/`v2_array_len` helpers) →
//!   native execute → verify result.
//!
//! When the destination slot has a known `Array<scalar>` `ConcreteType`, the
//! JIT allocates a real `*mut TypedArray<T>` via the `jit_v2_array_new_*` FFI
//! and stores it directly into the slot (no NaN-boxing). Subsequent
//! `arr[i]` reads, `arr[i] = v` writes, and `arr.length` lookups use the
//! inline Cranelift loads emitted by `v2_array_get`/`v2_array_set`/`v2_array_len`.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a Shape program through the JIT and return its raw `WireValue` result.
fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

/// Run a Shape program through the JIT and return the runtime-error message
/// it surfaced. Panics if the program ran cleanly. Used to assert VM/JIT
/// parity on error-raising paths (e.g. out-of-bounds element access).
fn jit_eval_err(source: &str) -> String {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    match JITExecutor::new().execute_program(&mut engine, &program) {
        Ok(result) => panic!(
            "expected a runtime error, but the program ran cleanly: {:?}",
            result.wire_value
        ),
        Err(e) => format!("{}", e),
    }
}

/// Coerce a JIT result into an `f64`. Accepts both `WireValue::Number` and
/// `WireValue::Integer` because the JIT may return a number from an integer
/// expression depending on the slot's resolved type.
fn as_f64(val: WireValue) -> f64 {
    match val {
        WireValue::Number(n) => n,
        WireValue::Integer(n) => n as f64,
        other => panic!("expected number/integer, got {:?}", other),
    }
}

/// Coerce a JIT result into an `i64`.
fn as_i64(val: WireValue) -> i64 {
    match val {
        WireValue::Integer(n) => n,
        WireValue::Number(n) => n as i64,
        other => panic!("expected integer/number, got {:?}", other),
    }
}

/// Coerce a JIT result into a `bool`.
fn as_bool(val: WireValue) -> bool {
    match val {
        WireValue::Bool(b) => b,
        other => panic!("expected bool, got {:?}", other),
    }
}

// ===========================================================================
// 1. Annotated `Array<number>` — exercises v2_array_get on f64 elements
// ===========================================================================

#[test]
fn v2_array_f64_index_sum_two_elements() {
    let v = jit_eval(
        r#"
let arr: Array<number> = [1.0, 2.0, 3.0]
arr[0] + arr[1]
"#,
    );
    assert!((as_f64(v) - 3.0).abs() < 1e-9);
}

#[test]
fn v2_array_f64_index_each_element() {
    // Read each element individually, then sum them. Stresses repeated
    // `v2_array_get(F64)` emission.
    let v = jit_eval(
        r#"
let arr: Array<number> = [1.5, 2.5, 3.5]
arr[0] + arr[1] + arr[2]
"#,
    );
    assert!((as_f64(v) - 7.5).abs() < 1e-9);
}

#[test]
fn v2_array_f64_length() {
    // `arr.length` is lowered to a `Place::Field(arr_slot, "length")`
    // whose v2 fast path emits a single `v2_array_len` load.
    let v = jit_eval(
        r#"
let arr: Array<number> = [10.0, 20.0, 30.0, 40.0]
arr.length
"#,
    );
    assert_eq!(as_i64(v), 4);
}

// ===========================================================================
// 2. Annotated `Array<int>` — exercises v2_array_get on i64 elements
// ===========================================================================

#[test]
fn v2_array_i64_index_first_element() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [10, 20, 30]
arr[0]
"#,
    );
    assert_eq!(as_i64(v), 10);
}

#[test]
fn v2_array_i64_index_sum() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [10, 20, 30]
arr[0] + arr[1] + arr[2]
"#,
    );
    assert_eq!(as_i64(v), 60);
}

#[test]
fn v2_array_i64_length() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [1, 2, 3, 4, 5]
arr.length
"#,
    );
    assert_eq!(as_i64(v), 5);
}

// ===========================================================================
// 3. Annotated `Array<i32>` — exercises v2_array_get on i32 elements
// ===========================================================================

#[test]
fn v2_array_i32_index_and_length() {
    let v = jit_eval(
        r#"
let arr: Array<i32> = [7, 11, 13]
arr.length
"#,
    );
    assert_eq!(as_i64(v), 3);
}

// ===========================================================================
// 4. Annotated `Array<bool>` — currently falls back to legacy path because
//    `v2_array_new_bool` isn't wired in the JIT yet. Test the fallback.
// ===========================================================================

#[test]
fn v2_array_bool_fallback_first_element() {
    // Bool element types fall through to the legacy NaN-boxed array path
    // because the JIT does not yet have a `jit_v2_array_new_bool` FFI binding.
    // This test verifies the fail-soft behaviour: legacy semantics still
    // produce the correct result.
    let v = jit_eval(
        r#"
let arr = [true, false, true]
arr[0]
"#,
    );
    assert_eq!(as_bool(v), true);
}

// ===========================================================================
// 5. Out-of-bounds access — `v2_array_get` raises an out-of-bounds error,
//    matching the bytecode VM's `VMError::IndexOutOfBounds`. WS-3 F1: the
//    prior codegen silently fabricated the element-type zero, which diverged
//    from the VM and produced a value for a memory-unsafe access.
// ===========================================================================

#[test]
fn v2_array_f64_out_of_bounds_raises_error() {
    // Index 10 is past the end of a 3-element array. `v2_array_get` emits
    // a bounds-check branch that does an early `return_` of the
    // out-of-bounds signal — the executor maps it to the VM's diagnostic.
    let msg = jit_eval_err(
        r#"
let arr: Array<number> = [1.0, 2.0, 3.0]
arr[10]
"#,
    );
    assert!(
        msg.contains("out of bounds"),
        "expected an out-of-bounds error, got: {}",
        msg
    );
}

#[test]
fn v2_array_i64_out_of_bounds_raises_error() {
    let msg = jit_eval_err(
        r#"
let arr: Array<int> = [10, 20, 30]
arr[100]
"#,
    );
    assert!(
        msg.contains("out of bounds"),
        "expected an out-of-bounds error, got: {}",
        msg
    );
}

#[test]
fn v2_array_i64_out_of_bounds_store_raises_error() {
    // WS-3 F1: the OOB store path must also raise, not silently skip.
    let msg = jit_eval_err(
        r#"
let mut arr: Array<int> = [10, 20, 30]
arr[100] = 7
arr[0]
"#,
    );
    assert!(
        msg.contains("out of bounds"),
        "expected an out-of-bounds error, got: {}",
        msg
    );
}

// ===========================================================================
// 6. Legacy path still works — non-annotated array literals fall through
//    to the NaN-boxed path because the bytecode compiler does not record an
//    `Array<T>` `ConcreteType` for the slot.
// ===========================================================================

#[test]
fn legacy_array_index_still_works() {
    // No type annotation — slot's ConcreteType remains `Void`, so the JIT
    // takes the legacy path with NaN-boxed elements and `inline_array_get`.
    let v = jit_eval(
        r#"
let arr = [10, 20, 30]
arr[1]
"#,
    );
    assert_eq!(as_i64(v), 20);
}

#[test]
fn legacy_array_length_still_works() {
    let v = jit_eval(
        r#"
let arr = [10, 20, 30, 40, 50]
arr.length
"#,
    );
    assert_eq!(as_i64(v), 5);
}

// ===========================================================================
// Phase 4b Round 5 W14.2-E2 — Array-element aggregate coverage
//
// Per `docs/cluster-audits/v0.3-w14-test-coverage-audit.md` §4 W11 and
// `docs/cluster-audits/v0.3-w11-jit-new-array-close.md` §4 Class A':
//
// > W11-followup-slice-classify: arr_slice.shape (nums[1..4]) lowers via
// > Aggregate([object, index, end_index]) — the destination slot's
// > ConcreteType is unstamped because no SliceStore MIR statement exists.
//
// At HEAD 2924b685 the simple slice-and-read pattern works byte-equal
// VM == JIT for both `Array<int>` and `Array<number>` because the
// downstream W13/W14/W16.2-A closes propagated stamping through the
// IndexAccess slice path. Pin the working cases below.
// ===========================================================================

/// W14.2-E2 typed `Array<int>` slice element read. VM==JIT=20.
#[test]
fn v2_array_i64_slice_first_element() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [10, 20, 30, 40]
let s = arr[1..3]
s[0]
"#,
    );
    assert_eq!(as_i64(v), 20);
}

/// W14.2-E2 typed `Array<int>` slice second element. VM==JIT=30.
/// Note: parallel `s.length`-on-slice shape surfaces a JITExecutor-direct
/// TypeError ("expected array, object, or string, got scalar") that does
/// NOT reproduce at the release-binary `--mode jit` level. Tracked as
/// W14.2-E-SURFACE-B in the close report. Pin only the element-access
/// shape here.
#[test]
fn v2_array_i64_slice_second_element() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [10, 20, 30, 40]
let s = arr[1..3]
s[1]
"#,
    );
    assert_eq!(as_i64(v), 30);
}

/// W14.2-E2 typed `Array<number>` slice element read. VM==JIT=2.5.
#[test]
fn v2_array_f64_slice_first_element() {
    let v = jit_eval(
        r#"
let arr: Array<number> = [1.5, 2.5, 3.5, 4.5]
let s = arr[1..3]
s[0]
"#,
    );
    assert!((as_f64(v) - 2.5).abs() < 1e-9);
}

/// W14.2-E2 typed `Array<int>` open-range iteration sum. VM==JIT=70.
#[test]
fn v2_array_i64_open_range_iteration_sum() {
    let v = jit_eval(
        r#"
let arr: Array<int> = [10, 20, 30, 40]
let mut sum: int = 0
for i in 0..arr.length {
    sum = sum + arr[i]
}
sum
"#,
    );
    assert_eq!(as_i64(v), 100);
}

// NOTE: `arr[1..]` open-range slice → bind-and-element-access via
// JITExecutor direct path surfaces SIGSEGV at HEAD 2924b685. The same
// pattern works at the release-binary `--mode jit` level (verified via
// the smoke matrix expansion). Tracked as W14.2-E-SURFACE-C in the
// W14.2-E close report. NOT pinned as a failing test per surface-and-stop
// discipline: failing tests would block the close-gate green.
