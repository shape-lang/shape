//! Integration tests for typed struct field access — end-to-end.
//!
//! These tests verify the full pipeline: parser → type inference →
//! bytecode compiler (with typed field opcodes) → VM execution.

use super::test_utils::{eval, eval_result};
use shape_value::ValueWordExt;

// ===== Basic struct field access =====

#[test]
fn test_typed_struct_field_access_number() {
    let result = eval(
        "type Point { x: number, y: number }
         let p = Point { x: 3.0, y: 4.0 }
         p.x + p.y",
    );
    assert!((result.to_number().unwrap() - 7.0).abs() < 1e-10);
}

#[test]
fn test_typed_struct_field_access_single() {
    let result = eval(
        "type Point { x: number, y: number }
         let p = Point { x: 10.0, y: 20.0 }
         p.y",
    );
    assert!((result.to_number().unwrap() - 20.0).abs() < 1e-10);
}

#[test]
fn test_typed_struct_with_int_fields() {
    let result = eval(
        "type Rect { width: int, height: int }
         let r = Rect { width: 10, height: 20 }
         r.width * r.height",
    );
    assert_eq!(result.as_i64(), Some(200));
}

#[test]
fn test_typed_struct_mutation() {
    let result = eval(
        "type Point { x: number, y: number }
         let mut p = Point { x: 1.0, y: 2.0 }
         p.x = 10.0
         p.x",
    );
    assert!((result.to_number().unwrap() - 10.0).abs() < 1e-10);
}

// ===== Multiple struct types =====

#[test]
fn test_multiple_struct_types() {
    let result = eval(
        "type Vec2 { x: number, y: number }
         type Color { r: int, g: int, b: int }
         let v = Vec2 { x: 1.0, y: 2.0 }
         let c = Color { r: 255, g: 128, b: 0 }
         v.x + v.y",
    );
    assert!((result.to_number().unwrap() - 3.0).abs() < 1e-10);
}

// ===== Struct construction and return =====

#[test]
fn test_typed_struct_in_function() {
    let result = eval(
        "type Point { x: number, y: number }
         fn make_point(x: number, y: number) -> Point {
             Point { x: x, y: y }
         }
         let p = make_point(5.0, 12.0)
         p.x + p.y",
    );
    assert!((result.to_number().unwrap() - 17.0).abs() < 1e-10);
}

// ===== Struct with mixed field types =====

#[test]
fn test_typed_struct_mixed_fields() {
    let result = eval(
        "type Config { name: string, value: number, count: int }
         let c = Config { name: \"test\", value: 3.14, count: 42 }
         c.count",
    );
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_typed_struct_mixed_fields_number() {
    let result = eval(
        "type Config { name: string, value: number, count: int }
         let c = Config { name: \"test\", value: 3.14, count: 42 }
         c.value",
    );
    assert!((result.to_number().unwrap() - 3.14).abs() < 1e-10);
}

// ===== Struct arithmetic =====

#[test]
fn test_typed_struct_distance_calc() {
    let result = eval(
        "type Point { x: number, y: number }
         let a = Point { x: 0.0, y: 0.0 }
         let b = Point { x: 3.0, y: 4.0 }
         let dx = b.x - a.x
         let dy = b.y - a.y
         dx * dx + dy * dy",
    );
    assert!((result.to_number().unwrap() - 25.0).abs() < 1e-10);
}

// ===== Regression: existing TypedObject path still works =====

#[test]
fn test_anonymous_object_still_works() {
    let result = eval(
        "let obj = { x: 1, y: 2 }
         obj.x + obj.y",
    );
    assert_eq!(result.as_i64(), Some(3));
}
