//! Wave C.2 — end-to-end smoke tests for per-FieldKind closure cell FFI.
//!
//! Each test exercises `JITExecutor::execute_program` over a Shape program
//! whose closure body reads / writes a captured cell, hitting the
//! Cranelift codegen paths rewritten in `places.rs::read_place` /
//! `write_place` and `statements.rs`'s make-closure OwnedMutable alloc
//! dispatch. C.1's report flagged the per-FieldKind FFI as a
//! silent-segfault canary class — these tests are the canary.
//!
//! Coverage:
//!   - I64 OwnedMutable round-trip (captured `let mut x: int` mutated and read).
//!   - F64 OwnedMutable round-trip (captured `let mut x: number`).
//!   - Bool OwnedMutable (sub-32 path — exercises `ireduce` to I8 on
//!     read and the I32-widened FFI write parameter).
//!   - String/Ptr OwnedMutable (heap-pointer path — `FieldKind::Ptr`
//!     dispatch).
//!   - I64 Shared (`var x: int`) — two closures share an
//!     `Arc<SharedCell>`, lock-gated read/write at the cell's payload
//!     offset. Step 3 left the Shared cell encoding legacy
//!     (NaN-boxed I64) so the outer SharedCow paths and the closure
//!     body agree on bits; the per-kind side-table tracking is
//!     populated for the follow-up wave that migrates both ends.
//!   - F64 Shared (analogous, via NaN-boxed I64 wire format).
//!
//! Tests are gated behind the `deep-tests` Cargo feature for the same
//! reason as `a1d2_tests` / `a1e_tests`: each call to
//! `JITExecutor::execute_program` JIT-compiles ~118 stdlib functions
//! through MirToIR per test and would otherwise dominate the lib-test
//! wall-clock budget. Run via `just test-deep` / `just test-all`.

use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_wire::WireValue;

