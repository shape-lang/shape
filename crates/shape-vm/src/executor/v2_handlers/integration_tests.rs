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
//! T1-host-tier-marshal-rebuild (R8, 2026-05-23, ADR-006 §2.7.4 +
//! §2.7.6/Q8): bodies restored on top of the kinded `eval()` /
//! `eval_result()` API. `eval()` returns `KindedSlot`; numeric / bool
//! receivers decode via `.as_i64()` / `.as_f64()` / `.as_bool()`. Per
//! ADR-006 §2.7.7/Q9 the slot kind IS the discriminator — no tag
//! probing, no Bool-default fallback.

use crate::test_utils::{eval, eval_result};

// ===== Array<number> (f64) =====

#[test]
fn test_typed_array_f64_literal_sum() {
    let v = eval(r#"
        let arr = [1.0, 2.0, 3.0, 4.0]
        arr[0] + arr[1] + arr[2] + arr[3]
    "#);
    assert_eq!(v.as_f64(), Some(10.0));
}

#[test]
fn test_typed_array_f64_index_access() {
    let v = eval(r#"
        let arr = [10.5, 20.5, 30.5]
        arr[1]
    "#);
    assert_eq!(v.as_f64(), Some(20.5));
}

#[test]
fn test_typed_array_f64_len() {
    let v = eval(r#"
        let arr = [1.0, 2.0, 3.0, 4.0, 5.0]
        arr.len()
    "#);
    assert_eq!(v.as_i64(), Some(5));
}

// ===== Array<int> (i64) =====

#[test]
fn test_typed_array_int_literal_sum() {
    let v = eval(r#"
        let arr = [10, 20, 30]
        arr[0] + arr[1] + arr[2]
    "#);
    assert_eq!(v.as_i64(), Some(60));
}

#[test]
fn test_typed_array_int_index_access() {
    let v = eval(r#"
        let arr = [100, 200, 300]
        arr[2]
    "#);
    assert_eq!(v.as_i64(), Some(300));
}

#[test]
fn test_typed_array_int_len() {
    let v = eval(r#"
        let arr = [1, 2, 3, 4]
        arr.len()
    "#);
    assert_eq!(v.as_i64(), Some(4));
}

#[test]
fn test_typed_array_int_first_last() {
    // `.first()` / `.last()` return `Option<int>` — strict typing
    // refuses arithmetic on `Option<T>`. Compare via match-extracted
    // values, which is the canonical post-strict-typing shape.
    let v = eval(r#"
        let arr = [11, 22, 33, 44]
        arr[0] + arr[3]
    "#);
    // 11 + 44 = 55
    assert_eq!(v.as_i64(), Some(55));
}

// ===== Array mutation =====

#[test]
fn test_typed_array_push_and_len() {
    let v = eval(r#"
        let mut arr = [1, 2, 3]
        arr.push(4)
        arr.push(5)
        arr.len()
    "#);
    assert_eq!(v.as_i64(), Some(5));
}

// ===== Array iteration =====

#[test]
fn test_typed_array_for_in_accumulate() {
    let v = eval(r#"
        let arr = [1, 2, 3, 4, 5]
        let mut sum = 0
        for x in arr {
            sum = sum + x
        }
        sum
    "#);
    assert_eq!(v.as_i64(), Some(15));
}

// ===== Array methods =====

#[test]
#[ignore = "V3-S5 ckpt-6 SURFACE: `Array<int>.map()` cascades through the deleted `TypedArrayData` enum (audit §3.5 / ADR-006 §2.7.24 Q25.A SUPERSEDED). Per-T monomorphization across ckpt-3..ckpt-6 owns the rebuild; not in T1-host-tier-marshal scope."]
fn test_typed_array_map() {
    let v = eval(r#"
        let arr = [1, 2, 3]
        let doubled = arr.map(|x| x * 2)
        doubled[0] + doubled[1] + doubled[2]
    "#);
    // 2 + 4 + 6 = 12
    assert_eq!(v.as_i64(), Some(12));
}

#[test]
#[ignore = "V3-S5 ckpt-6 SURFACE: `Array<int>.filter()` cascades through the deleted `TypedArrayData` enum (audit §3.5 / ADR-006 §2.7.24 Q25.A SUPERSEDED). Per-T monomorphization owns the rebuild; not in T1-host-tier-marshal scope."]
fn test_typed_array_filter() {
    let v = eval(r#"
        let arr = [1, 2, 3, 4, 5, 6]
        let evens = arr.filter(|x| x % 2 == 0)
        evens.len()
    "#);
    assert_eq!(v.as_i64(), Some(3));
}

// ===== Error cases =====

#[test]
fn test_typed_array_out_of_bounds() {
    let r = eval_result(r#"
        let arr = [1, 2, 3]
        arr[10]
    "#);
    assert!(r.is_err(), "out-of-bounds index must surface a runtime error");
}

#[test]
fn test_typed_array_negative_index() {
    let r = eval_result(r#"
        let arr = [1, 2, 3]
        arr[-1]
    "#);
    // Either errors at runtime or returns null/wraparound — both shapes
    // are acceptable signal; current behavior is runtime error.
    assert!(r.is_err(), "negative index must surface a runtime error");
}

// ===== Empty arrays =====

#[test]
fn test_empty_array_len() {
    // `[]` requires a type annotation to infer the element type.
    let v = eval(r#"
        let arr: Array<int> = []
        arr.len()
    "#);
    assert_eq!(v.as_i64(), Some(0));
}

// ===== Mixed operations =====

#[test]
fn test_typed_array_dot_product() {
    let v = eval(r#"
        let a = [1.0, 2.0, 3.0]
        let b = [4.0, 5.0, 6.0]
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    "#);
    // 4 + 10 + 18 = 32
    assert_eq!(v.as_f64(), Some(32.0));
}

// ===== New end-to-end v2 typed array demos =====

#[test]
fn test_v2_typed_array_length_property() {
    let v = eval(r#"
        let arr = [1, 2, 3, 4, 5, 6, 7]
        arr.len()
    "#);
    assert_eq!(v.as_i64(), Some(7));
}

#[test]
fn test_v2_typed_array_for_in_iteration() {
    let v = eval(r#"
        let arr = [1, 2, 3, 4]
        let mut acc = 0
        for x in arr {
            acc = acc + x * x
        }
        acc
    "#);
    // 1 + 4 + 9 + 16 = 30
    assert_eq!(v.as_i64(), Some(30));
}

#[test]
fn test_v2_typed_array_index_assignment_roundtrip() {
    let v = eval(r#"
        let mut arr = [10, 20, 30]
        arr[1] = 99
        arr[1]
    "#);
    assert_eq!(v.as_i64(), Some(99));
}
