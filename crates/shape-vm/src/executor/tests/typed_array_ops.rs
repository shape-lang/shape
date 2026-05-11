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

