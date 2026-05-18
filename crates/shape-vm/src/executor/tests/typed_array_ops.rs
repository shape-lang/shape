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
