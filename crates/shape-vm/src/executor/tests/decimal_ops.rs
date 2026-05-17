//! Decimal type operation tests
//!
//! Tests for Decimal support: toString, toFixed, arithmetic (mod),
//! TypedObject storage/retrieval, method dispatch, and struct schema.

// Phase-2c surface (W11): every test body in this file consumed
// the deleted `ValueWord` carrier (`as_decimal`, `as_arc_string`,
// `as_i64` on `ValueWord`). Per playbook §7 REVISED part 4 +
// ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted
// host-tier carriers), bodies are surfaced as `todo!()` until
// Phase-2c restores the kinded host-tier carriers.

#[test]
fn test_decimal_to_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_decimal_to_fixed() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_decimal_mod() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_decimal_round_trip_through_f64() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_decimal_neg() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_struct_decimal_field_preserves_type() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

#[test]
fn test_struct_int_field_preserves_type() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord carrier)")
}

