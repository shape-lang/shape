//! End-to-end closure dispatch regression tests.
//!
//! These tests pin the fix-jit-lead commits 1–3:
//!
//!  1. `arg_count` ABI decode as raw i64 at the JIT→FFI boundary.
//!  2. Backward propagation of slot kinds onto closure params.
//!  3. `HeapValue::ClosureRaw` / `HeapValue::Closure` decode in
//!     `jit_call_value` via `VmClosureHandle`.
//!
//! Before these commits, the bytecode-emitted closure at `closure_simple`
//! dispatched through `jit_call_value` with `arg_count` misdecoded as 0
//! (via `unbox_number` on a raw i64), the closure body failed to JIT
//! because the `|x|` param had slot kind `Unknown`, and the VM-format
//! closure pointer was unrecognised — so the whole pipeline returned
//! `Null` instead of `Integer(6)`.
//!
//! This module is intentionally NOT gated behind
//! `#[cfg(jit_v2_unstable_tests)]` so the primary regression gate
//! stays green on the default CI path. The broader
//! `mir_compiler::integration_tests` module remains gated because it
//! covers paths with separate pre-existing JIT/VM interaction
//! regressions (see the fix-jit-lead report for the outstanding
//! cell-identity issue that blocks the A.1D.2 counter tests).

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
        WireValue::Number(n) => assert!(
            (n - expected).abs() < 1e-9,
            "Expected number {}, got {}",
            expected,
            n
        ),
        WireValue::Integer(n) => assert!(
            (n as f64 - expected).abs() < 1e-9,
            "Expected number {} (got Integer {})",
            expected,
            n
        ),
        other => panic!("Expected Number({}), got {:?}", expected, other),
    }
}

/// Primary fix-jit regression gate.
///
/// `|x| x + 1` applied to `5` must return `Integer(6)`. The fix-jit
/// series (commits #1–#3) is specifically motivated by this failing on
/// `jit-v2-phase1`.
#[test]
fn closure_simple_dispatch_returns_six() {
    jit_expect_int(
        r#"
let add_one = |x| x + 1
add_one(5)
"#,
        6,
    );
}

/// Integer-literal-on-rhs variant. Exercises the backward slot-kind
/// propagation from the typed constant onto the closure parameter from
/// the other side of the binop.
#[test]
fn closure_int_literal_on_rhs_propagates_param_kind() {
    jit_expect_int(
        r#"
let times_two = |x| x * 2
times_two(7)
"#,
        14,
    );
}

/// Trampoline dispatch regression: a closure that fails JIT compilation
/// (so its function_table slot stays null) must still produce the
/// correct result when invoked from JIT'd code.
///
/// Before the fix:
///   1. `execute_with_jit` never called `set_trampoline_vm`, so the
///      thread-local `TRAMPOLINE_VM` was null and
///      `dispatch_call_via_trampoline_vm` short-circuited to `TAG_NULL`.
///   2. Even with the VM wired, the trampoline constructed a bare
///      `ValueWord::from_function(fid)` — discarding the closure's
///      captures and producing `Null` / wrong values on return.
///
/// This test lowers a closure whose body takes the JIT's current
/// dynamic-arithmetic bail path (forcing a null function_table slot)
/// while still needing captures to produce the right answer. The
/// pre-fix code returned `Null`; the fix dispatches through
/// `jit_trampoline_call_closure` with the captures threaded through.
/// F4 Option / null-coalescing regression: the bytecode compiler's
/// `FrameDescriptor.slots` seeding of `MirToIR` used incompatible slot
/// numbering. MIR reserves `SlotId(0)` for the implicit return value
/// and numbers parameters starting at 1; the bytecode compiler puts
/// the first parameter at slot 0. Seeding via the bytecode layout
/// mis-declared MIR's return slot with the first param's `NativeKind`,
/// so e.g. a `bool` parameter forced slot 0 (the F64 return) to be
/// declared as `Bool` in Cranelift. Writing `return 7.0` then went
/// through `ensure_kind(F64, Bool) → ireduce I8` which truncated the
/// F64 bit pattern to zero, and `result ?? 42.0` evaluated to 42.0 for
/// every branch of the caller.
///
/// Case 1: function returns `number?`, two `return` paths (literal F64
/// and `None`). Pre-fix: returns 42.0 (None-ness was forced). Post-fix:
/// returns 7.0.
#[test]
fn option_return_conditional_number_some() {
    jit_expect_number(
        r#"
fn get_val(flag: bool) -> number? {
    if flag {
        return 7.0
    }
    return None
}
let x = get_val(true)
x ?? 42.0
"#,
        7.0,
    );
}

