//! Bidirectional Type Checking
//!
//! Implements bidirectional type checking for improved type inference,
//! especially for closure expressions passed to higher-order functions
//! where the expected parameter types can be propagated inward.
//!
//! ## Check Modes
//!
//! - **`Infer`** -- No expected type; purely synthesise from the expression.
//! - **`Check(Type)`** -- Hard constraint: the expression *must* have this
//!   type. Emitted for explicitly annotated bindings and return positions.
//!   A mismatch is a type error.
//! - **`Synth(Type)`** -- Soft hint: the expression is *expected* to have
//!   this type but may refine it. Used when propagating closure parameter
//!   types inferred from generic method signatures (e.g. the element type
//!   `T` from `Vec<T>.map(fn(T) -> U) -> Vec<U>`).
//!
//! ## Flow
//!
//! `check_expr` dispatches on the mode:
//! - `Infer` falls through to `infer_expr` (pure synthesis).
//! - `Check` calls `check_against`, which infers the expression and then
//!   emits an equality constraint between inferred and expected types.
//! - `Synth` calls `synthesize_with_hint`, which infers the expression,
//!   emits the constraint, and returns the inferred type (not the hint)
//!   so downstream inference stays precise.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Expr, FunctionParameter, ObjectEntry, TypeAnnotation};

/// Mode for bidirectional type checking
#[derive(Debug, Clone)]
pub enum CheckMode {
    /// Infer the type without any expectation
    Infer,
    /// Check the expression against an expected type (hard constraint)
    Check(Type),
    /// Synthesize with a hint type (soft constraint)
    Synth(Type),
}

impl CheckMode {
    /// Get the expected type if in Check or Synth mode
    pub fn expected(&self) -> Option<&Type> {
        match self {
            CheckMode::Infer => None,
            CheckMode::Check(ty) | CheckMode::Synth(ty) => Some(ty),
        }
    }

    /// Check if this is a hard constraint
    pub fn is_hard_constraint(&self) -> bool {
        matches!(self, CheckMode::Check(_))
    }
}

impl TypeInferenceEngine {
    /// Check an expression with a given mode
    ///
    /// This is the main entry point for bidirectional type checking.
    pub fn check_expr(&mut self, expr: &Expr, mode: CheckMode) -> TypeResult<Type> {
        match mode {
            CheckMode::Infer => self.infer_expr(expr),
            CheckMode::Check(expected) => self.check_against(expr, &expected),
            CheckMode::Synth(hint) => self.synth_with_hint(expr, &hint),
        }
    }

