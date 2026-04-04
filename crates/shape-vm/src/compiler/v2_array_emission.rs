//! Typed array emission helpers for the v2 runtime.
//!
//! This module provides inference functions that determine whether an array
//! literal or annotated variable can use typed array opcodes instead of
//! generic (NaN-boxed) array operations.
//!
//! These are pure query functions -- they do NOT modify compilation state.
//! Integration into the actual opcode emission paths will happen separately.

use crate::type_tracking::{SlotKind, TypeTracker};
use shape_ast::ast::{Expr, Literal, TypeAnnotation};

/// Check if an array literal has a proven homogeneous element type.
///
/// Returns `Some(SlotKind)` when every element in the array is provably the
/// same scalar type, allowing the compiler to emit a typed `NewArray` variant.
///
/// Rules:
/// - All `Number` literals -> `Float64`
/// - All `Int` literals -> `Int64`
/// - All `Bool` literals -> `Bool`
/// - All `String` literals -> `String`
/// - All elements have the same tracked `storage_hint` in the type tracker -> that kind
/// - Empty array -> `None` (element type unknown)
/// - Heterogeneous or unresolvable -> `None`
pub fn infer_array_element_type(elements: &[Expr], type_tracker: &TypeTracker) -> Option<SlotKind> {
    if elements.is_empty() {
        return None;
    }

    // First pass: try to resolve purely from literal types.
    // This is the fastest path and covers array-literal expressions like `[1, 2, 3]`.
    if let Some(kind) = infer_from_literals(elements) {
        return Some(kind);
    }

    // Second pass: try to resolve from tracked variable types.
    // Handles cases like `[a, b, c]` where all identifiers have known storage hints.
    infer_from_tracked_types(elements, type_tracker)
}

/// Check if a variable's type annotation specifies a typed array.
///
/// Recognizes both `Array<T>` (Generic form) and `T[]` (Array form) annotations.
///
/// Examples:
/// - `let arr: Array<number>` -> `Some(Float64)`
/// - `let arr: Array<int>` -> `Some(Int64)`
/// - `let arr: Array<i32>` -> `Some(Int32)`
/// - `let arr: Array<bool>` -> `Some(Bool)`
/// - `let arr: Array<string>` -> `Some(String)`
/// - `let arr: number[]` -> `Some(Float64)`
/// - `let arr: Array<SomeStruct>` -> `None`
pub fn typed_array_from_annotation(annotation: &TypeAnnotation) -> Option<SlotKind> {
    match annotation {
        // `Array<T>` form
        TypeAnnotation::Generic { name, args } if *name == "Array" && args.len() == 1 => {
            scalar_annotation_to_slot_kind(&args[0])
        }
        // `T[]` form (desugars to TypeAnnotation::Array)
        TypeAnnotation::Array(inner) => scalar_annotation_to_slot_kind(inner),
        _ => None,
    }
}

/// Map a scalar type annotation to a `SlotKind`.
///
/// Only maps types that have a direct v2 typed representation.
/// Returns `None` for compound types, user-defined types, etc.
fn scalar_annotation_to_slot_kind(annotation: &TypeAnnotation) -> Option<SlotKind> {
    match annotation {
        TypeAnnotation::Basic(name) => match name.as_str() {
            "number" => Some(SlotKind::Float64),
            "int" => Some(SlotKind::Int64),
            "i8" => Some(SlotKind::Int8),
            "u8" => Some(SlotKind::UInt8),
            "i16" => Some(SlotKind::Int16),
            "u16" => Some(SlotKind::UInt16),
            "i32" => Some(SlotKind::Int32),
            "u32" => Some(SlotKind::UInt32),
            "u64" => Some(SlotKind::UInt64),
            "isize" => Some(SlotKind::IntSize),
            "usize" => Some(SlotKind::UIntSize),
            "bool" => Some(SlotKind::Bool),
            "string" => Some(SlotKind::String),
            _ => None,
        },
        _ => None,
    }
}

/// Attempt to infer a homogeneous element type purely from literal nodes.
fn infer_from_literals(elements: &[Expr]) -> Option<SlotKind> {
    let mut kind: Option<SlotKind> = None;

    for elem in elements {
        let elem_kind = match elem {
            Expr::Literal(Literal::Number(_), _) => SlotKind::Float64,
            Expr::Literal(Literal::Int(_), _) => SlotKind::Int64,
            Expr::Literal(Literal::Bool(_), _) => SlotKind::Bool,
            Expr::Literal(Literal::String(_), _) => SlotKind::String,
            Expr::Literal(Literal::TypedInt(_, w), _) => typed_int_width_to_slot(*w),
            // Non-literal or unsupported literal -- can't infer from literals alone.
            _ => return None,
        };

        match kind {
            Some(prev) if prev != elem_kind => return None, // heterogeneous
            Some(_) => {}                                    // same, continue
            None => kind = Some(elem_kind),
        }
    }

    kind
}