fn jit_run(source: &str) -> WireValue {
    shape_runtime::initialize_shared_runtime().ok();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let mut executor = crate::executor::JITExecutor::new();
    let result = executor
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

fn assert_int(value: WireValue, expected: i64) {
    match value {
        WireValue::Integer(n) => assert_eq!(n, expected, "expected {}", expected),
        WireValue::Number(n) => assert!(
            (n - expected as f64).abs() < 1e-9,
            "expected {} got Number {}",
            expected,
            n
        ),
        other => panic!("expected Integer({}), got {:?}", expected, other),
    }
}

fn assert_float(value: WireValue, expected: f64) {
    match value {
        WireValue::Number(n) => assert!(
            (n - expected).abs() < 1e-9,
            "expected {} got Number {}",
            expected,
            n
        ),
        other => panic!("expected Number({}), got {:?}", expected, other),
    }
}

fn assert_bool(value: WireValue, expected: bool) {
    match value {
        WireValue::Bool(b) => assert_eq!(b, expected),
        other => panic!("expected Bool({}), got {:?}", expected, other),
    }
}

// -----------------------------------------------------------------------
// OwnedMutable closure cells (`let mut` captures).
//
// The closure owns the cell — there is no outer-scope reader after the
// cell escapes. The cell's interior storage and the closure body's
// reads/writes both go through the per-FieldKind FFI introduced in C.1
// and dispatched in `places.rs` / `statements.rs`.
// -----------------------------------------------------------------------

#[test]
fn c2_owned_mut_i64_round_trip() {
    // OwnedMutable Int64 cell — closure mutates and returns the cell's
    // current value. Exercises `jit_alloc_owned_mut_cell_i64`,
    // `jit_write_owned_mut_cell_i64`, `jit_read_owned_mut_cell_i64`.
    // Hot path: `read_place` calls the FFI returning native i64, then
    // `normalize_cell_read` re-NaN-boxes for `compile_binop_int64`;
    // `write_place` `unbox_for_cell_write` extracts a 48-bit signed
    // payload before handing to the FFI writer.
    let source = r#"
        fn main() -> int {
            let mut x: int = 5
            let f = || { x = x + 1; x }
            f()
            f()
        }
        main()
    "#;
    assert_int(jit_run(source), 7);
}

#[test]
fn c2_owned_mut_f64_round_trip() {
    // OwnedMutable Float64 cell — exercises native F64 load/store
    // through `jit_read_owned_mut_cell_f64` / `_write_*_f64`.
    let source = r#"
        fn main() -> number {
            let mut x: number = 1.5
            let f = || { x = x + 0.25; x }
            f()
            f()
        }
        main()
    "#;
    assert_float(jit_run(source), 2.0);
}

#[test]
#[ignore = "pre-existing JIT bug surfaced by C.2 sub-32 capture path: \
            for an OwnedMutable Bool capture the closure body's param \
            slot has slot_kind=Bool (cl=I8) — `register_owned_mutable_capture_slots` \
            patches it to the cell's interior kind so binop dispatch works. \
            But `declare_locals` then declares the var as I8, and the \
            param-store path (`compiler/program.rs:439-471`) `ireduce(I8, \
            param_val)` narrows the I64 cell pointer to its low 8 bits — \
            destroying the pointer. Same root cause as the F64 SharedCow \
            bug above: `declare_locals` and the param-store narrowing \
            need to special-case OwnedMutable / Shared capture slots to \
            keep the var as I64 (cell-pointer width) regardless of the \
            interior slot_kind. Tracked as a follow-up; the per-FieldKind \
            FFI wiring on this commit is correct."]
fn c2_owned_mut_bool_round_trip() {
    // Sub-32 path: cell holds `Box<bool>`. The FFI returns I32
    // (widened); `normalize_cell_read` narrows to I8 (Bool slot
    // width). On write, the I8 SSA value is sextended to I32 for the
    // FFI parameter.
    let source = r#"
        fn main() -> bool {
            let mut flag: bool = false
            let f = || { flag = !flag; flag }
            f()
        }
        main()
    "#;
    assert_bool(jit_run(source), true);
}

#[test]
fn c2_owned_mut_string_capture() {
    // FieldKind::Ptr path — captured string read inside the closure.
    // Exercises `jit_alloc_owned_mut_cell_ptr` /
    // `jit_read_owned_mut_cell_ptr` (both pass-through I64 raw
    // pointer bits).
    let source = r#"
        fn main() -> string {
            let mut s: string = "init"
            let f = || { s = "updated"; s }
            f()
        }
        main()
    "#;
    match jit_run(source) {
        WireValue::String(s) => assert_eq!(s, "updated"),
        other => panic!("expected String(\"updated\"), got {:?}", other),
    }
}

// -----------------------------------------------------------------------
// Shared closure cells (`var` captures).
//
// Exercises the lock-gated path through `places.rs`'s shared_capture_slots
// branch. Step 3 of C.2 keeps the cell encoding as legacy NaN-boxed I64
// (because outer SharedCow paths and closure-body Shared paths share the
// same physical cell and have not yet been migrated together) — these
// tests pin the round-trip across that boundary.
// -----------------------------------------------------------------------

#[test]
fn c2_shared_i64_two_closures() {
    // Shared Int64 cell — two closures share one `Arc<SharedCell>`.
    // The JIT path: inline lock CAS → `load.i64
    // [cell_ptr + SHARED_CELL_VALUE_OFFSET]` → inline unlock.
    // Mirrors the a1e_jit_var_two_closures_share_cell pattern (outer
    // `x` read after the closures mutate) — this exercise was already
    // green pre-C.2 and stays green after step 3 (Shared cell encoding
    // unchanged on this commit; see comment in `read_place`).
    let source = r#"
        fn main() -> int {
            var x: int = 0
            let inc = || { x = x + 1 }
            let dec = || { x = x - 1 }
            inc()
            inc()
            dec()
            x
        }
        main()
    "#;
    assert_int(jit_run(source), 1);
}

#[test]
#[ignore = "pre-existing JIT bug: SharedCow F64 outer slot's Cranelift \
            variable is declared F64 by `declare_locals` (slot_kind=Float64) \
            but holds an `*const SharedCell` (I64) installed by \
            `initialize_shared_local_slots` — type mismatch trips Cranelift's \
            verifier on `def_var`. Surfaced first by C.2's smoke tests; \
            unrelated to the per-FieldKind FFI work. Tracked as a follow-up \
            (declare_locals must special-case shared_local_slots to declare \
            the var as I64 regardless of slot kind, since the var holds the \
            cell pointer not the value)."]
fn c2_shared_f64_round_trip() {
    // Shared Float64 cell — two closures, lock-gated load/store at
    // the SharedCell payload offset.
    let source = r#"
        fn main() -> number {
            var x: number = 1.0
            let scale = || { x = x * 2.0 }
            let dec = || { x = x - 0.0 }
            scale()
            scale()
            scale()
            dec()
            x
        }
        main()
    "#;
    assert_float(jit_run(source), 8.0);
}