    /// Check an expression against an expected type
    ///
    /// The expected type guides inference and provides better error messages.
    pub fn check_against(&mut self, expr: &Expr, expected: &Type) -> TypeResult<Type> {
        match expr {
            // Function expression: use expected function type for parameter inference
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                span: _span,
            } => self.check_function_expr_against(params, return_type.as_ref(), body, expected),

            // Array: propagate element type to elements
            Expr::Array(elements, _) => {
                if let Type::Concrete(TypeAnnotation::Array(elem_ty)) = expected {
                    self.check_array_against(elements, &Type::Concrete(*elem_ty.clone()))
                } else {
                    // Expected isn't an array type, infer and unify
                    let inferred = self.infer_expr(expr)?;
                    self.constraints.push((inferred.clone(), expected.clone()));
                    Ok(inferred)
                }
            }

            // Object: propagate field types
            Expr::Object(entries, _) => {
                if let Type::Concrete(TypeAnnotation::Object(expected_fields)) = expected {
                    self.check_object_against(entries, expected_fields)
                } else {
                    let inferred = self.infer_expr(expr)?;
                    self.constraints.push((inferred.clone(), expected.clone()));
                    Ok(inferred)
                }
            }

            // Conditional: propagate expected to both branches
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                let cond_type = self.infer_expr(condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                let then_type = self.check_against(then_expr, expected)?;

                if let Some(else_e) = else_expr {
                    let else_type = self.check_against(else_e, expected)?;
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }

            // Match: propagate expected to arms
            Expr::Match(match_expr, _) => {
                let _scrutinee_type = self.infer_expr(&match_expr.scrutinee)?;

                let mut arm_types = Vec::new();

                for arm in &match_expr.arms {
                    self.env.push_scope();
                    self.bind_pattern_vars(&arm.pattern)?;

                    let arm_type = self.check_against(&arm.body, expected)?;
                    arm_types.push(arm_type);

                    self.env.pop_scope();
                }

                // All arms should have the same type (the expected type)
                if !arm_types.is_empty() {
                    let first = arm_types[0].clone();
                    for ty in &arm_types[1..] {
                        self.constraints.push((first.clone(), ty.clone()));
                    }
                    Ok(first)
                } else {
                    Ok(expected.clone())
                }
            }

            // Numeric-conversion LITERAL ADOPTION through an ENUM-CONSTRUCTOR
            // payload (spec §4, constructor-payload-vs-expected path). When an
            // `Ok`/`Err`/`Some` constructor (parsed as a `FunctionCall`) whose
            // argument is a bare numeric LITERAL is checked against an EXPECTED
            // `Result<T,E>` / `Option<T>` carrier, propagate the expected
            // variant-payload type to the constructor's argument so the literal
            // adopts the expected numeric type — exactly the adoption the direct
            // contexts (let-annotation, comparison, struct-field, match-arm)
            // already get. `fn f() -> Result<number> { Ok(42) }` then accepts
            // (42 adopts `number`).
            //
            // GATED on the argument being a literal that ACTUALLY adopts
            // (`constructor_arg_adopts_literal`). A non-literal argument
            // (`Ok(x)` / `Ok(x * 2)`) is LEFT to the default `infer_function_call`
            // path: it carries the same `arg_type ~ payload` constraint via the
            // builtin `Ok`/`Err`/`Some` signature + `push_return_constraint`, and
            // intercepting it here would bypass the unannotated-param callsite
            // refinement that `infer_function_call` feeds (which `Ok(x)` with an
            // unannotated `x` relies on). An out-of-range literal also does not
            // adopt, so it too falls through and correctly rejects. This keeps
            // the intercept to exactly the bare-literal FP-regression class and
            // never introduces loose numeric widening of a VALUE.
            Expr::FunctionCall { name, args, .. }
                if matches!(name.as_str(), "Ok" | "Err" | "Some")
                    && self.constructor_arg_adopts_literal(name, args, expected) =>
            {
                let payloads = self
                    .constructor_payload_types_from_expected(name, args.len(), expected)
                    .expect("constructor_arg_adopts_literal proved payloads exist");
                for (arg, payload) in args.iter().zip(payloads.iter()) {
                    self.check_against(arg, payload)?;
                }
                Ok(expected.clone())
            }

            // Numeric-conversion LITERAL ADOPTION (spec §4): a bare integer
            // literal in a concrete numeric context adopts that context type
            // when its value is losslessly representable in it. `let n: number =
            // 5` types `5` as the number literal `5.0`; `let x: u8 = 200` types
            // `200` as a u8 literal. NO rejecting constraint is pushed (the
            // literal IS the expected type). An OUT-OF-RANGE literal does NOT
            // adopt — it falls through to the default arm, where `(int, u8)`
            // fails the tightened §2 lattice and `let x: u8 = 300` correctly
            // compile-rejects (never a silent wrap).
            Expr::Literal(..)
                if Self::adopt_int_literal_in_context(expr, expected).is_some() =>
            {
                Ok(expected.clone())
            }

            // Default: infer and constrain to expected
            _ => {
                let inferred = self.infer_expr(expr)?;
                self.constraints.push((inferred.clone(), expected.clone()));
                Ok(inferred)
            }
        }
    }

