//! Vec<T> typed array integration tests — bytecode-level tests for typed array
//! construction, SIMD arithmetic, and method dispatch.
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

// Phase-2c surface (W11): every test body in this file consumed
// the deleted `ValueWord` carrier (`ValueWord::from_i64`,
// `ValueWord::from_*_array`, `Constant::Value(ValueWord)`, etc.).
// Per playbook §7 REVISED part 4 + ADR-006 §2.7.4 (host-tier
// eval/marshal API rebuild — deleted host-tier carriers), bodies
// are surfaced as `todo!()` until Phase-2c restores the kinded
// host-tier carriers. Restore once the kinded constant-table /
// kinded marshal layer lands.

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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_avg() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_min() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_max() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_float_array_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_avg() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_min() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_max() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_int_array_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
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

