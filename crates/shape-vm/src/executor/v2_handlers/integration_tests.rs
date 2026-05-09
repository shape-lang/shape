//! End-to-end integration tests for typed arrays.
//!
//! These tests compile Shape source code and execute it, verifying that
//! typed array operations produce correct results through the full pipeline:
//! parse → compile → bytecode → VM execution.
//!
//! Currently these exercise the v1 typed array path (NewTypedArray) since the
//! bytecode compiler does not yet emit v2 typed array opcodes. The v2 path
//! (NewTypedArrayF64 etc.) is tested via direct bytecode tests in v2_opcode_tests.
//!
//! Phase-2c surface: every test body in this file consumed the deleted
//! `ValueWord` carrier via `eval(...).as_i64()` / `eval(...).to_number()`.
//! The host-tier `eval` API itself returns `ValueWord` from the deleted
//! shape-value carrier (see `crate::test_utils`); bodies replaced with
//! `todo!()` per playbook §7 REVISED part 4 + ADR-006 §2.7.4 (host-tier
//! `eval`/marshal API rebuild). Restore once `eval_typed_*` convenience
//! helpers are wired through the kinded-bits return path.

// ===== Array<number> (f64) =====

#[test]
fn test_typed_array_f64_literal_sum() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_f64_index_access() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_f64_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Array<int> (i64) =====

#[test]
fn test_typed_array_int_literal_sum() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_int_index_access() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_int_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_int_first_last() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Array mutation =====

#[test]
fn test_typed_array_push_and_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Array iteration =====

#[test]
fn test_typed_array_for_in_accumulate() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Array methods =====

#[test]
fn test_typed_array_map() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_typed_array_filter() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Error cases =====

#[test]
fn test_typed_array_out_of_bounds() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval_result/marshal API rebuild)")
}

#[test]
fn test_typed_array_negative_index() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval_result/marshal API rebuild)")
}

// ===== Empty arrays =====

#[test]
fn test_empty_array_len() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== Mixed operations =====

#[test]
fn test_typed_array_dot_product() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== New end-to-end v2 typed array demos =====

#[test]
fn test_v2_typed_array_length_property() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_v2_typed_array_for_in_iteration() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_v2_typed_array_index_assignment_roundtrip() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}
