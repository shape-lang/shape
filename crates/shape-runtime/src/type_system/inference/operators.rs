//! Operator type inference
//!
//! Handles type inference for literals, binary operators, and unary operators.
//!
//! ## Option Propagation
//!
//! Arithmetic operators support automatic Option propagation:
//! - `Option<Number> + Number` -> `Option<Number>`
//! - `Number + Option<Number>` -> `Option<Number>`
//! - `Option<Number> + Option<Number>` -> `Option<Number>`
//!
//! This enables ergonomic handling of nullable values without explicit unwrapping.
//! At runtime, NaN sentinel is used for Option<f64>, so propagation is zero-cost.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{BinaryOp, Literal, Span, TypeAnnotation, UnaryOp};

impl TypeInferenceEngine {
    /// Infer type of a literal
    pub(crate) fn infer_literal(&self, lit: &Literal) -> TypeResult<Type> {
        Ok(match lit {
            Literal::Int(_) => Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Literal::UInt(_) => Type::Concrete(TypeAnnotation::Basic("u64".to_string())),
            Literal::TypedInt(_, w) => {
                Type::Concrete(TypeAnnotation::Basic(w.type_name().to_string()))
            }
            Literal::Number(_) => BuiltinTypes::number(),
            Literal::Decimal(_) => Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
            Literal::String(_) => BuiltinTypes::string(),
            Literal::Char(_) => Type::Concrete(TypeAnnotation::Basic("char".to_string())),
            Literal::FormattedString { .. } => BuiltinTypes::string(),
            Literal::ContentString { .. } => BuiltinTypes::string(),
            Literal::Bool(_) => BuiltinTypes::boolean(),
            // `None` is polymorphic: Option<T> for fresh T.
            Literal::None => Self::wrap_in_option(Type::fresh_var()),
            Literal::Unit => Type::Concrete(TypeAnnotation::Basic("()".to_string())),
            Literal::Timeframe(_) => Type::Concrete(TypeAnnotation::Basic("timeframe".to_string())),
        })
    }

