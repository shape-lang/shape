//! Structural Type Equality
//!
//! Implements proper structural equality for types, replacing the string-based
//! comparison with actual type structure comparison.

use crate::type_system::{Type, TypeConstraint};

// TypeVar is used in tests below
use shape_ast::ast::TypeAnnotation;

/// Check if two types are structurally equal
///
/// This function replaces the previous `format!("{:?}", a) == format!("{:?}", b)`
/// comparison with proper structural equality.
pub fn types_equal(a: &Type, b: &Type) -> bool {
    match (a, b) {
        // Variable equality
        (Type::Variable(v1), Type::Variable(v2)) => v1 == v2,

        // Concrete type equality
        (Type::Concrete(ann1), Type::Concrete(ann2)) => annotations_equal(ann1, ann2),

        // Generic type equality
        (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) => {
            if a1.len() != a2.len() {
                return false;
            }
            types_equal(b1, b2) && a1.iter().zip(a2.iter()).all(|(t1, t2)| types_equal(t1, t2))
        }

        // Constrained type equality
        (
            Type::Constrained {
                var: v1,
                constraint: c1,
            },
            Type::Constrained {
                var: v2,
                constraint: c2,
            },
        ) => v1 == v2 && constraints_equal(c1, c2),

        // Function type equality
        (
            Type::Function {
                params: p1,
                returns: r1,
            },
            Type::Function {
                params: p2,
                returns: r2,
            },
        ) => {
            p1.len() == p2.len()
                && p1.iter().zip(p2.iter()).all(|(a, b)| types_equal(a, b))
                && types_equal(r1, r2)
        }

        // Different type kinds are not equal
        _ => false,
    }
}

/// Check if two type annotations are structurally equal
pub fn annotations_equal(a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
    match (a, b) {
        // Basic types
        (TypeAnnotation::Basic(n1), TypeAnnotation::Basic(n2)) => n1 == n2,

        // Reference types
        (TypeAnnotation::Reference(n1), TypeAnnotation::Reference(n2)) => n1 == n2,

        // Array types
        (TypeAnnotation::Array(e1), TypeAnnotation::Array(e2)) => annotations_equal(e1, e2),

        // Tuple types
        (TypeAnnotation::Tuple(t1), TypeAnnotation::Tuple(t2)) => {
            t1.len() == t2.len()
                && t1
                    .iter()
                    .zip(t2.iter())
                    .all(|(a1, a2)| annotations_equal(a1, a2))
        }

        // Object types
        (TypeAnnotation::Object(f1), TypeAnnotation::Object(f2)) => {
            if f1.len() != f2.len() {
                return false;
            }
            // Check that all fields match (order matters for now)
            f1.iter().zip(f2.iter()).all(|(field1, field2)| {
                field1.name == field2.name
                    && field1.optional == field2.optional
                    && annotations_equal(&field1.type_annotation, &field2.type_annotation)
            })
        }

        // Function types
        (
            TypeAnnotation::Function {
                params: p1,
                returns: r1,
            },
            TypeAnnotation::Function {
                params: p2,
                returns: r2,
            },
        ) => {
            if p1.len() != p2.len() {
                return false;
            }
            let params_equal = p1.iter().zip(p2.iter()).all(|(param1, param2)| {
                param1.optional == param2.optional
                    && annotations_equal(&param1.type_annotation, &param2.type_annotation)
            });
            params_equal && annotations_equal(r1, r2)
        }

        // Union types (order-independent)
        (TypeAnnotation::Union(u1), TypeAnnotation::Union(u2)) => {
            if u1.len() != u2.len() {
                return false;
            }
            // Check that every type in u1 exists in u2
            u1.iter()
                .all(|t1| u2.iter().any(|t2| annotations_equal(t1, t2)))
                && u2
                    .iter()
                    .all(|t2| u1.iter().any(|t1| annotations_equal(t1, t2)))
        }

        // Intersection types (order-independent)
        (TypeAnnotation::Intersection(i1), TypeAnnotation::Intersection(i2)) => {
            if i1.len() != i2.len() {
                return false;
            }
            i1.iter()
                .all(|t1| i2.iter().any(|t2| annotations_equal(t1, t2)))
                && i2
                    .iter()
                    .all(|t2| i1.iter().any(|t1| annotations_equal(t1, t2)))
        }

        // Generic types
        (
            TypeAnnotation::Generic { name: n1, args: a1 },
            TypeAnnotation::Generic { name: n2, args: a2 },
        ) => {
            n1 == n2
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(t1, t2)| annotations_equal(t1, t2))
        }

        // Void
        (TypeAnnotation::Void, TypeAnnotation::Void) => true,

        // Never
        (TypeAnnotation::Never, TypeAnnotation::Never) => true,

        // Null
        (TypeAnnotation::Null, TypeAnnotation::Null) => true,

        // Undefined
        (TypeAnnotation::Undefined, TypeAnnotation::Undefined) => true,

        // Different kinds are not equal
        _ => false,
    }
}

