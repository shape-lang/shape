//! Unit tests for v2 opcode execution in the VM interpreter.
//!
//! Tests exercise typed array, typed field, and sized integer (i32) opcodes
//! by constructing raw instruction sequences. The compiler does not yet emit
//! these opcodes, so we build them manually.

// Phase-2c surface (W11): every test body in this file consumed
// the deleted `ValueWord` carrier (`ValueWord::from_i64`,
// `ValueWord::from_*_array`, `Constant::Value(ValueWord)`, etc.).
// Per playbook §7 REVISED part 4 + ADR-006 §2.7.4 (host-tier
// eval/marshal API rebuild — deleted host-tier carriers), bodies
// are surfaced as `todo!()` until Phase-2c restores the kinded
// host-tier carriers. Restore once the kinded constant-table /
// kinded marshal layer lands.

#[test]
fn test_v2_typed_array_f64_create_push_get() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_typed_array_f64_set() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_typed_array_f64_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_typed_array_i64_create_push_get() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_typed_array_f64_out_of_bounds() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_typed_array_i64_out_of_bounds() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_field_store_load_f64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_field_store_load_i64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_field_store_load_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_field_load_bool_default_zero() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_field_multiple_fields() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_new_typed_struct_sets_refcount_and_kind() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_add_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_sub_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_mul_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_div_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_mod_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_div_i32_by_zero() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_mod_i32_by_zero() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_i32_overflow_wraps() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_i32_underflow_wraps() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_eq_i32_true() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_eq_i32_false() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_neq_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_lt_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_gt_i32() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_lte_i32_equal() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_gte_i32_equal() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

#[test]
fn test_v2_lt_i32_negative() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")
}