    /// Check if a type is Option<T> and extract the inner type
    fn unwrap_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if args.len() == 1 => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    if name == "Option" {
                        return Some(args[0].clone());
                    }
                }
                None
            }
            // Handle T? desugared to TypeAnnotation::Generic { name: "Option", args }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Option" && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    /// Wrap a type in Option<T>
    fn wrap_in_option(ty: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".to_string(),
            ))),
            args: vec![ty],
        }
    }

    /// Check if a type is Result<T>/Option<T>/T? and extract the success type.
    fn unwrap_result_or_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Reference(name))
                | Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "Result" || name == "Option" =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Result" || name == "Option") && !args.is_empty() =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    /// Wrap a type in Result<T, AnyError>.
    fn wrap_in_result(&self, ty: Type) -> Type {
        self.wrap_result_type(ty)
    }

    /// Compute the result type for numeric arithmetic based on operand types.
    ///
    /// Same concrete numeric type → preserve it (int*int→int, number*number→number).
    /// Mixed concrete numeric → widen to number (int*float→number).
    /// Unknown operand type (TypeVar) → default to number.
    fn numeric_result_type(left: &Type, right: &Type) -> Type {
        match (left, right) {
            // Same concrete numeric type → preserve it
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if l == r && BuiltinTypes::is_numeric_type_name(l) => left.clone(),
            // Mixed concrete numeric → widen to number
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if BuiltinTypes::is_numeric_type_name(l) && BuiltinTypes::is_numeric_type_name(r) => {
                BuiltinTypes::number()
            }
            // Unknown operand type (TypeVar) → default to number
            _ => BuiltinTypes::number(),
        }
    }

    fn is_string_like(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Basic(name))
            | Type::Concrete(TypeAnnotation::Reference(name)) => name == "string",
            Type::Concrete(TypeAnnotation::Union(types)) => types.iter().any(|ann| {
                matches!(ann, TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) if name == "string")
            }),
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name))
                        | Type::Concrete(TypeAnnotation::Basic(name))
                        if name == "Option"
                ) && matches!(
                    &args[0],
                    Type::Concrete(TypeAnnotation::Basic(name))
                        | Type::Concrete(TypeAnnotation::Reference(name))
                        if name == "string"
                )
            }
            _ => false,
        }
    }

    fn is_vec_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => matches!(
                inner.as_ref(),
                TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name)
                    if BuiltinTypes::is_numeric_type_name(name)
            ),
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Vec" => {
                args.first().is_some_and(|arg| {
                    matches!(arg, TypeAnnotation::Basic(n) | TypeAnnotation::Reference(n)
                        if BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name))
                        | Type::Concrete(TypeAnnotation::Basic(name))
                        if name == "Vec"
                ) && matches!(
                    &args[0],
                    Type::Concrete(TypeAnnotation::Basic(name))
                        | Type::Concrete(TypeAnnotation::Reference(name))
                        if BuiltinTypes::is_numeric_type_name(name)
                )
            }
            _ => false,
        }
    }

    fn is_mat_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Mat" => {
                args.first().is_some_and(|arg| {
                    matches!(arg, TypeAnnotation::Basic(n) | TypeAnnotation::Reference(n)
                        if BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name))
                        | Type::Concrete(TypeAnnotation::Basic(name))
                        if name == "Mat"
                ) && matches!(
                    &args[0],
                    Type::Concrete(TypeAnnotation::Basic(name))
                        | Type::Concrete(TypeAnnotation::Reference(name))
                        if BuiltinTypes::is_numeric_type_name(name)
                )
            }
            _ => false,
        }
    }

    fn mat_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Mat".to_string(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        })
    }

    fn vec_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Vec".to_string(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        })
    }

    /// Build intersection type for object-like `+` (structural merge).
    fn infer_object_add_type(left: &Type, right: &Type) -> Option<Type> {
        fn push_members(ty: &Type, out: &mut Vec<TypeAnnotation>) -> bool {
            match ty {
                Type::Concrete(TypeAnnotation::Object(fields)) => {
                    out.push(TypeAnnotation::Object(fields.clone()));
                    true
                }
                Type::Concrete(TypeAnnotation::Reference(name)) => {
                    out.push(TypeAnnotation::Reference(name.clone()));
                    true
                }
                Type::Concrete(TypeAnnotation::Intersection(types)) => {
                    out.extend(types.clone());
                    true
                }
                _ => false,
            }
        }

        let mut members = Vec::new();
        if !push_members(left, &mut members) || !push_members(right, &mut members) {
            return None;
        }

        Some(Type::Concrete(TypeAnnotation::Intersection(members)))
    }

    /// Shared numeric arithmetic inference for `+`, `-`, `*`, `/`, `%`.
    fn infer_numeric_arithmetic_op(
        &mut self,
        left: &Type,
        right: &Type,
        span: Span,
    ) -> TypeResult<Type> {
        // Check for Option propagation
        let left_inner = Self::unwrap_option_type(left);
        let right_inner = Self::unwrap_option_type(right);

        let (effective_left, effective_right, is_optional) = match (&left_inner, &right_inner) {
            (Some(l), Some(r)) => (l.clone(), r.clone(), true),
            (Some(l), None) => (l.clone(), right.clone(), true),
            (None, Some(r)) => (left.clone(), r.clone(), true),
            (None, None) => (left.clone(), right.clone(), false),
        };

        // Constrain operands to be numeric (int, float, number, decimal)
        // without forcing to `number` — preserves type specificity
        let left_bound = TypeVar::fresh();
        self.push_constraint_with_origin(
            effective_left.clone(),
            Type::Constrained {
                var: left_bound,
                constraint: Box::new(TypeConstraint::Numeric),
            },
            span,
        );
        let right_bound = TypeVar::fresh();
        self.push_constraint_with_origin(
            effective_right.clone(),
            Type::Constrained {
                var: right_bound,
                constraint: Box::new(TypeConstraint::Numeric),
            },
            span,
        );

        // Compute result type based on operand types
        let result = Self::numeric_result_type(&effective_left, &effective_right);

        if is_optional {
            Ok(Self::wrap_in_option(result))
        } else {
            Ok(result)
        }
    }

    /// Infer type of binary operation
    ///
    /// Supports Option propagation: if either operand is Option<T>, the result is Option<T>.
    pub(crate) fn infer_binary_op(
        &mut self,
        left: &Type,
        op: &BinaryOp,
        right: &Type,
        span: Span,
    ) -> TypeResult<Type> {
        match op {
            BinaryOp::Add => {
                if let Some(merged) = Self::infer_object_add_type(left, right) {
                    return Ok(merged);
                }
                // String concatenation is allowed in Shape and should not force
                // numeric constraints on the opposite operand.
                if Self::is_string_like(left) || Self::is_string_like(right) {
                    return Ok(BuiltinTypes::string());
                }
                // Operator trait fallback: if left type implements Add, return left type
                if let Some(result_type) = self.check_operator_trait(left, "Add") {
                    return Ok(result_type);
                }
                self.infer_numeric_arithmetic_op(left, right, span)
            }
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                if matches!(op, BinaryOp::Mul) {
                    if Self::is_mat_number(left) && Self::is_vec_number(right) {
                        return Ok(Self::vec_number_type());
                    }
                    if Self::is_mat_number(left) && Self::is_mat_number(right) {
                        return Ok(Self::mat_number_type());
                    }
                }
                // Operator trait fallback
                let trait_name = match op {
                    BinaryOp::Sub => "Sub",
                    BinaryOp::Mul => "Mul",
                    BinaryOp::Div => "Div",
                    _ => "", // Mod has no operator trait
                };
                if !trait_name.is_empty() {
                    if let Some(result_type) = self.check_operator_trait(left, trait_name) {
                        return Ok(result_type);
                    }
                }
                self.infer_numeric_arithmetic_op(left, right, span)
            }

            BinaryOp::Equal | BinaryOp::NotEqual => {
                // Equality can work on any types, but they should be the same
                self.push_constraint_with_origin(left.clone(), right.clone(), span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::Less | BinaryOp::Greater | BinaryOp::LessEq | BinaryOp::GreaterEq => {
                // Comparison operations with Option propagation
                let left_inner = Self::unwrap_option_type(left);
                let right_inner = Self::unwrap_option_type(right);

                let (effective_left, effective_right, is_optional) =
                    match (&left_inner, &right_inner) {
                        (Some(l), Some(r)) => (l.clone(), r.clone(), true),
                        (Some(l), None) => (l.clone(), right.clone(), true),
                        (None, Some(r)) => (left.clone(), r.clone(), true),
                        (None, None) => (left.clone(), right.clone(), false),
                    };

                self.push_constraint_with_origin(effective_left.clone(), effective_right, span);
                // Add constraint that types must be comparable
                let var = TypeVar::fresh();
                self.push_constraint_with_origin(
                    effective_left,
                    Type::Constrained {
                        var,
                        constraint: Box::new(TypeConstraint::Comparable),
                    },
                    span,
                );

                if is_optional {
                    // Comparison with Option returns Option<Bool>
                    Ok(Self::wrap_in_option(BuiltinTypes::boolean()))
                } else {
                    Ok(BuiltinTypes::boolean())
                }
            }

            BinaryOp::And | BinaryOp::Or => {
                // Logical operations
                self.push_constraint_with_origin(left.clone(), BuiltinTypes::boolean(), span);
                self.push_constraint_with_origin(right.clone(), BuiltinTypes::boolean(), span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::FuzzyEqual | BinaryOp::FuzzyLess | BinaryOp::FuzzyGreater => {
                // Fuzzy comparison for numbers
                self.push_constraint_with_origin(left.clone(), BuiltinTypes::number(), span);
                self.push_constraint_with_origin(right.clone(), BuiltinTypes::number(), span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => {
                // Bitwise operations require integer operands
                self.push_constraint_with_origin(left.clone(), BuiltinTypes::integer(), span);
                self.push_constraint_with_origin(right.clone(), BuiltinTypes::integer(), span);
                Ok(BuiltinTypes::integer())
            }

            BinaryOp::Pow => {
                // Exponentiation with Option propagation
                let left_inner = Self::unwrap_option_type(left);
                let right_inner = Self::unwrap_option_type(right);

                let (effective_left, effective_right, is_optional) =
                    match (&left_inner, &right_inner) {
                        (Some(l), Some(r)) => (l.clone(), r.clone(), true),
                        (Some(l), None) => (l.clone(), right.clone(), true),
                        (None, Some(r)) => (left.clone(), r.clone(), true),
                        (None, None) => (left.clone(), right.clone(), false),
                    };

                self.push_constraint_with_origin(effective_left, BuiltinTypes::number(), span);
                self.push_constraint_with_origin(effective_right, BuiltinTypes::number(), span);

                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::number()))
                } else {
                    Ok(BuiltinTypes::number())
                }
            }

            BinaryOp::NullCoalesce => {
                // Null coalescing operator - result type is union of left (non-null) and right
                // For now, return the right type as a simple approximation
                Ok(right.clone())
            }

            BinaryOp::ErrorContext => {
                // Context wrapping always returns Result<SuccessType>.
                // - Result<T> !! ctx -> Result<T>
                // - Option<T>/T? !! ctx -> Result<T>
                // - T !! ctx -> Result<T>
                let success =
                    Self::unwrap_result_or_option_type(left).unwrap_or_else(|| left.clone());
                Ok(self.wrap_in_result(success))
            }

            BinaryOp::Pipe => {
                // Pipe operator - left is piped into right (which should be a function)
                // Result type is determined by the right side's return type
                // For now, return a new type variable that will be resolved later
                Ok(Type::fresh_var())
            }
        }
    }

    /// Infer type of unary operation
    ///
    /// Supports Option propagation: if operand is Option<T>, result is Option<ResultType>.
    pub(crate) fn infer_unary_op(&mut self, op: &UnaryOp, operand: &Type) -> TypeResult<Type> {
        let inner = Self::unwrap_option_type(operand);
        let (effective_operand, is_optional) = match &inner {
            Some(t) => (t.clone(), true),
            None => (operand.clone(), false),
        };

        match op {
            UnaryOp::Not => {
                self.constraints
                    .push((effective_operand, BuiltinTypes::boolean()));
                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::boolean()))
                } else {
                    Ok(BuiltinTypes::boolean())
                }
            }
            UnaryOp::Neg => {
                // Operator trait fallback: if operand type implements Neg, return that type
                if let Some(result_type) = self.check_operator_trait(&effective_operand, "Neg") {
                    return if is_optional {
                        Ok(Self::wrap_in_option(result_type))
                    } else {
                        Ok(result_type)
                    };
                }
                self.constraints
                    .push((effective_operand, BuiltinTypes::number()));
                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::number()))
                } else {
                    Ok(BuiltinTypes::number())
                }
            }
            UnaryOp::BitNot => {
                self.constraints
                    .push((effective_operand, BuiltinTypes::integer()));
                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::integer()))
                } else {
                    Ok(BuiltinTypes::integer())
                }
            }
        }
    }

    /// Check if a type implements an operator trait (Add, Sub, Mul, Div, Neg, Eq, Ord).
    /// If so, returns the result type (the operand type itself for Self-returning traits).
    fn check_operator_trait(&self, operand_type: &Type, trait_name: &str) -> Option<Type> {
        let type_name = match operand_type {
            Type::Concrete(TypeAnnotation::Basic(name))
            | Type::Concrete(TypeAnnotation::Reference(name)) => name.as_str(),
            _ => return None,
        };
        // Skip primitive/numeric types — they use the built-in arithmetic path
        if BuiltinTypes::is_numeric_type_name(type_name)
            || type_name == "string"
            || type_name == "bool"
        {
            return None;
        }
        if self.env.type_implements_trait(type_name, trait_name) {
            Some(operand_type.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unwrap_option_generic() {
        let option_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".to_string(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        let inner = TypeInferenceEngine::unwrap_option_type(&option_num);
        assert!(inner.is_some());
        assert_eq!(inner.unwrap(), BuiltinTypes::number());
    }

    #[test]
    fn test_unwrap_option_annotation() {
        let option_num = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".to_string(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        });
        let inner = TypeInferenceEngine::unwrap_option_type(&option_num);
        assert!(inner.is_some());
    }

    #[test]
    fn test_unwrap_non_option() {
        let num = BuiltinTypes::number();
        let inner = TypeInferenceEngine::unwrap_option_type(&num);
        assert!(inner.is_none());
    }

    #[test]
    fn test_wrap_in_option() {
        let num = BuiltinTypes::number();
        let wrapped = TypeInferenceEngine::wrap_in_option(num);
        assert!(matches!(wrapped, Type::Generic { .. }));

        // Verify it's Option<number>
        let unwrapped = TypeInferenceEngine::unwrap_option_type(&wrapped);
        assert!(unwrapped.is_some());
    }

    #[test]
    fn test_error_context_promotes_option_to_result() {
        let mut engine = TypeInferenceEngine::new();
        let option_num = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".to_string(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        });
        let inferred = engine
            .infer_binary_op(
                &option_num,
                &BinaryOp::ErrorContext,
                &BuiltinTypes::string(),
                Span::DUMMY,
            )
            .expect("option !! context should infer");

        let expected = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".to_string())),
            ],
        };
        assert_eq!(inferred, expected);
    }

    #[test]
    fn test_error_context_keeps_result_inner_type() {
        let mut engine = TypeInferenceEngine::new();
        let result_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".to_string())),
            ],
        };
        let inferred = engine
            .infer_binary_op(
                &result_num,
                &BinaryOp::ErrorContext,
                &BuiltinTypes::string(),
                Span::DUMMY,
            )
            .expect("result !! context should infer");
        assert_eq!(inferred, result_num);
    }

    #[test]
    fn test_infer_literal_formatted_string_is_string() {
        let engine = TypeInferenceEngine::new();
        let inferred = engine
            .infer_literal(&Literal::FormattedString {
                value: "x={x}".to_string(),
                mode: shape_ast::ast::InterpolationMode::Braces,
            })
            .expect("formatted string literal should infer");
        assert_eq!(inferred, BuiltinTypes::string());
    }

    #[test]
    fn test_infer_literal_none_is_option_not_null() {
        let engine = TypeInferenceEngine::new();
        let inferred = engine
            .infer_literal(&Literal::None)
            .expect("None literal should infer");

        match inferred {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Option"
                    ),
                    "None must infer as Option<T>, got {:?}",
                    base
                );
                assert_eq!(args.len(), 1, "Option must have exactly one type argument");
                assert!(
                    !matches!(&args[0], Type::Concrete(TypeAnnotation::Null)),
                    "None must not infer as null"
                );
            }
            other => panic!("expected Option<T> for None, got {:?}", other),
        }
    }

    #[test]
    fn test_add_object_types_produces_intersection() {
        let mut engine = TypeInferenceEngine::new();
        let left = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]));
        let right = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "z".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]));

        let inferred = engine
            .infer_binary_op(&left, &BinaryOp::Add, &right, Span::DUMMY)
            .expect("object + object should infer");

        assert!(
            matches!(inferred, Type::Concrete(TypeAnnotation::Intersection(_))),
            "expected intersection type, got {:?}",
            inferred
        );
    }
}
