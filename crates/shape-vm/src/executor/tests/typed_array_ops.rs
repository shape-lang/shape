//! Vec<T> typed array integration tests — end-to-end exercises for typed
//! array construction, SIMD arithmetic, and method dispatch.
//!
//! T1-host-tier-marshal-rebuild (Phase 2d Wave 1): the pre-strict-typing
//! bodies built `Constant::Value(ValueWord)` constants encoding pre-formed
//! typed arrays via the deleted `ValueWord::from_*_array` constructors and
//! a hand-emitted stack-based `CallMethod` convention. That host-tier
//! marshal API was deleted by the strict-typing bulldozer; per ADR-006
//! §2.7.4 / §2.7.5 the post-`KindedSlot` shape drives these tests through
//! the language surface (`eval(...)` → `KindedSlot`) and reads the result
//! via the §2.7.6 / Q8 scalar accessors (`as_i64` / `as_f64` / `as_bool`).
//! Re-introducing `Constant::Value(ValueWord)` (under any rename) or a
//! polymorphic carrier on the test path is refused by playbook §1 T1
//! "Forbidden in this sub-cluster".
//!
//! Some bodies remain `todo!()` because the *language* feature they
//! exercise — typed-array literals lowered to `NewTypedArray*` opcodes via
//! parser/compiler intrinsics — is still SURFACE under separate Wave 2
//! sub-clusters (W17-array-typed-receiver, W17-typed-carrier-monomorphization).
//! Those are unblocked once their respective sub-clusters land.

use super::test_utils::{eval, eval_typed_i64};

#[test]
fn test_new_typed_array_ints() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_new_typed_array_floats() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_new_typed_array_bools() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_new_typed_array_mixed_falls_back() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_sum() {
    // T1 smoke target: number-array sum returns Float64. The result reads
    // via `as_f64()` on the `KindedSlot` (§2.7.6 / Q8).
    let result = eval("[1.0, 2.0, 3.0, 4.0].sum()");
    assert_eq!(result.as_f64(), Some(10.0));
}

#[test]
fn test_float_array_avg() {
    // W17-array-typed-receiver: v2 typed-number-array `avg` PHF entry
    // wired in this sub-cluster. The receiver is a v2 `TypedArray<f64>`
    // pointer (`NativeKind::UInt64` carrier); the body delegates to
    // `v2_array_detect::avg_elements`.
    let result = eval("[2.0, 4.0, 6.0, 8.0].avg()");
    assert_eq!(result.as_f64(), Some(5.0));
}

#[test]
fn test_float_array_min() {
    // W17-array-typed-receiver: v2 typed-number-array `min` PHF entry
    // wired in this sub-cluster.
    let result = eval("[3.5, 1.5, 4.5, 2.5].min()");
    assert_eq!(result.as_f64(), Some(1.5));
}

#[test]
fn test_float_array_max() {
    // W17-array-typed-receiver: v2 typed-number-array `max` PHF entry
    // wired in this sub-cluster.
    let result = eval("[3.5, 1.5, 4.5, 2.5].max()");
    assert_eq!(result.as_f64(), Some(4.5));
}

#[test]
fn test_float_array_len() {
    // T1 smoke target: `len()` returns Int64 even on a float array.
    assert_eq!(eval_typed_i64("[1.0, 2.0, 3.0].len()"), 3);
}

#[test]
fn test_float_array_dot_product() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_norm() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_cumsum() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_diff() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_abs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_to_array() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_sum() {
    // T1 smoke target: `[10, 20, 30].sum()` runs end-to-end through the
    // post-`KindedSlot` host-tier `eval()` helper. The compiler routes the
    // int array literal through the typed `NewTypedArrayI64` emission and
    // the `sum` PHF entry on `typed_int_array_methods.rs`, which returns
    // an `Int64` `KindedSlot`. The §2.7.6 / Q8 scalar accessor decodes the
    // result without any host-tier `ValueWord` synthesis.
    let result = eval("[10, 20, 30].sum()");
    assert_eq!(result.as_i64(), Some(60));
}

#[test]
fn test_int_array_avg() {
    // W17-array-typed-receiver: v2 typed-int-array `avg` PHF entry
    // wired in this sub-cluster. Result kind is `Float64` (mean of an
    // integer array is a float).
    let result = eval("[2, 4, 6, 8].avg()");
    assert_eq!(result.as_f64(), Some(5.0));
}