/// Map an `IntWidth` to the corresponding `SlotKind`.
fn typed_int_width_to_slot(w: shape_ast::IntWidth) -> SlotKind {
    use shape_ast::IntWidth;
    match w {
        IntWidth::I8 => SlotKind::Int8,
        IntWidth::U8 => SlotKind::UInt8,
        IntWidth::I16 => SlotKind::Int16,
        IntWidth::U16 => SlotKind::UInt16,
        IntWidth::I32 => SlotKind::Int32,
        IntWidth::U32 => SlotKind::UInt32,
        IntWidth::U64 => SlotKind::UInt64,
    }
}

/// Attempt to infer a homogeneous element type from type-tracked identifiers.
///
/// Only succeeds when every element is an `Identifier` whose local slot has a
/// known, non-`Unknown` `storage_hint`, and all those hints are equal.
fn infer_from_tracked_types(elements: &[Expr], type_tracker: &TypeTracker) -> Option<SlotKind> {
    let mut kind: Option<SlotKind> = None;

    for elem in elements {
        let elem_kind = expr_storage_hint(elem, type_tracker)?;
        if elem_kind == SlotKind::Unknown || elem_kind == SlotKind::NanBoxed {
            return None;
        }

        match kind {
            Some(prev) if prev != elem_kind => return None,
            Some(_) => {}
            None => kind = Some(elem_kind),
        }
    }

    kind
}

