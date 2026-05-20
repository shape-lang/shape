//! γ-CP4 jit-makefieldref regression tests.
//!
//! Pins the JIT codegen for `MakeFieldRef` — `&`/`&mut` references that
//! project into a typed-object field (`&mut b.value`) — per ADR-006
//! §2.7.13 (`RefTarget`) + §2.3 (typed-Arc carrier). v0.3-gating per the
//! NO-KNOWN-INCORRECTNESS program.
//!
//! ## Background
//!
//! `RefTarget::TypedField` (`crates/shape-value/src/reference.rs`) is the
//! VM's carrier for a reference into a typed-object field: a v2-raw
//! `TypedObjectPtr` receiver + a `field_offset` + the projected slot's
//! `NativeKind`. The VM resolves/reads/writes it via
//! `resolve_typed_object_receiver` / `read_ref_target` / `write_ref_target`
//! in `crates/shape-vm/src/executor/variables/mod.rs`.
//!
//! Pre-fix, the JIT had NO `RefTarget`-aware deref. `MakeFieldRef`
//! appeared only in `optimizer/escape_analysis.rs` bookkeeping; the
//! `mir_compiler` `Rvalue::Borrow` handler routed every non-ref-param
//! borrow — `Place::Field` projections included — through the
//! per-function stack-cell path. That path:
//!
//!   1. reads the field VALUE into a fresh Cranelift stack cell, and
//!   2. registers the cell in `ref_stack_slots` keyed on
//!      `place.root_local()` — for `&mut b.value` the root local is the
//!      *struct* `b`, not the field.
//!
//! `reload_referenced_locals` then writes the cell contents (a field
//! scalar) back into `b`'s slot variable after every call, overwriting
//! the `TypedObject` pointer with an integer. The next field access
//! (`inline_typed_field_get`) dereferenced that integer-as-pointer →
//! deterministic SIGSEGV (`ec=139`) under `--mode jit`, while the VM
//! returned the correct result.
//!
//! ## The fix
//!
//! `Rvalue::Borrow` now compiles a `Place::Field` projection to the
//! **address of the field slot** inside the live object (`places.rs`
//! `emit_typed_field_address`: `TypedObject*` base + `TYPED_OBJ_HEADER`
//! + field byte offset) — the JIT analogue of the VM's
//! `TypedObjectStorage` base + `field_offset`. No throwaway cell is
//! allocated, so `ref_stack_slots` gains no entry and
//! `reload_referenced_locals` never clobbers the struct local. Deref
//! load/store through the field address mutates the field in place,
//! byte-equal to the VM.
//!
//! This is FULL JIT codegen — both the toplevel borrow site and the
//! `bump` ref-param callee JIT-compile via MirToIR (verified with
//! `--trace-jit=shape_jit=debug`: "jit-mir compiled top-level code" +
//! "jit-mir compiled function ... bump"), no deopt-to-interpreter.
//!
//! Sister-class to `ref_param_regression_tests` (the `Place::Local`
//! ref-param chain) — this module covers the `Place::Field` projection.
//!
//! Gated behind `deep-tests` for the same reason as
//! `ref_param_regression_tests`: `JITExecutor::execute_program`
//! JIT-compiles the stdlib on every test, so default-parallelism CI
//! runs would race the JIT code cache.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

fn jit_expect_int(source: &str, expected: i64) {
    match jit_eval(source) {
        WireValue::Integer(n) => {
            assert_eq!(n, expected, "Expected integer {}, got {}", expected, n);
        }
        WireValue::Number(n) => {
            assert!(
                (n - expected as f64).abs() < 1e-9,
                "Expected integer {} (got Number {})",
                expected,
                n
            );
        }
        other => panic!("Expected Integer({}), got {:?}", expected, other),
    }
}

