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
//! ## 7b — closure-capture array refcount imbalance → SIGABRT
//!
//! `let data = [1,2,3]; let g = || data.sum(); g(); g(); ...` — the VM
//! prints `6` every call; the JIT SIGABRTed (`malloc(): unaligned
//! tcache chunk`, ec=134) on the 3rd+ call.
//!
//! Root cause: a per-call refcount imbalance on the closure-captured
//! array. `jit_call_value`'s `Ptr(HeapKind::Closure)` dispatch arm
//! extracted each capture via `block.read_capture_kinded(idx)` — a RAW
//! bit read that does NOT bump the refcount — and handed the bits to
//! `jit_trampoline_call_closure`. That trampoline materialises a FRESH
//! `OwnedClosureBlock` from those bits and the fresh block's `Drop`
//! (`release_typed_closure`) releases each heap capture per the layout
//! capture masks. Its doc-comment states the contract: "the JIT
//! pre-incremented each share before crossing the FFI boundary." The
//! JIT did NOT. So every closure call retired one share of the
//! captured array; after the original binding's + the closure block's
//! shares were consumed the next access dereferenced freed memory.
//!
//! Fix (producer-side, balanced): `jit_call_value` retains each
//! heap-typed capture via the kind-driven `KindedSlot::clone` dispatch
//! before handing it to the trampoline — the same dispatch table as the
//! VM's per-capture `clone_with_kind` at `call_convention.rs:776`.
//! Retain-on-extract + release-at-fresh-block-drop nets to zero per
//! call; the closure is callable any number of times.
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
// 7b — closure-capture array refcount balance
// ═══════════════════════════════════════════════════════════════════════

/// Primary 7b gate: a closure that captures an array, called five times.
/// Pre-fix the JIT SIGABRTed (`malloc(): unaligned tcache chunk`,
/// ec=134) on the 3rd call — a per-call refcount imbalance freed the
/// captured array's allocation under the still-live closure. Post-fix
/// `jit_call_value` retains each heap capture per call, balancing the
/// fresh trampoline block's release; the closure is callable any number
/// of times. The final `g()` returns `6` (`1 + 2 + 3`).
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
/// Each call must balance the captured array's refcount — a leak or an
/// over-release accumulates and either grows the refcount unbounded or
/// (the observed failure) frees the allocation mid-program. The loop
/// body returns the running sum; the final result pins the closure
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
/// failure threshold of the primary gate. The imbalance was in
/// `jit_call_value`'s capture extraction (one missing retain per call);
/// ten calls without it would over-release the captured array four
/// times past free. Post-fix the closure stays callable.
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
/// called repeatedly. Confirms per-capture retain accounting is keyed
/// to the right allocation — a cross-capture imbalance would corrupt
/// one array's refcount while the other stays balanced.
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