/// Try to get the storage hint for an expression from the type tracker.
///
/// Currently only resolves `Identifier` expressions (local variables).
/// Could be extended to handle more expression forms in the future.
fn expr_storage_hint(expr: &Expr, type_tracker: &TypeTracker) -> Option<SlotKind> {
    // For identifiers, we'd need the local slot index, which isn't available
    // from the AST alone. This path requires cooperation from the compiler
    // to resolve names -> slots. For now, only literal-based inference is
    // fully self-contained. This function is a placeholder for the future
    // integration point.
    //
    // When the compiler calls `infer_array_element_type`, it can pre-resolve
    // identifiers to slots and use `type_tracker.get_local_storage_hint(slot)`
    // before calling this module.
    let _ = (expr, type_tracker);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn span() -> Span {
        Span::default()
    }

    fn num_lit(v: f64) -> Expr {
        Expr::Literal(Literal::Number(v), span())
    }

    fn int_lit(v: i64) -> Expr {
        Expr::Literal(Literal::Int(v), span())
    }

    fn bool_lit(v: bool) -> Expr {
        Expr::Literal(Literal::Bool(v), span())
    }

    fn string_lit(s: &str) -> Expr {
        Expr::Literal(Literal::String(s.to_string()), span())
    }

    fn typed_int_lit(v: i64, w: shape_ast::IntWidth) -> Expr {
        Expr::Literal(Literal::TypedInt(v, w), span())
    }

    fn ident(name: &str) -> Expr {
        Expr::Identifier(name.to_string(), span())
    }

    fn tracker() -> TypeTracker {
        TypeTracker::empty()
    }

    // ---------------------------------------------------------------
    // infer_array_element_type
    // ---------------------------------------------------------------

    #[test]
    fn test_empty_array_returns_none() {
        let tt = tracker();
        assert_eq!(infer_array_element_type(&[], &tt), None);
    }

    #[test]
    fn test_all_numbers() {
        let tt = tracker();
        let elems = vec![num_lit(1.0), num_lit(2.5), num_lit(3.14)];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::Float64)
        );
    }

    #[test]
    fn test_all_ints() {
        let tt = tracker();
        let elems = vec![int_lit(1), int_lit(2), int_lit(3)];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::Int64)
        );
    }

    #[test]
    fn test_all_bools() {
        let tt = tracker();
        let elems = vec![bool_lit(true), bool_lit(false), bool_lit(true)];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::Bool)
        );
    }

    #[test]
    fn test_all_strings() {
        let tt = tracker();
        let elems = vec![string_lit("a"), string_lit("b")];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::String)
        );
    }

    #[test]
    fn test_all_typed_i32() {
        let tt = tracker();
        let elems = vec![
            typed_int_lit(1, shape_ast::IntWidth::I32),
            typed_int_lit(2, shape_ast::IntWidth::I32),
        ];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::Int32)
        );
    }

    #[test]
    fn test_mixed_int_and_number_returns_none() {
        let tt = tracker();
        let elems = vec![int_lit(1), num_lit(2.0)];
        assert_eq!(infer_array_element_type(&elems, &tt), None);
    }

    #[test]
    fn test_mixed_literals_and_identifiers_returns_none() {
        // Identifiers can't be resolved to slots without compiler context,
        // so the literal path bails and the tracked-type path also returns None.
        let tt = tracker();
        let elems = vec![int_lit(1), ident("x")];
        assert_eq!(infer_array_element_type(&elems, &tt), None);
    }

    #[test]
    fn test_single_element_number() {
        let tt = tracker();
        let elems = vec![num_lit(42.0)];
        assert_eq!(
            infer_array_element_type(&elems, &tt),
            Some(SlotKind::Float64)
        );
    }

    #[test]
    fn test_mixed_typed_int_widths_returns_none() {
        let tt = tracker();
        let elems = vec![
            typed_int_lit(1, shape_ast::IntWidth::I32),
            typed_int_lit(2, shape_ast::IntWidth::U8),
        ];
        assert_eq!(infer_array_element_type(&elems, &tt), None);
    }

    #[test]
    fn test_all_identifiers_without_tracking_returns_none() {
        let tt = tracker();
        let elems = vec![ident("a"), ident("b")];
        assert_eq!(infer_array_element_type(&elems, &tt), None);
    }

    // ---------------------------------------------------------------
    // typed_array_from_annotation
    // ---------------------------------------------------------------

    #[test]
    fn test_annotation_array_number() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Float64));
    }

    #[test]
    fn test_annotation_array_int() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Int64));
    }

    #[test]
    fn test_annotation_array_i32() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("i32".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Int32));
    }

    #[test]
    fn test_annotation_array_bool() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("bool".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Bool));
    }

    #[test]
    fn test_annotation_array_string() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("string".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::String));
    }

    #[test]
    fn test_annotation_array_u8() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("u8".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::UInt8));
    }

    #[test]
    fn test_annotation_array_sugar_number() {
        // T[] syntax -> TypeAnnotation::Array(Box<T>)
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("number".to_string())));
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Float64));
    }

    #[test]
    fn test_annotation_array_sugar_int() {
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        assert_eq!(typed_array_from_annotation(&ann), Some(SlotKind::Int64));
    }

    #[test]
    fn test_annotation_array_custom_type_returns_none() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("Point".to_string())],
        };
        assert_eq!(typed_array_from_annotation(&ann), None);
    }

    #[test]
    fn test_annotation_non_array_generic_returns_none() {
        use shape_ast::ast::type_path::TypePath;
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("HashMap"),
            args: vec![
                TypeAnnotation::Basic("string".to_string()),
                TypeAnnotation::Basic("int".to_string()),
            ],
        };
        assert_eq!(typed_array_from_annotation(&ann), None);
    }

    #[test]
    fn test_annotation_basic_type_returns_none() {
        let ann = TypeAnnotation::Basic("number".to_string());
        assert_eq!(typed_array_from_annotation(&ann), None);
    }

    #[test]
    fn test_annotation_array_nested_generic_returns_none() {
        use shape_ast::ast::type_path::TypePath;
        // Array<Array<int>> -- inner type is not a scalar Basic
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Basic("int".to_string())],
            }],
        };
        assert_eq!(typed_array_from_annotation(&ann), None);
    }

    // ---------------------------------------------------------------
    // scalar_annotation_to_slot_kind (indirect via typed_array_from_annotation)
    // ---------------------------------------------------------------

    #[test]
    fn test_all_scalar_widths_via_annotation() {
        use shape_ast::ast::type_path::TypePath;
        let cases = vec![
            ("number", SlotKind::Float64),
            ("int", SlotKind::Int64),
            ("i8", SlotKind::Int8),
            ("u8", SlotKind::UInt8),
            ("i16", SlotKind::Int16),
            ("u16", SlotKind::UInt16),
            ("i32", SlotKind::Int32),
            ("u32", SlotKind::UInt32),
            ("u64", SlotKind::UInt64),
            ("isize", SlotKind::IntSize),
            ("usize", SlotKind::UIntSize),
            ("bool", SlotKind::Bool),
            ("string", SlotKind::String),
        ];
        for (type_name, expected_kind) in cases {
            let ann = TypeAnnotation::Generic {
                name: TypePath::simple("Array"),
                args: vec![TypeAnnotation::Basic(type_name.to_string())],
            };
            assert_eq!(
                typed_array_from_annotation(&ann),
                Some(expected_kind),
                "Array<{type_name}> should map to {expected_kind:?}"
            );
        }
    }

    // ---------------------------------------------------------------
    // typed_int_width_to_slot
    // ---------------------------------------------------------------

    #[test]
    fn test_typed_int_width_mapping() {
        use shape_ast::IntWidth;
        assert_eq!(typed_int_width_to_slot(IntWidth::I8), SlotKind::Int8);
        assert_eq!(typed_int_width_to_slot(IntWidth::U8), SlotKind::UInt8);
        assert_eq!(typed_int_width_to_slot(IntWidth::I16), SlotKind::Int16);
        assert_eq!(typed_int_width_to_slot(IntWidth::U16), SlotKind::UInt16);
        assert_eq!(typed_int_width_to_slot(IntWidth::I32), SlotKind::Int32);
        assert_eq!(typed_int_width_to_slot(IntWidth::U32), SlotKind::UInt32);
        assert_eq!(typed_int_width_to_slot(IntWidth::U64), SlotKind::UInt64);
    }
}