fn jit_expect_number(source: &str, expected: f64) {
    match jit_eval(source) {
        WireValue::Number(n) => {
            assert!(
                (n - expected).abs() < 1e-9,
                "Expected number {}, got {}",
                expected,
                n
            );
        }
        WireValue::Integer(n) => {
            assert!(
                (n as f64 - expected).abs() < 1e-9,
                "Expected number {} (got Integer {})",
                expected,
                n
            );
        }
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

fn jit_expect_bool(source: &str, expected: bool) {
    match jit_eval(source) {
        WireValue::Bool(b) => {
            assert_eq!(b, expected, "Expected bool {}, got {}", expected, b);
        }
        other => panic!("Expected Bool({}), got {:?}", expected, other),
    }
}

/// Primary regression gate: `&mut` into an `int` field, mutated through
/// the borrowed cell by a ref-param callee.
///
/// Pre-fix: deterministic SIGSEGV (`ec=139`) under `--mode jit` — the
/// throwaway stack cell snapshotted the field value and
/// `reload_referenced_locals` overwrote `b`'s `TypedObject` pointer with
/// the field integer, so `print(b.value)`'s `inline_typed_field_get`
/// dereferenced an integer-as-pointer. The VM returned `11`.
///
/// Post-fix: the borrow compiles to the field slot address; `bump`
/// mutates the field in place; `b.value` reads `11`. VM == JIT.
#[test]
fn jit_field_ref_mut_int_mutates_in_place() {
    jit_expect_int(
        r#"
type Box { value: int }
fn bump(&mut r) { r = r + 1 }
let mut b = Box { value: 10 }
bump(&mut b.value)
b.value
"#,
        11,
    );
}

/// `&mut` field reference on a `number` field. Exercises the F64 field
/// slot — the field address points at the same 8-byte slot regardless
/// of the field's `NativeKind`.
#[test]
fn jit_field_ref_mut_number_mutates_in_place() {
    jit_expect_number(
        r#"
type Pt { x: number }
fn bump(&mut r) { r = r + 1.0 }
let mut p = Pt { x: 5.0 }
bump(&mut p.x)
p.x
"#,
        6.0,
    );
}

/// Shared (`&`, immutable) field reference — read-through only. The
/// callee reads the borrowed field value without mutating it.
#[test]
fn jit_field_ref_shared_read_through() {
    jit_expect_int(
        r#"
type Box { value: int }
fn readit(&r) -> int { r }
let b = Box { value: 77 }
readit(&b.value)
"#,
        77,
    );
}

/// The original field-ref `&mut` mutation must NOT leak into sibling
/// fields. Pins that the field address targets exactly the projected
/// slot (`TYPED_OBJ_HEADER` + the field's byte offset), not slot 0.
#[test]
fn jit_field_ref_mut_targets_correct_slot() {
    jit_expect_int(
        r#"
type Rec { a: int, b: int, c: int }
fn bump(&mut r) { r = r + 100 }
let mut rec = Rec { a: 1, b: 2, c: 3 }
bump(&mut rec.b)
rec.a + rec.b + rec.c
"#,
        // a=1 unchanged, b=2+100=102, c=3 unchanged → 106
        106,
    );
}

/// A field reference into the LAST field of a multi-field struct —
/// catches an off-by-one in the field byte-offset → field-address
/// computation.
#[test]
fn jit_field_ref_mut_last_field() {
    jit_expect_int(
        r#"
type Rec { a: int, b: int, c: int }
fn bump(&mut r) { r = r + 7 }
let mut rec = Rec { a: 1, b: 2, c: 3 }
bump(&mut rec.c)
rec.c
"#,
        10,
    );
}

/// Sequential `&mut` field-ref calls compose: three mutations through
/// the same field must net +3. Pins that the field address is stable
/// across calls and that `reload_referenced_locals` does NOT corrupt
/// the struct local between calls (the pre-fix SIGSEGV path).
#[test]
fn jit_field_ref_sequential_calls_compose() {
    jit_expect_int(
        r#"
type Box { value: int }
fn bump(&mut r) { r = r + 1 }
let mut b = Box { value: 0 }
bump(&mut b.value)
bump(&mut b.value)
bump(&mut b.value)
b.value
"#,
        3,
    );
}

/// `&mut` field reference on a `bool` field.
#[test]
fn jit_field_ref_mut_bool_field() {
    jit_expect_bool(
        r#"
type Flag { on: bool }
fn flip(&mut r) { r = !r }
let mut f = Flag { on: false }
flip(&mut f.on)
f.on
"#,
        true,
    );
}

/// A field-ref `&mut` followed by a plain (non-ref) field write — the
/// field write must observe the ref-mutated value. Pins that the field
/// memory is shared between the field-address ref path and the inline
/// `typed_object_set_field` path.
#[test]
fn jit_field_ref_then_plain_field_access() {
    jit_expect_int(
        r#"
type Box { value: int }
fn bump(&mut r) { r = r + 5 }
let mut b = Box { value: 10 }
bump(&mut b.value)
b.value = b.value * 2
b.value
"#,
        // 10 -> +5 -> 15 -> *2 -> 30
        30,
    );
}