#[test]
fn test_int_array_min() {
    // W17-array-typed-receiver: v2 typed-int-array `min` PHF entry
    // wired in this sub-cluster.
    let result = eval("[3, 1, 4, 1, 5, 9, 2, 6].min()");
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_int_array_max() {
    // W17-array-typed-receiver: v2 typed-int-array `max` PHF entry
    // wired in this sub-cluster.
    let result = eval("[3, 1, 4, 1, 5, 9, 2, 6].max()");
    assert_eq!(result.as_i64(), Some(9));
}

#[test]
fn test_int_array_len() {
    // T1 smoke target: `len()` of a typed int array returns an Int64. The
    // `eval_typed_i64` helper (`test_utils.rs:118`) stamps Int64 onto the
    // top-level return-bits and unwraps the §2.7.6 scalar accessor.
    assert_eq!(eval_typed_i64("[1, 2, 3, 4, 5].len()"), 5);
}

#[test]
fn test_int_array_abs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_to_array() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_any() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_any_all_false() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_all() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_all_with_false() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_bool_array_to_array() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_unknown_method_errors() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_unknown_method_errors() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

// =========================================================================
// Phase 4b Round 3 Surface-1A LANG-W13-3-iife-closure-capture regression.
//
// Pre-fix: IIFE result was Unknown — `let r = (|y|...)(x); r + N`
// surfaced "Cannot infer types for binary operation `Add`: operand types
// are `unknown` and `int`" at compile time, or "no method 'add' on
// receiver kind Int64" at VM runtime when the IIFE result entered a
// `total += ...` accumulator.
//
// Post-fix (per ADR-006 §2.7.5 producer-side stamp-at-compile-time): the
// closure-body return type is inferred at the `__call__` MethodCall site
// (via caller-context arg types) and `last_expr_*` is populated so
// `propagate_initializer_type_to_slot` records the destination's
// NumericType. See
// `crates/shape-vm/src/compiler/expressions/function_calls.rs::
// compile_expr_method_call` for the producer site.
//
// Sibling type-inference fix at
// `crates/shape-runtime/src/type_system/inference/expressions.rs`
// special-cases `MethodCall { method: "__call__" }` on Function-typed
// receivers so the result type is the function's declared return type.
// =========================================================================

#[test]
fn test_iife_literal_body() {
    // Baseline: IIFE with a literal-bodied closure resolves end-to-end.
    assert_eq!(eval_typed_i64("(|y| y + 1)(3)"), 4);
}

#[test]
fn test_iife_result_typed_in_binop() {
    // Regression: `let r = (|y|...)(x); r + 100` must resolve the
    // closure's return type to `int` so the binop emits AddInt.
    assert_eq!(eval_typed_i64("let r = (|y| y + 1)(3); r + 100"), 104);
}

#[test]
fn test_iife_with_captured_outer_int() {
    // Regression: closure body references an outer-scope `int`
    // identifier. The caller-context arg-type seed at the `__call__`
    // site lets the body's `y + base` resolve.
    assert_eq!(
        eval_typed_i64("let base = 7; let r = (|y| y + base)(3); r + 100"),
        110
    );
}

#[test]
fn test_iife_in_for_loop_compound_accumulator() {
    // Regression: the f06 fuzz-corpus seed pattern (closures/
    // f06_capture_in_loop.shape). Pre-fix VM exited with "no method
    // 'add' on receiver kind Int64" because the IIFE's destination
    // slot stayed Unknown and the compound-assign `total +=` resolved
    // to a method-dispatch instead of typed AddInt. (1+7)+(2+7)+(3+7)=27.
    assert_eq!(
        eval_typed_i64(
            "let base = 7; let v: Vec<int> = [1, 2, 3]; let mut total = 0; for x in v { total += (|y| y + base)(x) }; total"
        ),
        27
    );
}

// =========================================================================
// Phase 4b Round 3 Surface-1B LANG-W13-3-double-filter-chain regression.
//
// Pre-fix: chained `.filter().filter()` couldn't specialize the outer
// `filter` call because `concrete_type_for_expr(receiver)` returned None
// for the inner-MethodCall receiver — `try_monomorphize_method_call`
// bailed and the generic-template fall-through entered an infinite
// loop / timed out (single-timeout on the c08 seed at
// `collections/c08_double_filter.shape`).
//
// Post-fix (per ADR-006 §2.7.5 producer-side stamp-at-compile-time):
// `concrete_type_for_expr` reads `specialized_call_return_concrete_type`
// for `Expr::MethodCall` receivers, recovering the inner call's
// specialized return ConcreteType from `monomorphized_method_call_sites`.
// See `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs::
// concrete_type_for_expr` MethodCall arm.
// =========================================================================

#[test]
fn test_double_filter_chain_len() {
    // The c08 fuzz-corpus seed pattern. Pre-fix: VM hung (ec=124).
    // Post-fix: elements 3, 4, 5, 6, 7 survive both filters → 5.
    // Uses `eval_with_prelude` because `.filter()` is stdlib-resident.
    use super::test_utils::eval_with_prelude;
    let result = eval_with_prelude(
        "let v: Vec<int> = [1,2,3,4,5,6,7,8,9,10]; v.filter(|x| x > 2).filter(|x| x < 8).len()",
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_triple_filter_chain_len() {
    // Same producer-side stamp composes across N chained filters.
    // Filter to >2, <8, !=5 → 3, 4, 6, 7 (4 elements).
    use super::test_utils::eval_with_prelude;
    let result = eval_with_prelude(
        "let v: Vec<int> = [1,2,3,4,5,6,7,8,9,10]; v.filter(|x| x > 2).filter(|x| x < 8).filter(|x| x != 5).len()",
    );
    assert_eq!(result.as_i64(), Some(4));
}

#[test]
fn test_double_filter_chain_into_let() {
    // Variant: the chained-filter result is bound to a let binding.
    // Pre-fix: hung in compile (`infinite loop` analog at the
    // bytecode-emission level when the receiver-type couldn't be
    // recovered). Post-fix: the intermediate Vec<int> from
    // `.filter()` resolves so the outer `.filter()` specializes.
    use super::test_utils::eval_with_prelude;
    let result = eval_with_prelude(
        "let v: Vec<int> = [1,2,3,4,5]; let r = v.filter(|x| x > 1).filter(|x| x < 5); r.len()",
    );
    assert_eq!(result.as_i64(), Some(3));
}

// ─────────────────────────────────────────────────────────────────────
// Phase 4b Round 4 LANG-9-spin-2c-reduce-2param-closure-inference
// regression coverage: pins the §4.3 territory's CURRENT compile-success
// at HEAD `5d842283` after Surface-1B merge `f37476f7` (close commit
// `05eb1d6d`) added the `Expr::MethodCall` arm to `concrete_type_for_expr`
// at `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs:1423-1425`.
//
// The 2-param closure-arg-type seeding for `reduce`-shaped HOFs is already
// in place via `install_pending_closure_param_types_for_hof`'s `is_reduce`
// branch at `crates/shape-vm/src/compiler/expressions/function_calls.rs:1591-1597`
// (`vec![Some(elem_ann.clone()), Some(elem_ann)]` — homogeneous-fold
// seeding for `acc` + `x`). Pre-Surface-1B the receiver-type resolution
// for chained `.map().reduce(...)` failed (no MethodCall arm in
// `concrete_type_for_expr`), so hints were never installed and the
// closure's `a + b` failed strict-typing as
// "Cannot infer types for binary operation Add: operand types are
// `unknown` and `unknown`".
//
// These tests use `compile_with_prelude` (NOT `eval_*`) because the
// runtime SURFACE'es at V3-S5 ckpt-2 (`Vec<int>.reduce` handler
// `handle_int_reduce` returns `ckpt2_surface("Vec<int>.reduce", args)`
// at `crates/shape-vm/src/executor/objects/typed_array_methods.rs:488-493`
// — explicitly UNREACHABLE until V3-S5 ckpt-6 STRICT close per
// `docs/cluster-audits/v0.3-w15-lang-9-spinoffs-audit.md` §2.4 / §4.4).
// Compile-success is the bounded contract the §4.3 territory owns; runtime
// is W16.2-N partition territory per `docs/cluster-audits/v0.3-w16-v3s5-
// ckpt56-strict-close-audit.md`.

#[test]
fn test_lang9_spin_2c_reduce_chained_map_wrong_order_is_clean_error() {
    // ε-3 reduce-argorder fix: the wrong-order call
    // `.reduce(0, |a, b| a + b)` (init first) is genuinely ill-typed —
    // Shape's `reduce` is `reduce(f, init)`, callback FIRST (see
    // `crates/shape-runtime/stdlib-src/core/vec.shape:59`). Before the fix
    // this miscompiled into a re-entrant `main` (infinite loop) because the
    // int `0` bound the generic callable param `f`. It must now be a CLEAN
    // compile-time error surfaced by the arg-kind guard in
    // `install_pending_closure_param_types_for_hof`.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1,2,3,4,5].map(|x| x * 2).reduce(0, |a, b| a + b)",
    );
    let err = result.expect_err(
        "wrong-order reduce(init, closure) must be a clean compile error, not a miscompile",
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("reduce") && msg.contains("first argument"),
        "error must explain the arg-order problem; got: {msg}"
    );
}

#[test]
fn test_lang9_spin_2c_reduce_callback_first_compiles() {
    // Sibling shape: callback-first form `.reduce(|a,b|a+b, 0)` exercises
    // the same 2-param closure-arg-type seeding path with the closure as
    // the FIRST positional arg. Hints are per-closure-param positional,
    // so hint indexing is identical regardless of the closure's index in
    // the call's arg list.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1,2,3,4,5].map(|x| x * 2).reduce(|a, b| a + b, 0)",
    );
    assert!(
        result.is_ok(),
        "callback-first reduce must compile; got: {:?}",
        result.err()
    );
}

#[test]
fn test_lang9_spin_2c_reduce_chained_map_f64_correct_order_compiles() {
    // Sibling shape: f64 element + accumulator, callback FIRST (correct
    // order). The receiver's element type drives both `a` and `b` hint via
    // the homogeneous-fold branch — `vec![Some(number_ann), Some(number_ann)]`
    // — so the closure body `a + b` resolves to AddNumber.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1.0, 2.0, 3.0, 4.0, 5.0].map(|x| x * 2.0).reduce(|a, b| a + b, 0.0)",
    );
    assert!(
        result.is_ok(),
        "f64 chained map.reduce(closure, init) must compile; got: {:?}",
        result.err()
    );
}

