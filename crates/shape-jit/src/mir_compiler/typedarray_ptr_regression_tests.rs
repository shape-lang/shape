//! γ-CP5 jit-typedarray-ptr regression tests.
//!
//! Pins two JIT bugs un-masked by the Family-α TypedArray fix, both
//! v0.3-gating under the NO-KNOWN-INCORRECTNESS program.
//!
//! ## 7a — struct-field array indexed wrong element
//!
//! A struct field of type `Array<int>` carries a v2 `TypedArray<T>`
//! pointer (data@8 / len@16 layout). Pre-fix, the JIT `Place::Index`
//! codegen's v2 fast path (`v2_typed_array_elem_kind`) recognised ONLY
//! `Place::Local` bases — a field-projected base (`b.items[i]`,
//! `Place::Field`) returned `None` and fell through to the legacy
//! `inline_array_get` which uses the v1 array layout (data@+0 / len@+8
//! past an 8-byte header). The wrong byte offset returned the wrong
//! element — `b.items[i]` always read element 0's bits.
//!
//! Fix: `v2_typed_array_elem_kind` recognises a `Place::Field` base via
//! the schema-derived `field_array_elem_kinds` map (stamped at
//! `populate_field_byte_offsets_from_schemas` time from the canonical
//! `TypeSchemaRegistry` `FieldType::Array(elem)` declaration). The
//! field-projected v2 array then uses the correct v2 layout.
//!
//! ## 7b — closure-capture ownership mismatch → SIGABRT
//!
//! `let data = [1,2,3]; let g = || data.sum(); g(); g(); ...` — the VM
//! prints `6` every call; the JIT SIGABRTed (`malloc(): unaligned
//! tcache chunk`, ec=134) on the 3rd+ call.
//!
//! Root cause: a per-call ownership mismatch on closure captures.
//! `jit_call_value`'s `Ptr(HeapKind::Closure)` dispatch arm extracted
//! raw capture bits and handed them to `jit_trampoline_call_closure`.
//! That trampoline materialises a fresh owning `OwnedClosureBlock` and
//! drops it after the call. For heap captures that required a matching
//! retain; for cell-storage captures (`OwnedMutable` / `Shared`) there is
//! no legal clone of the raw cell pointer, so the same shape can double
//! free the original cell.
//!
//! Fix: raw-Arc closure calls now borrow the existing `OwnedClosureBlock`
//! into `VirtualMachine::execute_closure`. The VM frame setup clones only
//! the immutable captures it installs into locals, while owned/shared
//! cell pointers remain owned by the original closure block.
//!
//! Gated behind `deep-tests` for the same reason as
//! `closure_dispatch_regression_tests` / `field_ref_regression_tests`:
//! `JITExecutor::execute_program` JIT-compiles the stdlib on every
//! test, so default-parallelism CI runs would race the JIT code cache.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

/// Run a Shape program through the full JIT pipeline and return the
/// raw `WireValue` result.
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

// ═══════════════════════════════════════════════════════════════════════
// 7a — struct-field array indexed access
// ═══════════════════════════════════════════════════════════════════════

/// Primary 7a gate: a struct field of type `Array<int>`, indexed inside
/// a function. Pre-fix every index returned element 0 (`10`) because the
/// `Place::Field` base fell through to the v1-layout `inline_array_get`.
/// Post-fix each index returns the correct element. The function `get`
/// JIT-compiles via MirToIR (the `Place::Field` base inside `b.items[i]`).
#[test]
fn jit_struct_field_array_index_0() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn get(b: Bag, i: int) -> int { return b.items[i] }
let b = Bag { items: [10, 20, 30, 40, 50] }
get(b, 0)
"#,
        10,
    );
}

#[test]
fn jit_struct_field_array_index_1() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn get(b: Bag, i: int) -> int { return b.items[i] }
let b = Bag { items: [10, 20, 30, 40, 50] }
get(b, 1)
"#,
        20,
    );
}

#[test]
fn jit_struct_field_array_index_2() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn get(b: Bag, i: int) -> int { return b.items[i] }
let b = Bag { items: [10, 20, 30, 40, 50] }
get(b, 2)
"#,
        30,
    );
}