    /// Expected payload type(s) for an `Ok`/`Err`/`Some` constructor checked
    /// against an EXPECTED `Result<T,E>` / `Option<T>` carrier.
    ///
    /// - `Some` against `Option<T>` → `[T]`
    /// - `Ok` against `Result<T,E>` → `[T]`
    /// - `Err` against `Result<T,E>` → `[E]`
    ///
    /// Returns `None` (so the bidirectional arm falls through to plain
    /// inference) when the constructor doesn't match the expected carrier, the
    /// arity isn't 1, or the payload type can't be extracted. The returned
    /// payload type may be a bare type VARIABLE (an unresolved `Result<T,E>`
    /// from an annotation still being solved); `check_against` against a
    /// variable simply pushes the normal equality constraint, so literal
    /// adoption only fires when the expected payload is concrete-numeric.
    pub(crate) fn constructor_payload_types_from_expected(
        &self,
        name: &str,
        arity: usize,
        expected: &Type,
    ) -> Option<Vec<Type>> {
        if arity != 1 {
            return None;
        }
        match name {
            "Some" if self.is_option_type(expected) => {
                self.result_or_option_success_type(expected).map(|t| vec![t])
            }
            "Ok" if self.is_result_type(expected) => {
                self.result_or_option_success_type(expected).map(|t| vec![t])
            }
            "Err" if self.is_result_type(expected) => {
                self.result_error_type(expected).map(|e| vec![e])
            }
            _ => None,
        }
    }

    /// Whether the `Ok`/`Err`/`Some` constructor `name(args)` has an argument
    /// that is a bare numeric LITERAL which ADOPTS the expected variant-payload
    /// type extracted from `expected`. This is the precise gate for the
    /// constructor-payload-vs-expected intercept: it fires only when re-routing
    /// the argument through `check_against` would change the outcome (a literal
    /// adopting a numeric payload), so non-literal arguments (`Ok(x)`,
    /// `Ok(x * 2)`) and out-of-range / non-numeric literals fall through to the
    /// default `infer_function_call` path unchanged.
    pub(crate) fn constructor_arg_adopts_literal(
        &self,
        name: &str,
        args: &[Expr],
        expected: &Type,
    ) -> bool {
        let Some(payloads) = self.constructor_payload_types_from_expected(name, args.len(), expected)
        else {
            return false;
        };
        args.iter().zip(payloads.iter()).any(|(arg, payload)| {
            Self::adopt_int_literal_in_context(arg, payload).is_some()
        })
    }

    /// Synthesize type with a hint (soft constraint)
    ///
    /// The hint guides inference but doesn't force the type.
    fn synth_with_hint(&mut self, expr: &Expr, hint: &Type) -> TypeResult<Type> {
        let inferred = self.infer_expr(expr)?;

        // Try to unify with hint - if it fails, just return inferred
        // This is a "soft" constraint that helps but doesn't force
        if self.unifier.try_unify(&inferred, hint).is_ok() {
            Ok(hint.clone())
        } else {
            Ok(inferred)
        }
    }

    /// Check a function expression against an expected function type
    fn check_function_expr_against(
        &mut self,
        params: &[FunctionParameter],
        return_type_ann: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
        expected: &Type,
    ) -> TypeResult<Type> {
        // Extract expected param types and return type from expected function type
        let (expected_params, expected_return) = match expected {
            Type::Concrete(TypeAnnotation::Function {
                params: expected_param_anns,
                returns,
            }) => {
                let param_types: Vec<Type> = expected_param_anns
                    .iter()
                    .map(|p| Type::Concrete(p.type_annotation.clone()))
                    .collect();
                let return_type = Type::Concrete(*returns.clone());
                (param_types, return_type)
            }
            Type::Function {
                params: fp,
                returns: fr,
            } => (fp.clone(), *fr.clone()),
            _ => {
                // Expected isn't a function type, fall back to regular inference
                return self.infer_function_expr(params, return_type_ann, body);
            }
        };

        // Enter a new scope for the function
        self.env.push_scope();
        self.push_fallible_scope();

        // Bind parameters with expected types (or declared/fresh if not enough info)
        for (i, param) in params.iter().enumerate() {
            let param_type = if i < expected_params.len() {
                expected_params[i].clone()
            } else if let Some(ann) = &param.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };

            // Define all identifiers from the pattern
            for name in param.get_identifiers() {
                self.env.define(&name, TypeScheme::mono(param_type.clone()));
            }
        }