#[test]
fn test_lang9_spin_2c_reduce_direct_array_compiles() {
    // Sibling shape: non-chained receiver `[1,2,3]` (no intermediate
    // `.map`). The receiver-type resolution goes through the `Expr::Array`
    // arm of `concrete_type_for_expr` (populated by LANG-9 close at
    // `compile_expr_array`), then `install_pending_closure_param_types_
    // for_hof`'s is_reduce branch seeds both `acc` and `x` as `int`.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1, 2, 3].reduce(|acc, x| acc + x, 0)",
    );
    assert!(
        result.is_ok(),
        "direct array.reduce must compile; got: {:?}",
        result.err()
    );
}

// ──────────────────────────────────────────────────────────────────────
// ε-3 reduce-argorder: a wrong-argument-order `reduce` call (an int /
// literal where the callback closure is expected) must produce a CLEAN
// compile-time type error — never the pre-fix re-entrant `main`
// miscompile (infinite loop / timeout). The guard lives in
// `install_pending_closure_param_types_for_hof`
// (`crates/shape-vm/src/compiler/expressions/function_calls.rs`):
// the callback is positional arg 0 for every wired HOF, so a provably
// non-callable arg 0 (literal / array literal / object literal) is
// rejected with a `SemanticError`.

#[test]
fn test_reduce_correct_order_compiles() {
    // The exact close-gate input: callback first, init second — the
    // correct order for Shape's `reduce(f, init)` signature.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1, 2, 3].reduce(|acc, x| acc + x, 0)",
    );
    assert!(
        result.is_ok(),
        "correct-order reduce(closure, init) must compile; got: {:?}",
        result.err()
    );
}