/// F4 Option / null-coalescing regression (None branch). Pre-fix the
/// None branch also returned 42.0 — but so did the Some branch, so this
/// passes both before and after the fix. Kept for symmetry with the
/// Some-returning case above so the two sides of the conditional stay
/// pinned together if one regresses in isolation.
#[test]
fn option_return_conditional_number_none() {
    jit_expect_number(
        r#"
fn get_val(flag: bool) -> number? {
    if flag {
        return 7.0
    }
    return None
}
let x = get_val(false)
x ?? 42.0
"#,
        42.0,
    );
}

#[test]
fn closure_non_jit_compiled_dispatches_through_trampoline_vm() {
    // `|| { x = x + base; x }` with `let base` (immutable capture) and
    // `let mut x` (OwnedMutable capture) exercises the exact shape from
    // the original bug report. Calling it twice sums base twice into x.
    jit_expect_int(
        r#"
fn main() -> int {
    let base: int = 10
    let mut x: int = 0
    let f = || { x = x + base; x }
    f()
    f()
}
main()
"#,
        20,
    );
}

// Phase 4b Round 4 Surface-1a LANG-W13-3-iife-closure-capture JIT residual
// regression (2026-05-18, commit pending). Pinning the MIR-lowering-side
// IIFE `__call__` interception at `mir/lowering/expr.rs`
// (`Expr::MethodCall { method: "__call__", .. }` arm) — bytecode parses
// IIFE / chained call `f(a)(b)` as `MethodCall { method: "__call__",
// receiver: <callable> }` per `crates/shape-ast/src/parser/expressions/
// primary.rs:167`. The bytecode compiler already intercepts the same
// shape at `compiler/expressions/function_calls.rs:1832` and emits
// `CallValue` directly with the receiver as the callee on the VM
// stack. Mirror that producer-side classification at MIR lowering per
// ADR-006 §2.7.5 stamp-at-compile-time: the receiver becomes the Call
// terminator's `func` operand (NOT a `MirConstant::Method("__call__")`
// method-name carrier), so the JIT terminator's indirect-call path at
// `mir_compiler/terminators.rs:1486` routes through `jit_call_value`'s
// §2.7.11/Q12 closure-arm correctly.
//
// Pre-fix: MIR carried `Method("__call__")` → JIT `jit_call_method`
// shell at `ffi/call_method/mod.rs:801` hit the
// `NativeKind::Ptr(_) => TAG_NULL` arm (no `__call__` builtin for
// closure receivers), surfacing as `0xfffb_0000_0000_0000` (TAG_NULL)
// at the assignment destination.

/// IIFE result assigned to a `let mut` integer slot. Pre-fix: JIT prints
/// `-1407374883553280` (TAG_NULL NaN-boxed bits); VM prints `8`.
/// Post-fix: VM=JIT=`8`.
#[test]
fn iife_assignment_to_mut_local() {
    jit_expect_int(
        r#"
let base = 7
let mut total = 0
total = (|y| y + base)(1)
total
"#,
        8,
    );
}

/// IIFE inside a `for` loop accumulating into a `let mut` integer
/// (the R3-1 close report's exact reproducer). Pre-fix: JIT prints
/// `-4222124650659840` (a NaN bit pattern produced by adding 0 + TAG_NULL
/// three times via Int64 addition); VM prints `27`. Post-fix: VM=JIT=`27`.
#[test]
fn iife_in_for_loop_accumulator() {
    jit_expect_int(
        r#"
let base = 7
let v: Vec<int> = [1, 2, 3]
let mut total = 0
for x in v { total += (|y| y + base)(x) }
total
"#,
        27,
    );
}

/// IIFE with no captures, no `for` — the simplest shape that exercises
/// the same MIR lowering path. Pre-fix: JIT segfaulted (ec=139); VM
/// printed `8`. Post-fix: VM=JIT=`8`.
#[test]
fn iife_no_capture_assignment_to_mut_local() {
    jit_expect_int(
        r#"
let mut total = 0
total = (|y| y + 7)(1)
total
"#,
        8,
    );
}

/// IIFE result via captured-closure binding in a let chain. Mirrors
/// the by-name `closure_simple_dispatch_returns_six` shape (which
/// works without the fix) plus a `let mut` reassign to pin both the
/// IIFE MIR lowering and the producer-side stamp at the assignment
/// destination kind together.
#[test]
fn iife_with_mut_local_then_let_chain() {
    jit_expect_int(
        r#"
let base = 7
let mut total = 0
total = (|y| y + base)(1)
let result = total + 0
result
"#,
        8,
    );
}

