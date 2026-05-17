//! Integration tests for typed struct field access — end-to-end.
//!
//! These tests verify the full pipeline: parser → type inference →
//! bytecode compiler (with typed field opcodes) → VM execution.
//!
//! Phase-2c surface: every test body in this file consumed the deleted
//! `ValueWord` carrier via `eval(...).as_i64()` / `eval(...).to_number()`.
//! The host-tier `eval` API itself returns `ValueWord` from the deleted
//! shape-value carrier (see `crate::test_utils`); bodies replaced with
//! `todo!()` per playbook §7 REVISED part 4 + ADR-006 §2.7.4 (host-tier
//! `eval`/marshal API rebuild). Restore once `eval_typed_*` convenience
//! helpers are wired through the kinded-bits return path.

// ===== Basic struct field access =====

#[test]
fn test_typed_struct_field_access_number() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_struct_field_access_single() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_struct_with_int_fields() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_struct_mutation() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Multiple struct types =====

#[test]
fn test_multiple_struct_types() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Struct construction and return =====

#[test]
fn test_typed_struct_in_function() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Struct with mixed field types =====

#[test]
fn test_typed_struct_mixed_fields() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_struct_mixed_fields_number() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Struct arithmetic =====

#[test]
fn test_typed_struct_distance_calc() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Regression: existing TypedObject path still works =====

#[test]
fn test_anonymous_object_still_works() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}