        let inferred_result = self.infer_callable_return_type(body, true);
        let was_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        let constrained_expected_return =
            self.apply_fallibility_to_return_type(expected_return.clone(), was_fallible);
        if was_fallible && !self.is_result_type(&expected_return) {
            self.constraints
                .push((constrained_expected_return.clone(), expected_return.clone()));
        }

        // Constrain inferred callable return to expected return type
        self.constraints
            .push((inferred_return_type, constrained_expected_return.clone()));

        // Build the function type using Type::Function to preserve type variables
        let mut actual_param_types: Vec<Type> = Vec::with_capacity(params.len());
        for (i, p) in params.iter().enumerate() {
            let ty = if i < expected_params.len() {
                expected_params[i].clone()
            } else if let Some(ann) = &p.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };
            actual_param_types.push(ty);
        }

        Ok(Type::Function {
            params: actual_param_types,
            returns: Box::new(constrained_expected_return),
        })
    }

    /// Infer a function expression (fallback when no expected type)
    fn infer_function_expr(
        &mut self,
        params: &[FunctionParameter],
        return_type_ann: Option<&TypeAnnotation>,
        body: &[shape_ast::ast::Statement],
    ) -> TypeResult<Type> {
        self.env.push_scope();
        self.push_fallible_scope();

        let mut param_types = Vec::new();
        for param in params {
            let param_type = if let Some(ann) = &param.type_annotation {
                Type::Concrete(ann.clone())
            } else {
                self.fresh_type_var()
            };

            // Define all identifiers from the pattern
            for name in param.get_identifiers() {
                self.env.define(&name, TypeScheme::mono(param_type.clone()));
            }
            param_types.push(param_type);
        }

        let inferred_result = self.infer_callable_return_type(body, return_type_ann.is_some());
        let was_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        // If return type is annotated, constrain inferred type to annotation.
        // When the annotation is a Result/Option and the inferred body type is
        // a bare success value, constrain against the success type (Shape
        // implicitly Ok/Some-wraps the return value of a fallible/optional
        // function).
        let return_type = if let Some(ann) = return_type_ann {
            let annotated = Type::Concrete(ann.clone());
            self.push_return_constraint(inferred_return_type, annotated.clone());
            annotated
        } else {
            inferred_return_type
        };
        let return_type = self.apply_fallibility_to_return_type(return_type, was_fallible);

        // Build function type using Type::Function to preserve type variables
        Ok(Type::Function {
            params: param_types,
            returns: Box::new(return_type),
        })
    }

    /// Check array elements against expected element type
    fn check_array_against(&mut self, elements: &[Expr], elem_type: &Type) -> TypeResult<Type> {
        for elem in elements {
            self.check_against(elem, elem_type)?;
        }
        Ok(BuiltinTypes::array(elem_type.clone()))
    }

    /// Check object entries against expected field types
    fn check_object_against(
        &mut self,
        entries: &[ObjectEntry],
        expected_fields: &[shape_ast::ast::ObjectTypeField],
    ) -> TypeResult<Type> {
        let mut result_fields = Vec::new();

        for entry in entries {
            match entry {
                ObjectEntry::Field {
                    key,
                    value,
                    type_annotation: _type_annotation,
                } => {
                    // Find expected field type if available
                    let expected_field_type = expected_fields
                        .iter()
                        .find(|f| &f.name == key)
                        .map(|f| Type::Concrete(f.type_annotation.clone()));

                    let field_type = if let Some(expected) = expected_field_type {
                        self.check_against(value, &expected)?
                    } else {
                        self.infer_expr(value)?
                    };

                    result_fields.push(shape_ast::ast::ObjectTypeField {
                        name: key.clone(),
                        optional: false,
                        type_annotation: field_type
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                        annotations: vec![],
                    });
                }
                ObjectEntry::Spread(expr) => {
                    // Infer the type of the spread expression and merge its fields.
                    // Explicit fields declared later in the literal override spread fields.
                    let spread_type = self.infer_expr(expr)?;
                    let spread_fields = self.extract_object_fields(&spread_type);
                    for sf in spread_fields {
                        result_fields.push(sf);
                    }
                }
            }
        }

        // Deduplicate fields: later entries (explicit fields) override earlier ones (spread fields).
        // This matches JS/TS semantics: { ...obj, x: 1 } means x: 1 overrides obj.x.
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for field in result_fields.into_iter().rev() {
            if seen.insert(field.name.clone()) {
                deduped.push(field);
            }
        }
        deduped.reverse();

        Ok(Type::Concrete(TypeAnnotation::Object(deduped)))
    }

    /// Extract object-typed fields from a type for spread merging.
    ///
    /// Handles:
    /// - `Type::Concrete(TypeAnnotation::Object(fields))` -- inline object types
    /// - `Type::Concrete(TypeAnnotation::Reference(name))` -- named struct types via type alias
    ///   or struct_type_defs lookup
    fn extract_object_fields(&self, ty: &Type) -> Vec<shape_ast::ast::ObjectTypeField> {
        match ty {
            Type::Concrete(TypeAnnotation::Object(fields)) => fields.clone(),
            Type::Concrete(TypeAnnotation::Reference(name)) => {
                // Try struct_type_defs first (registered during hoisting)
                if let Some(struct_def) = self.struct_type_defs.get(name.as_str()) {
                    return struct_def
                        .fields
                        .iter()
                        .map(|f| shape_ast::ast::ObjectTypeField {
                            name: f.name.clone(),
                            optional: false,
                            type_annotation: f.type_annotation.clone(),
                            annotations: vec![],
                        })
                        .collect();
                }
                // Fall back to type alias lookup (struct types are stored as Object aliases)
                if let Some(alias) = self.env.lookup_type_alias(name) {
                    if let TypeAnnotation::Object(fields) = &alias.type_annotation {
                        return fields.clone();
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_mode_expected() {
        let mode = CheckMode::Infer;
        assert!(mode.expected().is_none());

        let mode = CheckMode::Check(BuiltinTypes::number());
        assert!(mode.expected().is_some());

        let mode = CheckMode::Synth(BuiltinTypes::string());
        assert!(mode.expected().is_some());
    }

    #[test]
    fn test_check_mode_is_hard_constraint() {
        assert!(!CheckMode::Infer.is_hard_constraint());
        assert!(CheckMode::Check(BuiltinTypes::number()).is_hard_constraint());
        assert!(!CheckMode::Synth(BuiltinTypes::number()).is_hard_constraint());
    }

    #[test]
    fn test_extract_object_fields_from_inline_object() {
        let engine = super::super::TypeInferenceEngine::new();
        let ty = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
            shape_ast::ast::ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("string".to_string()),
                annotations: vec![],
            },
        ]));
        let fields = engine.extract_object_fields(&ty);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[1].name, "y");
    }

    #[test]
    fn test_extract_object_fields_from_unknown_returns_empty() {
        let engine = super::super::TypeInferenceEngine::new();
        let ty = BuiltinTypes::number();
        let fields = engine.extract_object_fields(&ty);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_object_fields_from_reference_via_alias() {
        let mut engine = super::super::TypeInferenceEngine::new();
        // Register a type alias: type Point = { x: int, y: int }
        engine.env.define_type_alias(
            "Point",
            &TypeAnnotation::Object(vec![
                shape_ast::ast::ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                shape_ast::ast::ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ]),
            None,
        );

        let ty = Type::Concrete(TypeAnnotation::Reference("Point".into()));
        let fields = engine.extract_object_fields(&ty);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[1].name, "y");
    }
}