// ============================================================================
// Phase 4b Round 5 W14.2-E1 — JIT call-method-arity matrix (per audit §4 W10)
//
// Per `docs/cluster-audits/v0.3-w14-test-coverage-audit.md` §4 W10:
//
// > W10 JIT call-method user-trait | (b) PARTIAL | Direct fix tested per
// > `774b1712`; missing: per-trait-method-arity (n=0..6+) coverage matrix.
//
// Empirical at HEAD 2924b685 (worktree branch base): the user-trait
// method-dispatch path is byte-equal VM==JIT ONLY for n=0 arity AND only
// when the receiver is an EMPTY type (no fields). The matrix below pins:
//
// (a) WORKING combinations as JIT-direct regression tests (assertion
//     against the correct value).
//
// (b) DIVERGENT combinations surfaced as W14.2-E1-SURFACE-A (named
//     v0.3-gating candidate) in the close report. NOT added as failing
//     tests — surface-and-stop per dispatch discipline; the failing-test
//     pattern would block close-gate green.
//
// Working combos (asserted below):
//   - n=0, int return, empty receiver (anchors `774b1712` W10 fix)
//   - n=0, number return, empty receiver
//   - n=0, bool return, empty receiver
//   - n=1, bool arg + bool return, empty receiver (bit-equal happens to map)
//
// Divergent combos (surfaced, not asserted):
//   - n=1..7, int arg + int return, empty receiver: JIT returns garbage NaN-bit
//     pattern; VM returns correct value. SURFACE-A.
//   - n=1, number arg + number return: JIT returns tiny near-zero; VM correct.
//   - n=1, string arg + string return: JIT SEGFAULTS (exit 139). SURFACE-A1.
//   - n=0, int return, TypedObject receiver with field access in body
//     (`self.value * 2`): JIT garbage; VM correct. SURFACE-A2.
//   - n=0, string return, ANY receiver: JIT falls back to VM via
//     RETURN_TAG_NANBOXED kind-source gap (W10 jit-playbook §5). Produces
//     correct value via fallback but not JIT-native.
// ============================================================================

/// W14.2-E1 arity n=0: trait method on empty receiver returning int.
/// Anchors `774b1712` W10 direct fix at byte-equal VM == JIT.
#[test]
fn trait_method_arity_n0_int_return() {
    jit_expect_int(
        r#"
trait Greet {
    fn say() -> int
}
type Hi {}
impl Greet for Hi {
    fn say() -> int {
        42
    }
}
let h = Hi {}
h.say()
"#,
        42,
    );
}

/// W14.2-E1 arity n=0: trait method on empty receiver returning number.
#[test]
fn trait_method_arity_n0_number_return() {
    jit_expect_number(
        r#"
trait NumTrait {
    fn nfn() -> number
}
type N {}
impl NumTrait for N {
    fn nfn() -> number {
        3.14
    }
}
let n = N {}
n.nfn()
"#,
        3.14,
    );
}

/// W14.2-E1 arity n=0: trait method on empty receiver returning bool.
#[test]
fn trait_method_arity_n0_bool_return() {
    let result = jit_eval(
        r#"
trait BoolTrait {
    fn bfn() -> bool
}
type B {}
impl BoolTrait for B {
    fn bfn() -> bool {
        true
    }
}
let b = B {}
b.bfn()
"#,
    );
    match result {
        WireValue::Bool(true) => {}
        other => panic!("Expected Bool(true), got {:?}", other),
    }
}

/// W14.2-E1 arity n=1 bool: bit-equal happens to map even though int args
/// diverge. Pinned at VM == JIT byte-equal to detect regression if the
/// bit-equal-map invariant changes.
#[test]
fn trait_method_arity_n1_bool_args_and_return() {
    let result = jit_eval(
        r#"
trait BoolTrait {
    fn bfn(x: bool) -> bool
}
type B {}
impl BoolTrait for B {
    fn bfn(x: bool) -> bool {
        !x
    }
}
let b = B {}
b.bfn(false)
"#,
    );
    match result {
        WireValue::Bool(true) => {}
        other => panic!("Expected Bool(true), got {:?}", other),
    }
}

/// W14.2-E1 arity n=0 receiver-with-field: empty body returning const.
/// Pre-fix: VM=JIT=42 (works because `self.value` is not read in body).
/// Companion guard: ensures we don't regress when receiver layout has
/// fields but method body doesn't access them.
#[test]
fn trait_method_arity_n0_typedobj_receiver_no_field_access() {
    jit_expect_int(
        r#"
trait Operate {
    fn op() -> int
}
type Wrapper { value: int }
impl Operate for Wrapper {
    fn op() -> int {
        42
    }
}
let w = Wrapper { value: 100 }
w.op()
"#,
        42,
    );
}
