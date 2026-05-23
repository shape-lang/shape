//! Integration tests for typed struct field access — end-to-end.
//!
//! These tests verify the full pipeline: parser → type inference →
//! bytecode compiler (with typed field opcodes) → VM execution.
//!
//! T1-host-tier-marshal-rebuild (R8, 2026-05-23, ADR-006 §2.7.4 +
//! §2.7.6/Q8): bodies restored on top of the kinded `eval()` API.
//! `eval()` returns `KindedSlot`; `.as_i64()` / `.as_f64()` /
//! `.as_bool()` decode per kind discriminator. No tag probing, no
//! Bool-default fallback per §2.7.7/Q9.

use crate::test_utils::eval;

// ===== Basic struct field access =====

#[test]
fn test_typed_struct_field_access_number() {
    let v = eval(r#"
        type Point { x: number, y: number }
        let p = Point { x: 3.5, y: 4.5 }
        p.x
    "#);
    assert_eq!(v.as_f64(), Some(3.5));
}

#[test]
fn test_typed_struct_field_access_single() {
    let v = eval(r#"
        type Single { v: number }
        let s = Single { v: 42.0 }
        s.v
    "#);
    assert_eq!(v.as_f64(), Some(42.0));
}

#[test]
fn test_typed_struct_with_int_fields() {
    let v = eval(r#"
        type Counter { count: int }
        let c = Counter { count: 100 }
        c.count
    "#);
    assert_eq!(v.as_i64(), Some(100));
}

#[test]
fn test_typed_struct_mutation() {
    let v = eval(r#"
        type Counter { count: int }
        let mut c = Counter { count: 1 }
        c.count = 99
        c.count
    "#);
    assert_eq!(v.as_i64(), Some(99));
}

// ===== Multiple struct types =====

#[test]
fn test_multiple_struct_types() {
    let v = eval(r#"
        type A { v: int }
        type B { v: int }
        let a = A { v: 10 }
        let b = B { v: 20 }
        a.v + b.v
    "#);
    assert_eq!(v.as_i64(), Some(30));
}

// ===== Struct construction and return =====

#[test]
fn test_typed_struct_in_function() {
    let v = eval(r#"
        type Pair { first: int, second: int }
        fn make() -> Pair {
            Pair { first: 7, second: 13 }
        }
        let p = make()
        p.first + p.second
    "#);
    assert_eq!(v.as_i64(), Some(20));
}

// ===== Struct with mixed field types =====

#[test]
fn test_typed_struct_mixed_fields() {
    let v = eval(r#"
        type Record { id: int, score: number, active: bool }
        let r = Record { id: 7, score: 99.5, active: true }
        r.id
    "#);
    assert_eq!(v.as_i64(), Some(7));
}

#[test]
fn test_typed_struct_mixed_fields_number() {
    let v = eval(r#"
        type Record { id: int, score: number, active: bool }
        let r = Record { id: 7, score: 99.5, active: true }
        r.score
    "#);
    assert_eq!(v.as_f64(), Some(99.5));
}

// ===== Struct arithmetic =====

#[test]
fn test_typed_struct_distance_calc() {
    let v = eval(r#"
        type Vec2 { x: number, y: number }
        let a = Vec2 { x: 3.0, y: 4.0 }
        a.x * a.x + a.y * a.y
    "#);
    // 9 + 16 = 25
    assert_eq!(v.as_f64(), Some(25.0));
}

// ===== Regression: existing TypedObject path still works =====

#[test]
fn test_anonymous_object_still_works() {
    let v = eval(r#"
        let obj = { x: 10, y: 20 }
        obj.x + obj.y
    "#);
    assert_eq!(v.as_i64(), Some(30));
}