#[test]
fn test_reduce_wrong_order_int_first_is_clean_error() {
    // The close-gate bug input: `reduce(0, |acc,x| acc+x)` — init first,
    // callback second (JS/conventional order, but WRONG for Shape).
    // Pre-fix: re-entrant `main` miscompile (infinite loop, ec=124).
    // Post-fix: clean compile-time `SemanticError`.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1, 2, 3].reduce(0, |acc, x| acc + x)",
    );
    let err = result.expect_err(
        "wrong-order reduce(int, closure) must be a clean compile error",
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("reduce")
            && msg.contains("first argument")
            && msg.contains("int"),
        "error must name `reduce`, the first-argument problem, and `int`; got: {msg}"
    );
}

#[test]
fn test_reduce_non_closure_first_arg_string_is_clean_error() {
    // A string literal as `reduce`'s first argument is equally
    // ill-typed and must surface a clean compile error.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "let _ = [1, 2, 3].reduce(\"seed\", |acc, x| acc + x)",
    );
    let err = result.expect_err(
        "reduce with a string first arg must be a clean compile error",
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("reduce") && msg.contains("string"),
        "error must name `reduce` and `string`; got: {msg}"
    );
}

#[test]
fn test_map_non_closure_arg_is_clean_error() {
    // Sibling-HOF coverage: `map` also takes its callback at positional
    // arg 0. A non-callable literal there is the same footgun and must
    // likewise be a clean compile error, not a miscompile.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude("let _ = [1, 2, 3].map(7)");
    let err = result
        .expect_err("map with a non-closure arg must be a clean compile error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("map") && msg.contains("first argument"),
        "error must name `map` and the first-argument problem; got: {msg}"
    );
}

