//! Structural Type Annotation Equality
//!
//! `annotations_equal` is the structural comparison for the **annotation**
//! layer (`TypeAnnotation`). It is NOT a type-equivalence relation.
//!
//! U1 (canonical-Type unification): the standalone `types_equal(&Type, &Type)`
//! and its `constraints_equal` helper were DELETED. They were the third,
//! arm-incomplete equality procedure (STRUCTURAL-AUDIT SB-3) that could not see
//! through the multiple `Array<T>` encodings (SB-4). The single
//! type-equivalence relation now lives in `ConstraintSolver::probe_equal`
//! (`solve_constraint` run in non-committing probe mode), reached via
//! `TypeInferenceEngine::types_equal`. Do not reintroduce a parallel
//! `Type`-level structural-equality fn here.

use shape_ast::ast::TypeAnnotation;

/// Check if two type annotations are structurally equal
pub fn annotations_equal(a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
    match (a, b) {
        // Basic types
        (TypeAnnotation::Basic(n1), TypeAnnotation::Basic(n2)) => n1 == n2,

        // Reference types
        (TypeAnnotation::Reference(n1), TypeAnnotation::Reference(n2)) => n1 == n2,

        // Borrow types (R1/GAP-2): `&T` / `&mut T`. A distinct constructor —
        // never unwrapped on one side. Match the mutability flag and recurse on
        // the inner annotation so `&int` matches `&int` but NOT `&number` /
        // bare `int`. Value-level int != number etc. stays intact because the
        // recursion bottoms out in the existing Basic arm.
        (
            TypeAnnotation::Borrow {
                mutable: m1,
                inner: i1,
            },
            TypeAnnotation::Borrow {
                mutable: m2,
                inner: i2,
            },
        ) => m1 == m2 && annotations_equal(i1, i2),

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

#[cfg(test)]
mod tests {
    use super::*;

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
