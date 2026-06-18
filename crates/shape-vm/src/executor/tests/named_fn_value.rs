//! Named-function-value carrier tests (R1 named-fn-as-value capture fix).
//!
//! A *named* function referenced as a value is produced as a bare
//! function-id with runtime kind `NativeKind::UInt64`
//! (`PushConst(Constant::Function)`), whereas a *closure* value is an
//! `Arc<HeapValue::ClosureRaw>` with kind `Ptr(HeapKind::Closure)`. The
//! two carriers meet at three sites that previously SIGSEGV'd when a
//! named-fn-id was treated as a closure pointer:
//!
//!   1. closure capture of a named fn that escapes (returned), then
//!      value-called — `op_make_closure` stored the bare fn-id into a
//!      `Ptr(HeapKind::Closure)` capture slot; `release_typed_closure`
//!      and the later value-call dereferenced the integer as an
//!      `Arc<HeapValue>` pointer.
//!   2. a captured named fn forwarded onward as a call argument — the
//!      capture was mis-classified as an unknown `Pointer(Void)` →
//!      `Ptr(HeapKind::NativeView)` slot.
//!   3. `xs.map(square)` / `xs.filter(is_even)` — the array HOF predicate
//!      check rejected the `UInt64` fn-id carrier outright.
//!
//! The fix: `op_make_closure` materializes a real zero-capture closure
//! carrier for a named-fn-id captured into a `Ptr(HeapKind::Closure)`
//! slot; `resolve_capture_concrete_type` classifies a capture forwarded
//! as a call argument as a function value; the array-HOF predicate check
//! accepts the `UInt64` fn-id carrier (`call_value_immediate_nb`'s
//! `UInt64` arm already value-calls it). A genuine non-callable value in
//! a closure-stamped slot is a clean RuntimeError, never a SIGSEGV.

use super::test_utils::{eval_result, eval_typed_i64};

/// CONFIRMED repro: a named fn passed as an unannotated param, captured
/// into the returned closure `|x| f(x)`, then invoked. Pre-fix: SIGSEGV.
#[test]
fn named_fn_captured_into_escaping_closure_then_called() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        fn wrap(f) { |x| f(x) }
        let w = wrap(square)
        w(5)
    "#;
    assert_eq!(eval_typed_i64(src), 25);
}

/// A captured named fn that is forwarded onward as a call argument
/// (`apply(g, y)`), not called directly in the closure body. Pre-fix the
/// capture was mis-classified `Pointer(Void)` → SIGSEGV.
#[test]
fn named_fn_captured_then_forwarded_as_call_arg() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        fn apply(f, v) { f(v) }
        fn wrap(g) { |y| apply(g, y) }
        let w = wrap(square)
        w(4)
    "#;
    assert_eq!(eval_typed_i64(src), 16);
}

/// Named fn carried across two closure-returning levels.
#[test]
fn named_fn_carried_across_two_levels() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        fn lvl1(f) { |x| f(x) }
        fn lvl2(g) { |y| lvl1(g)(y) }
        let w = lvl2(square)
        w(7)
    "#;
    assert_eq!(eval_typed_i64(src), 49);
}

/// Direct value-call of a named-fn param (no capture) must keep working —
/// this routes through `call_value_immediate_nb`'s `UInt64` arm.
#[test]
fn named_fn_direct_value_call_still_works() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        fn apply(f, v) { f(v) }
        apply(square, 6)
    "#;
    assert_eq!(eval_typed_i64(src), 36);
}

/// Capturing a genuine closure value (already `Ptr(HeapKind::Closure)`)
/// must be unaffected by the named-fn reconciliation.
#[test]
fn closure_value_captured_into_escaping_closure_still_works() {
    let src = r#"
        fn wrap2(g) { |x| g(x) }
        let w = wrap2(|y| y + 1)
        w(5)
    "#;
    assert_eq!(eval_typed_i64(src), 6);
}

/// `xs.map(named_fn)` — the array HOF predicate carrier check must accept
/// a `UInt64` fn-id (pre-fix: "predicate must be a closure, got kind
/// UInt64").
#[test]
fn map_with_named_fn() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        let xs = [1, 2, 3]
        let r = xs.map(square)
        r[2]
    "#;
    assert_eq!(eval_typed_i64(src), 9);
}

/// `xs.filter(named_fn)` — same carrier acceptance on the keep-mask path.
#[test]
fn filter_with_named_fn() {
    let src = r#"
        fn is_even(x: int) -> bool { x - (x / 2) * 2 == 0 }
        let xs = [1, 2, 3, 4]
        let r = xs.filter(is_even)
        r.len()
    "#;
    assert_eq!(eval_typed_i64(src), 2);
}

/// Whatever path a named-fn-as-value takes, it must NEVER segfault: the
/// worst case is a clean error. This is the binding-compliant
/// surface-and-stop guarantee — exercised here as "executes to a value or
/// a clean Result::Err, no abort".
#[test]
fn named_fn_as_value_never_aborts() {
    let src = r#"
        fn square(x: int) -> int { x * x }
        fn wrap(f) { |x| f(x) }
        let w = wrap(square)
        w(5)
    "#;
    // A clean Ok or Err is acceptable; a SIGSEGV would crash the test
    // process before any assertion. Reaching this assert at all proves no
    // abort occurred.
    let _ = eval_result(src);
}