/// Check if two type constraints are equal
pub fn constraints_equal(a: &TypeConstraint, b: &TypeConstraint) -> bool {
    match (a, b) {
        (TypeConstraint::Numeric, TypeConstraint::Numeric) => true,
        (TypeConstraint::Comparable, TypeConstraint::Comparable) => true,
        (TypeConstraint::Iterable, TypeConstraint::Iterable) => true,
        (TypeConstraint::HasField(n1, t1), TypeConstraint::HasField(n2, t2)) => {
            n1 == n2 && types_equal(t1, t2)
        }
        (
            TypeConstraint::Callable {
                params: p1,
                returns: r1,
            },
            TypeConstraint::Callable {
                params: p2,
                returns: r2,
            },
        ) => {
            p1.len() == p2.len()
                && p1.iter().zip(p2.iter()).all(|(t1, t2)| types_equal(t1, t2))
                && types_equal(r1, r2)
        }
        (TypeConstraint::OneOf(o1), TypeConstraint::OneOf(o2)) => {
            o1.len() == o2.len() && o1.iter().zip(o2.iter()).all(|(t1, t2)| types_equal(t1, t2))
        }
        (TypeConstraint::Extends(e1), TypeConstraint::Extends(e2)) => types_equal(e1, e2),
        (
            TypeConstraint::HasMethod {
                method_name: n1,
                arg_types: a1,
                return_type: r1,
            },
            TypeConstraint::HasMethod {
                method_name: n2,
                arg_types: a2,
                return_type: r2,
            },
        ) => {
            n1 == n2
                && a1.len() == a2.len()
                && a1.iter().zip(a2.iter()).all(|(t1, t2)| types_equal(t1, t2))
                && types_equal(r1, r2)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::TypeVar;

    #[test]
    fn test_basic_type_equality() {
        let num1 = Type::Concrete(TypeAnnotation::Basic("number".to_string()));
        let num2 = Type::Concrete(TypeAnnotation::Basic("number".to_string()));
        let str1 = Type::Concrete(TypeAnnotation::Basic("string".to_string()));

        assert!(types_equal(&num1, &num2));
        assert!(!types_equal(&num1, &str1));
    }

    #[test]
    fn test_variable_equality() {
        let v1 = Type::Variable(TypeVar::new("T1".to_string()));
        let v2 = Type::Variable(TypeVar::new("T1".to_string()));
        let v3 = Type::Variable(TypeVar::new("T2".to_string()));

        assert!(types_equal(&v1, &v2));
        assert!(!types_equal(&v1, &v3));
    }

    #[test]
    fn test_generic_type_equality() {
        let opt1 = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
            ))),
            args: vec![Type::Concrete(TypeAnnotation::Basic("number".to_string()))],
        };
        let opt2 = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
            ))),
            args: vec![Type::Concrete(TypeAnnotation::Basic("number".to_string()))],
        };
        let opt3 = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
            ))),
            args: vec![Type::Concrete(TypeAnnotation::Basic("string".to_string()))],
        };

        assert!(types_equal(&opt1, &opt2));
        assert!(!types_equal(&opt1, &opt3));
    }

    #[test]
    fn test_union_annotation_equality_order_independent() {
        let union1 = TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("number".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]);
        let union2 = TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("string".to_string()),
            TypeAnnotation::Basic("number".to_string()),
        ]);

        assert!(annotations_equal(&union1, &union2));
    }

    #[test]
    fn test_function_type_equality() {
        let func1 = TypeAnnotation::Function {
            params: vec![shape_ast::ast::FunctionParam {
                name: Some("x".to_string()),
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
            }],
            returns: Box::new(TypeAnnotation::Basic("string".to_string())),
        };
        let func2 = TypeAnnotation::Function {
            params: vec![shape_ast::ast::FunctionParam {
                name: Some("x".to_string()),
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
            }],
            returns: Box::new(TypeAnnotation::Basic("string".to_string())),
        };

        assert!(annotations_equal(&func1, &func2));
    }
}