#[test]
fn test_filter_named_function_arg_still_allowed() {
    // The guard must NOT false-positive on a legitimate non-literal
    // callable. An identifier could resolve to a function/closure, so
    // it is never rejected by `provably_non_callable_kind`. This call
    // may still fail for other reasons, but it must NOT be rejected
    // with the arg-order/arg-kind diagnostic.
    use super::test_utils::compile_with_prelude;
    let result = compile_with_prelude(
        "fn keep(n: int) -> bool { n > 1 }\nlet _ = [1, 2, 3].filter(keep)",
    );
    if let Err(err) = &result {
        let msg = format!("{err:?}");
        assert!(
            !msg.contains("expects a closure (function) as its first argument"),
            "named-function arg must not trip the arg-kind guard; got: {msg}"
        );
    }
}


// ──────────────────────────────────────────────────────────────────────
// Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18)
//
// Per ADR-006 §2.7.5 stamp-at-compile-time + §2.7.24 Q25.A SUPERSEDED +
// audit `v0.3-w16-v3s5-ckpt56-strict-close-audit.md` §2.1 + §3.A row 1.
// Verifies that `Array<UserStruct>` literals now route through the v2-raw
// `TypedArray<*const TypedObjectStorage>` carrier (`NewTypedArrayTypedObject`
// + per-element `TypedArrayPushTypedObject` + `TypedArrayGetTypedObject`
// for index access). Pre-fix: every shape SURFACEd at `op_new_array(N)`
// per the deleted `TypedArrayData` enum + `Buf<T>` wrapper layer.
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_typed_object_array_bare_literal_indexed_field() {
    // Bare-literal form: elements are struct literals.
    // `[B{v:1}, B{v:2}]` → `infer_array_element_type` returns
    // `Ptr(HeapKind::TypedObject)` per W16.2-A new arms.
    let result = eval(
        "type B { v: int }\n\
         let arr = [B { v: 1 }, B { v: 2 }]\n\
         arr[1].v",
    );
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_typed_object_array_annotated_literal_indexed_field() {
    // Annotated form: `let arr: Array<B> = [...]`.
    // The annotation routes through `resolve_typed_array_kind_from_annotation`
    // → `TypedArrayKind::TypedObject` per W16.2-A new annotation arm.
    let result = eval(
        "type B { v: int }\n\
         let arr: Array<B> = [B { v: 10 }, B { v: 20 }, B { v: 30 }]\n\
         arr[2].v",
    );
    assert_eq!(result.as_i64(), Some(30));
}

#[test]
fn test_typed_object_array_index_zero() {
    // Verify index 0 round-trip (the smoke-fixture target shape).
    let result = eval(
        "type Pt { x: int, y: int }\n\
         let arr = [Pt { x: 7, y: 11 }, Pt { x: 13, y: 17 }]\n\
         arr[0].y",
    );
    assert_eq!(result.as_i64(), Some(11));
}

#[test]
fn test_typed_object_array_function_call_result_elements() {
    // Function-call-result element form: elements are `f(...)` calls
    // returning a registered struct. `array_elements_all_typed_object`
    // resolves through `type_tracker.function_return_types`.
    // This is the sim-test shape from `bin/shape-cli/tests/stdlib/
    // simulation.rs:472` (`let boxes = [aabb(...), aabb(...), ...]`).
    let result = eval(
        "type Box { v: int }\n\
         fn make(v: int) -> Box { Box { v: v } }\n\
         let arr = [make(100), make(200), make(300)]\n\
         arr[2].v",
    );
    assert_eq!(result.as_i64(), Some(300));
}

#[test]
fn test_typed_object_array_struct_with_number_field() {
    // Mixed-field struct: number field round-trip through the v2-raw
    // TypedObject carrier. Validates that field access on indexed elements
    // works for non-int field types (the `aabb` shape from the sim tests).
    let result = eval(
        "type V { x: number, y: number }\n\
         let arr = [V { x: 1.5, y: 2.5 }, V { x: 3.5, y: 4.5 }]\n\
         arr[1].x",
    );
    assert_eq!(result.as_f64(), Some(3.5));
}

// ── r5c-2-β-δ-(α): `Ptr(HeapKind::TypedArray)` carrier regression tests ─────
//
// Before the fix, both trigger paths panicked at the vacated
// `unreachable!()` arm in `vm_impl/stack.rs::clone_with_kind`: the W12
// audit retired the `HeapKind::TypedArray` dispatch arm asserting "no live
// slot bits carry this kind", but `Array<T>` struct fields
// (`field_tag_to_native_kind` → `Ptr(HeapKind::TypedArray)`) and closure
// captures (`closure_layout.rs::native_kind_from_concrete_type` →
// `Ptr(HeapKind::TypedArray)`) both put this kind on a live slot. The fix
// re-instates the arm as a v2-raw `*mut TypedArray<T>` carrier (retain via
// `v2_retain`, release via `release_v2_typed_array`).

#[test]
fn test_struct_array_field_read_int() {
    // Trigger path 1: read an `Array<int>` field out of a struct. Pre-fix:
    // VM panic at `clone_with_kind` on `Ptr(HeapKind::TypedArray)`.
    let result = eval(
        "type Bag { items: Array<int> }\n\
         let b = Bag { items: [1, 2, 3] }\n\
         b.items[0]",
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_struct_array_field_read_each_index() {
    // Every index of a struct array field reads correctly — no drift,
    // no double-free across the three field reads.
    assert_eq!(
        eval(
            "type Bag { items: Array<int> }\n\
             let b = Bag { items: [10, 20, 30] }\n\
             let a: int = b.items[0]\n\
             let c: int = b.items[1]\n\
             let d: int = b.items[2]\n\
             a + c + d"
        )
        .as_i64(),
        Some(60)
    );
}

#[test]
fn test_struct_array_field_length() {
    // `op_length` on a `Ptr(HeapKind::TypedArray)` struct-field carrier.
    let result = eval(
        "type Bag { items: Array<int> }\n\
         let b = Bag { items: [1, 2, 3, 4, 5] }\n\
         b.items.length",
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_struct_array_field_number_elem() {
    // `Array<number>` struct field — exercises the F64 element-kind arm
    // of the shared `as_v2_typed_array` carrier classification.
    let result = eval(
        "type Bag { xs: Array<number> }\n\
         let b = Bag { xs: [1.5, 2.5, 3.5] }\n\
         b.xs[2]",
    );
    assert_eq!(result.as_f64(), Some(3.5));
}

#[test]
fn test_closure_captures_array_sum() {
    // Trigger path 2: a closure capturing an `Array<int>`. Pre-fix:
    // VM panic at `clone_with_kind` on the `Ptr(HeapKind::TypedArray)`
    // capture (and JIT SIGABRT).
    let result = eval(
        "let data = [10, 20, 30]\n\
         let f = || data.sum()\n\
         f()",
    );
    assert_eq!(result.as_i64(), Some(60));
}

#[test]
fn test_closure_captures_array_called_twice() {
    // Calling the array-capturing closure more than once must keep the
    // captured array's refcount balanced — no premature free.
    let result = eval(
        "let data = [10, 20, 30]\n\
         let f = || data.sum()\n\
         f() + f()",
    );
    assert_eq!(result.as_i64(), Some(120));
}

#[test]
fn test_struct_array_field_drop_balance_in_loop() {
    // Construct + read a struct array field repeatedly. A retain/release
    // imbalance in the `Ptr(HeapKind::TypedArray)` clone/drop arms would
    // surface as a leak (refcount drift) or a double-free abort here.
    let result = eval(
        "type Bag { items: Array<int> }\n\
         fn make_and_read() -> int {\n\
         \x20 let b = Bag { items: [7, 8, 9] }\n\
         \x20 return b.items[1]\n\
         }\n\
         let mut total = 0\n\
         for i in 0..50 {\n\
         \x20 total = total + make_and_read()\n\
         }\n\
         total",
    );
    assert_eq!(result.as_i64(), Some(400));
}
