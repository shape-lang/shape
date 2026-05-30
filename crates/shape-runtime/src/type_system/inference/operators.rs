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
    pub(crate) fn infer_literal(&mut self, lit: &Literal) -> TypeResult<Type> {
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
            Literal::Bool(_) => BuiltinTypes::boolean(),
            // `None` is polymorphic: Option<T> for fresh T.
            Literal::None => Self::wrap_in_option(self.fresh_type_var()),
            Literal::Unit => Type::Concrete(TypeAnnotation::Basic("()".to_string())),
            Literal::Timeframe(_) => Type::Concrete(TypeAnnotation::Basic("timeframe".to_string())),
        })
    }

    /// Check if a type is Option<T> and extract the inner type
    fn unwrap_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if args.len() == 1 => {
                if let Type::Concrete(ann) = base.as_ref() {
                    if ann.as_type_name_str() == Some("Option") {
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

    /// Is this type a bare null sentinel — the `None` literal or an explicit
    /// `null`/`Null` annotation?
    ///
    /// The `None` literal infers as `Option<var>` (an Option whose element is a
    /// still-unresolved type variable, see `infer_literal`). A concrete
    /// `Option<int>` is NOT a sentinel — its element type is known and equality
    /// against it should still type-check normally. Used by the `==`/`!=` arm to
    /// allow `None == x` (null-presence checks) without forcing same-type
    /// unification, while keeping `1 == "x"` a rejection.
    fn is_null_sentinel(ty: &Type) -> bool {
        match ty {
            // Explicit `null`/`None` annotation.
            Type::Concrete(TypeAnnotation::Null) => true,
            // `None` literal → `Option<var>`; only an unresolved element counts.
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
                    )
                    && matches!(args[0], Type::Variable(_)) =>
            {
                true
            }
            _ => false,
        }
    }

    /// Wrap a type in Option<T>
    fn wrap_in_option(ty: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
            ))),
            args: vec![ty],
        }
    }

    /// Check if a type is Result<T>/Option<T>/T? and extract the success type.
    fn unwrap_result_or_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(ann)
                    if ann.as_type_name_str().is_some_and(|n| n == "Result" || n == "Option") =>
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
    /// One operand still a TypeVar, the other concrete numeric → propagate the
    /// TypeVar as the result. This keeps the result type *linked* to the
    /// unresolved operand instead of eagerly collapsing to `number`. It is the
    /// load-bearing change for transitive inference: in `fn double(x){x*2}`
    /// the body's type becomes `x`'s own variable, so `double`'s return type
    /// is unified with its parameter. Once a callsite (possibly several calls
    /// deep) resolves the parameter, the return type resolves with it. The
    /// eager `number` collapse severed that link and made `double` infer as
    /// `fn(number)->number` even when every observed argument was an `int`.
    /// Both operands TypeVars → no information yet; default to `number`.
    fn numeric_result_type(left: &Type, right: &Type) -> Type {
        match (left, right) {
            // Same concrete numeric type → preserve it
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if l == r && BuiltinTypes::is_numeric_type_name(l) => left.clone(),
            // Mixed widths within the same script-level numeric family
            // (e.g. `i8 + i16`, `u8 + u32`) → preserve the script family type.
            // Both integer widths canonicalize to `int`; both float widths to
            // `number`. This keeps `i8 + i16` an `int` (not a `number`), so it
            // satisfies a `-> int` return. Cross-family mixes (int + float,
            // int + decimal) fall through to the widen-to-number arm below.
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if BuiltinTypes::is_numeric_type_name(l)
                && BuiltinTypes::is_numeric_type_name(r)
                && BuiltinTypes::canonical_script_alias(l).is_some()
                && BuiltinTypes::canonical_script_alias(l) == BuiltinTypes::canonical_script_alias(r) =>
            {
                let alias = BuiltinTypes::canonical_script_alias(l)
                    .expect("guarded by is_some() above");
                Type::Concrete(TypeAnnotation::Basic(alias.to_string()))
            }
            // Mixed concrete numeric across families → widen to number
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if BuiltinTypes::is_numeric_type_name(l) && BuiltinTypes::is_numeric_type_name(r) => {
                BuiltinTypes::number()
            }
            // One operand is an unresolved variable, the other a concrete
            // numeric type → propagate the variable so the result type stays
            // tied to the operand the call graph will eventually resolve.
            (Type::Variable(_), Type::Concrete(TypeAnnotation::Basic(r)))
                if BuiltinTypes::is_numeric_type_name(r) =>
            {
                left.clone()
            }
            (Type::Concrete(TypeAnnotation::Basic(l)), Type::Variable(_))
                if BuiltinTypes::is_numeric_type_name(l) =>
            {
                right.clone()
            }
            // Both operands unresolved (or non-basic) → default to number.
            _ => BuiltinTypes::number(),
        }
    }

    fn is_string_like(ty: &Type) -> bool {
        match ty {
            Type::Concrete(ann) if ann.as_type_name_str() == Some("string") => true,
            Type::Concrete(TypeAnnotation::Union(types)) => types.iter().any(|ann| {
                ann.as_type_name_str() == Some("string")
            }),
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("string")
                )
            }
            _ => false,
        }
    }

    fn is_vec_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => {
                inner.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Vec" => {
                args.first().is_some_and(|arg| {
                    arg.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Vec")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                )
            }
            _ => false,
        }
    }

    fn is_mat_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Mat" => {
                args.first().is_some_and(|arg| {
                    arg.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Mat")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                )
            }
            _ => false,
        }
    }

    fn mat_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Mat".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        })
    }

    fn vec_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Vec".into(),
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
        let left_bound = self.fresh_var();
        self.push_constraint_with_origin(
            effective_left.clone(),
            Type::Constrained {
                var: left_bound,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
            span,
        );
        let right_bound = self.fresh_var();
        self.push_constraint_with_origin(
            effective_right.clone(),
            Type::Constrained {
                var: right_bound,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
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
                // `null`/`None` is comparable to a value of any type — `None == 0`,
                // `x == None`, etc. are legitimate null-presence checks that must
                // not force same-type unification. When exactly one operand is a
                // bare null sentinel (the `None` literal infers as `Option<var>`,
                // or an explicit `Null` annotation), skip the `left ~ right`
                // constraint and yield bool directly.
                //
                // This is narrow on purpose: it does NOT relax equality between
                // two distinct non-null concrete types — `1 == "x"` still pushes
                // the same-type constraint and rejects.
                if Self::is_null_sentinel(left) != Self::is_null_sentinel(right) {
                    return Ok(BuiltinTypes::boolean());
                }
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
                let var = self.fresh_var();
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
                Ok(self.fresh_type_var())
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
                // Operator trait fallback (W1.6): if operand type implements
                // Not, return that type (unary `!` on user types is a UFCS
                // call to `Not::not(self) -> Self`). Sibling of the Neg
                // fallback below.
                if let Some(result_type) = self.check_operator_trait(&effective_operand, "Not") {
                    return if is_optional {
                        Ok(Self::wrap_in_option(result_type))
                    } else {
                        Ok(result_type)
                    };
                }
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
                // Unary analogue of `numeric_result_type` (the binary precision
                // fix). `-x` must preserve the operand's numeric type instead of
                // unconditionally widening to `number`:
                //   - concrete numeric (int/number/i8/decimal/…) → preserve it
                //     (so `-x` with x:int stays `int`, satisfying a `-> int`
                //     return and `int`/`number` separation);
                //   - unresolved Variable/Constrained → propagate the operand
                //     var (stays call-graph-linked, like the `var <op> concrete`
                //     arm of `numeric_result_type`) so a later callsite resolves
                //     the result with the parameter, instead of collapsing to
                //     `number` and severing the link;
                //   - non-numeric concrete (e.g. `-"s"`) → keep the `== number`
                //     constraint so it still rejects.
                let result = match &effective_operand {
                    Type::Concrete(TypeAnnotation::Basic(name))
                        if BuiltinTypes::is_numeric_type_name(name) =>
                    {
                        // Concrete numeric: validate via the Numeric trait bound
                        // (accepts int/number/i8/…, rejects non-numeric) without
                        // forcing `number`, then preserve the operand's type.
                        let bound = self.fresh_var();
                        self.constraints.push((
                            effective_operand.clone(),
                            Type::Constrained {
                                var: bound,
                                constraint: Box::new(TypeConstraint::ImplementsTrait {
                                    trait_name: "Numeric".to_string(),
                                }),
                            },
                        ));
                        effective_operand.clone()
                    }
                    Type::Variable(_) | Type::Constrained { .. } => {
                        // Unresolved: constrain to Numeric (keeps it linked to the
                        // call graph) and propagate the operand var as the result.
                        let bound = self.fresh_var();
                        self.constraints.push((
                            effective_operand.clone(),
                            Type::Constrained {
                                var: bound,
                                constraint: Box::new(TypeConstraint::ImplementsTrait {
                                    trait_name: "Numeric".to_string(),
                                }),
                            },
                        ));
                        effective_operand.clone()
                    }
                    // Non-numeric concrete (or other) → keep the strict `== number`
                    // constraint so genuinely-bad operands like `-"s"` reject.
                    _ => {
                        self.constraints
                            .push((effective_operand.clone(), BuiltinTypes::number()));
                        BuiltinTypes::number()
                    }
                };
                if is_optional {
                    Ok(Self::wrap_in_option(result))
                } else {
                    Ok(result)
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
            Type::Concrete(ann) => ann.as_type_name_str()?,
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

    fn basic(name: &str) -> Type {
        Type::Concrete(TypeAnnotation::Basic(name.to_string()))
    }

    #[test]
    fn test_mixed_width_int_arithmetic_stays_int() {
        // Strict-flip root R5: `i8 + i16` (mixed integer widths) must produce
        // `int`, not `number`, so it satisfies a `-> int` return. Previously
        // the mismatched-name path widened any mixed concrete pair to `number`.
        for (l, r) in [("i8", "i16"), ("u8", "u32"), ("i32", "i8"), ("i8", "int")] {
            let result = TypeInferenceEngine::numeric_result_type(&basic(l), &basic(r));
            assert_eq!(
                result,
                basic("int"),
                "{l} + {r} should stay `int`, got {result:?}"
            );
        }
    }

    #[test]
    fn test_int_plus_float_widens_to_number() {
        // Cross-family mixes still widen to `number` — int/number stay distinct.
        for (l, r) in [("i8", "f64"), ("int", "number"), ("i16", "f32")] {
            let result = TypeInferenceEngine::numeric_result_type(&basic(l), &basic(r));
            assert_eq!(
                result,
                BuiltinTypes::number(),
                "{l} + {r} should widen to `number`, got {result:?}"
            );
        }
    }

    #[test]
    fn test_unwrap_option_generic() {
        let option_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".into(),
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
            name: "Option".into(),
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
    fn test_null_sentinel_eq_skips_same_type_constraint() {
        // `None == 0` / `0 == None`: the `None` literal infers as `Option<var>`,
        // a null sentinel. Equality against it must NOT push a `left ~ right`
        // same-type constraint (null is comparable to any value), and yields bool.
        for left_is_none in [true, false] {
            let mut engine = TypeInferenceEngine::new();
            let none = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let (l, r) = if left_is_none {
                (none, basic("int"))
            } else {
                (basic("int"), none)
            };
            let before = engine.constraints.len();
            let inferred = engine
                .infer_binary_op(&l, &BinaryOp::Equal, &r, Span::DUMMY)
                .expect("null-sentinel equality should infer");
            assert_eq!(inferred, BuiltinTypes::boolean());
            assert_eq!(
                engine.constraints.len(),
                before,
                "null-sentinel `==` must not push a same-type unification constraint \
                 ({l:?} == {r:?})"
            );
        }
        // Explicit `Null` annotation is also a sentinel.
        let mut engine = TypeInferenceEngine::new();
        let before = engine.constraints.len();
        engine
            .infer_binary_op(
                &Type::Concrete(TypeAnnotation::Null),
                &BinaryOp::NotEqual,
                &basic("string"),
                Span::DUMMY,
            )
            .expect("explicit-Null inequality should infer");
        assert_eq!(engine.constraints.len(), before);
    }

    #[test]
    fn test_distinct_concrete_eq_still_constrains_and_rejects() {
        // NOT broad suppression: two distinct non-null concrete types (`1 == "x"`)
        // must STILL push the same-type unification constraint, which the solver
        // then rejects. `None == None` also keeps constraining (both sentinels →
        // condition is false), so `Option<var> ~ Option<var>` is still pushed.
        {
            let mut engine = TypeInferenceEngine::new();
            let before = engine.constraints.len();
            engine
                .infer_binary_op(&basic("int"), &BinaryOp::Equal, &basic("string"), Span::DUMMY)
                .expect("equality should infer");
            assert_eq!(
                engine.constraints.len(),
                before + 1,
                "`int == string` must still push the same-type constraint"
            );
        }
        {
            let mut engine = TypeInferenceEngine::new();
            let none_l = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let none_r = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let before = engine.constraints.len();
            engine
                .infer_binary_op(&none_l, &BinaryOp::Equal, &none_r, Span::DUMMY)
                .expect("None == None should infer");
            assert_eq!(
                engine.constraints.len(),
                before + 1,
                "two-sentinel `None == None` must still push the same-type constraint"
            );
        }

        // And the genuine mismatch `int ~ string` is actually rejected by the solver.
        let mut engine = TypeInferenceEngine::new();
        engine
            .infer_binary_op(&basic("int"), &BinaryOp::Equal, &basic("string"), Span::DUMMY)
            .expect("inference itself succeeds; the constraint is what fails");
        assert!(
            engine.solver.solve(&mut engine.constraints).is_err(),
            "`int == string` must still be a type error after the null-eq fix"
        );
    }

    #[test]
    fn test_error_context_promotes_option_to_result() {
        let mut engine = TypeInferenceEngine::new();
        let option_num = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".into(),
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
                "Result".into(),
            ))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".into())),
            ],
        };
        assert_eq!(inferred, expected);
    }

    #[test]
    fn test_error_context_keeps_result_inner_type() {
        let mut engine = TypeInferenceEngine::new();
        let result_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".into(),
            ))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".into())),
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
        let mut engine = TypeInferenceEngine::new();
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
        let mut engine = TypeInferenceEngine::new();
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