#[test]
fn jit_struct_field_array_index_3() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn get(b: Bag, i: int) -> int { return b.items[i] }
let b = Bag { items: [10, 20, 30, 40, 50] }
get(b, 3)
"#,
        40,
    );
}

#[test]
fn jit_struct_field_array_index_4() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn get(b: Bag, i: int) -> int { return b.items[i] }
let b = Bag { items: [10, 20, 30, 40, 50] }
get(b, 4)
"#,
        50,
    );
}

/// 7a: every distinct index summed across the array confirms each
/// element is read from the correct offset — not just one fixed slot.
/// Pre-fix this returned `50` (`10 * 5`, element 0 read five times via
/// the v1-layout `inline_array_get`). The per-index `let a: int = ...`
/// annotation keeps the `Place::Field` index reads in their own
/// statements (a bare chained `b.items[0] + b.items[1]` trips a
/// pre-existing inference limitation on nested field-index expressions
/// — orthogonal to this CP).
#[test]
fn jit_struct_field_array_index_sum_all() {
    jit_expect_int(
        r#"
type Bag { items: Array<int> }
fn sum_all(b: Bag) -> int {
    let a: int = b.items[0]
    let c: int = b.items[1]
    let d: int = b.items[2]
    let e: int = b.items[3]
    let f: int = b.items[4]
    return a + c + d + e + f
}
let b = Bag { items: [10, 20, 30, 40, 50] }
sum_all(b)
"#,
        150,
    );
}

// ═══════════════════════════════════════════════════════════════════════
// 7b — closure-capture array ownership balance
// ═══════════════════════════════════════════════════════════════════════

/// Primary 7b gate: a closure that captures an array, called five times.
/// Pre-fix the JIT SIGABRTed (`malloc(): unaligned tcache chunk`,
/// ec=134) on the 3rd call because raw capture bits were copied into a
/// fresh owning trampoline block. Post-fix `jit_call_value` borrows the
/// existing `OwnedClosureBlock` into the VM; frame setup performs the
/// normal kinded capture cloning for immutable values, and the closure is
/// callable any number of times. The final `g()` returns `6` (`1 + 2 + 3`).
#[test]
fn jit_closure_capture_array_called_five_times() {
    jit_expect_int(
        r#"
let data = [1, 2, 3]
let g = || data.sum()
g()
g()
g()
g()
g()
"#,
        6,
    );
}

/// 7b stress: the same closure called many times in a loop (≥10 calls).
/// Each call must borrow the original closure block rather than
/// constructing a new owner for its raw captures; otherwise repeated calls
/// accumulate ownership damage and eventually free a live allocation. The
/// loop body returns the running sum; the final result pins the closure
/// still produces `6` after 20 calls.
#[test]
fn jit_closure_capture_array_called_in_loop() {
    jit_expect_int(
        r#"
let data = [10, 20, 30]
let g = || data.sum()
let mut total = 0
let mut i = 0
while i < 20 {
    total = g()
    i = i + 1
}
total
"#,
        60,
    );
}

/// 7b: a closure capturing an array, called ten times — twice the
/// failure threshold of the primary gate. The bug was in
/// `jit_call_value`'s capture extraction: it rebuilt ownership from raw
/// bits instead of borrowing the existing closure block. Post-fix the
/// closure stays callable.
#[test]
fn jit_closure_capture_array_called_ten_times() {
    jit_expect_int(
        r#"
let data = [7, 8, 9]
let g = || data.sum()
g()
g()
g()
g()
g()
g()
g()
g()
g()
g()
"#,
        24,
    );
}

/// 7b: two distinct arrays captured by two distinct closures, each
/// called repeatedly. Confirms borrowed-block dispatch keeps each
/// closure's captures tied to its original allocation — a cross-capture
/// ownership mismatch would corrupt one captured array while the other
/// stays balanced.
#[test]
fn jit_two_closures_capture_distinct_arrays() {
    jit_expect_int(
        r#"
let a = [1, 1, 1]
let b = [100, 100, 100, 100]
let ga = || a.sum()
let gb = || b.sum()
ga()
gb()
ga()
gb()
ga()
gb()
ga() + gb()
"#,
        403,
    );
}
