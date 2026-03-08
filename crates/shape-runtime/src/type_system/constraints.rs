//! Type Constraint Solver
//!
//! Solves type constraints generated during type inference
//! to determine concrete types for type variables.

use super::checking::MethodTable;
use super::unification::Unifier;
use super::*;
use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
use std::collections::{HashMap, HashSet};

/// Check if a Type::Generic base is "Array" or "Vec".
fn is_array_or_vec_base(base: &Type) -> bool {
    match base {
        Type::Concrete(TypeAnnotation::Reference(name))
        | Type::Concrete(TypeAnnotation::Basic(name)) => name == "Array" || name == "Vec",
        _ => false,
    }
}

pub struct ConstraintSolver {
    /// Type unifier
    unifier: Unifier,
    /// Deferred constraints that couldn't be solved immediately.
    /// These are handled in solve() via multiple passes.
    _deferred: Vec<(Type, Type)>,
    /// Type variable bounds
    bounds: HashMap<TypeVar, TypeConstraint>,
    /// Method table for HasMethod constraint enforcement
    method_table: Option<MethodTable>,
    /// Trait implementation registry: set of "TraitName::TypeName" keys
    trait_impls: HashSet<String>,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintSolver {
    pub fn new() -> Self {
        ConstraintSolver {
            unifier: Unifier::new(),
            _deferred: Vec::new(),
            bounds: HashMap::new(),
            method_table: None,
            trait_impls: HashSet::new(),
        }
    }

    /// Attach a method table for HasMethod constraint enforcement.
    /// When set, HasMethod constraints are validated against this table
    /// instead of being accepted unconditionally.
    pub fn set_method_table(&mut self, table: MethodTable) {
        self.method_table = Some(table);
    }

    /// Register trait implementations for ImplementsTrait constraint enforcement.
    /// Each entry is a "TraitName::TypeName" key indicating that TypeName implements TraitName.
    pub fn set_trait_impls(&mut self, impls: HashSet<String>) {
        self.trait_impls = impls;
    }

    /// Solve all type constraints
    pub fn solve(&mut self, constraints: &mut Vec<(Type, Type)>) -> TypeResult<()> {
        // First pass: solve simple unification constraints
        let mut unsolved = Vec::new();

        for (t1, t2) in constraints.drain(..) {
            if self.solve_constraint(t1.clone(), t2.clone()).is_err() {
                // If we can't solve it now, defer it
                unsolved.push((t1, t2));
            }
        }

        // Second pass: try deferred constraints
        let mut made_progress = true;
        while made_progress && !unsolved.is_empty() {
            made_progress = false;
            let mut still_unsolved = Vec::new();

            for (t1, t2) in unsolved.drain(..) {
                if self.solve_constraint(t1.clone(), t2.clone()).is_err() {
                    still_unsolved.push((t1, t2));
                } else {
                    made_progress = true;
                }
            }

            unsolved = still_unsolved;
        }

        // Check if any constraints remain unsolved
        if !unsolved.is_empty() {
            return Err(TypeError::UnsolvedConstraints(unsolved));
        }

        // Apply bounds to type variables
        self.apply_bounds()?;

        Ok(())
    }

    /// Solve a single constraint
    fn solve_constraint(&mut self, t1: Type, t2: Type) -> TypeResult<()> {
        // Apply current substitutions before matching to avoid overwriting
        // existing bindings (e.g., T17=string overwritten by T17=T19 during
        // Function param/return pairwise unification).
        let t1 = self.unifier.apply_substitutions(&t1);
        let t2 = self.unifier.apply_substitutions(&t2);

        match (&t1, &t2) {
            // Variable constraints
            (Type::Variable(v1), Type::Variable(v2)) if v1 == v2 => Ok(()),

            // Constrained type variables — must be matched BEFORE the general
            // Variable arm, otherwise (Variable, Constrained) pairs are caught
            // by the Variable arm and the bound is never recorded.
            (Type::Constrained { var, constraint }, ty)
            | (ty, Type::Constrained { var, constraint }) => {
                // Record the constraint
                self.bounds.insert(var.clone(), *constraint.clone());

                // Unify with the underlying type
                self.solve_constraint(Type::Variable(var.clone()), ty.clone())
            }

            (Type::Variable(var), ty) | (ty, Type::Variable(var)) => {
                // Check occurs check
                if self.occurs_in(var, ty) {
                    return Err(TypeError::InfiniteType(var.clone()));
                }

                self.unifier.bind(var.clone(), ty.clone());
                Ok(())
            }

            // Concrete type constraints
            (Type::Concrete(ann1), Type::Concrete(ann2)) => {
                if self.unify_annotations(ann1, ann2)? {
                    Ok(())
                } else if Self::can_numeric_widen(ann1, ann2) {
                    // Implicit numeric promotion (int → number/float)
                    Ok(())
                } else {
                    Err(TypeError::TypeMismatch(
                        format!("{:?}", ann1),
                        format!("{:?}", ann2),
                    ))
                }
            }

            // Generic type constraints
            (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) => {
                self.solve_constraint(*b1.clone(), *b2.clone())?;

                let is_result_base = |base: &Type| {
                    matches!(
                        base,
                        Type::Concrete(TypeAnnotation::Reference(name))
                            | Type::Concrete(TypeAnnotation::Basic(name))
                            if name == "Result"
                    )
                };

                if a1.len() != a2.len() {
                    if is_result_base(&b1) && is_result_base(&b2) {
                        match (a1.len(), a2.len()) {
                            // `Result<T>` is error-agnostic shorthand and should unify
                            // with `Result<T, E>` by constraining only the success type.
                            (1, 2) | (2, 1) => {
                                self.solve_constraint(a1[0].clone(), a2[0].clone())?;
                                return Ok(());
                            }
                            _ => return Err(TypeError::ArityMismatch(a1.len(), a2.len())),
                        }
                    } else {
                        return Err(TypeError::ArityMismatch(a1.len(), a2.len()));
                    }
                }

                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.solve_constraint(arg1.clone(), arg2.clone())?;
                }

                Ok(())
            }

            // Function ~ Function: pairwise unify params + returns
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
                if p1.len() != p2.len() {
                    return Err(TypeError::ArityMismatch(p1.len(), p2.len()));
                }
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    // Parameter compatibility is checked from observed/actual to
                    // declared/expected shape so directional numeric widening
                    // (e.g. int -> number) remains valid in call constraints.
                    self.solve_constraint(param2.clone(), param1.clone())?;
                }
                self.solve_constraint(*r1.clone(), *r2.clone())
            }

            // Cross-compatibility: Type::Function ~ Concrete(TypeAnnotation::Function)
            (
                Type::Function {
                    params: fp,
                    returns: fr,
                },
                Type::Concrete(TypeAnnotation::Function {
                    params: cp,
                    returns: cr,
                }),
            )
            | (
                Type::Concrete(TypeAnnotation::Function {
                    params: cp,
                    returns: cr,
                }),
                Type::Function {
                    params: fp,
                    returns: fr,
                },
            ) => {
                if fp.len() != cp.len() {
                    return Err(TypeError::ArityMismatch(fp.len(), cp.len()));
                }
                for (f_param, c_param) in fp.iter().zip(cp.iter()) {
                    self.solve_constraint(
                        f_param.clone(),
                        Type::Concrete(c_param.type_annotation.clone()),
                    )?;
                }
                self.solve_constraint(*fr.clone(), Type::Concrete(*cr.clone()))
            }

            // Array<T> (Type::Generic with base "Array" or "Vec") ~ Concrete(Array(T))
            (Type::Generic { base, args }, Type::Concrete(TypeAnnotation::Array(elem)))
            | (Type::Concrete(TypeAnnotation::Array(elem)), Type::Generic { base, args })
                if args.len() == 1 && is_array_or_vec_base(base) =>
            {
                self.solve_constraint(args[0].clone(), Type::Concrete((**elem).clone()))
            }

            _ => Err(TypeError::TypeMismatch(
                format!("{:?}", t1),
                format!("{:?}", t2),
            )),
        }
    }

    /// Check if a type variable occurs in a type (occurs check)
    fn occurs_in(&self, var: &TypeVar, ty: &Type) -> bool {
        match ty {
            Type::Variable(v) => v == var,
            Type::Generic { base, args } => {
                self.occurs_in(var, base) || args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Constrained { var: v, .. } => v == var,
            Type::Function { params, returns } => {
                params.iter().any(|p| self.occurs_in(var, p)) || self.occurs_in(var, returns)
            }
            Type::Concrete(_) => false,
        }
    }

    /// Check if a numeric type can widen to another (directional).
    ///
    /// Integer-family types (`int`, `i16`, `u32`, `byte`, ...) can widen to
    /// number-family types (`number`, `f32`, `f64`, ...).
    /// `number → int` does NOT widen (lossy). `decimal → number` does NOT widen
    /// (different precision semantics).
    fn can_numeric_widen(from: &TypeAnnotation, to: &TypeAnnotation) -> bool {
        let from_name = match from {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => Some(name.as_str()),
            _ => None,
        };
        let to_name = match to {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => Some(name.as_str()),
            _ => None,
        };

        match (from_name, to_name) {
            (Some(f), Some(t)) => {
                BuiltinTypes::is_integer_type_name(f) && BuiltinTypes::is_number_type_name(t)
            }
            _ => false,
        }
    }

    /// Unify two type annotations
    fn unify_annotations(&self, ann1: &TypeAnnotation, ann2: &TypeAnnotation) -> TypeResult<bool> {
        match (ann1, ann2) {
            // Basic types
            (TypeAnnotation::Basic(_), TypeAnnotation::Basic(_)) => {
                Ok(ann1 == ann2 || Self::can_numeric_widen(ann1, ann2))
            }
            (TypeAnnotation::Reference(n1), TypeAnnotation::Reference(n2)) => Ok(n1 == n2),
            (TypeAnnotation::Basic(_), TypeAnnotation::Reference(_))
            | (TypeAnnotation::Reference(_), TypeAnnotation::Basic(_)) => {
                Ok(ann1 == ann2 || Self::can_numeric_widen(ann1, ann2))
            }

            // Array types
            (TypeAnnotation::Array(e1), TypeAnnotation::Array(e2)) => {
                self.unify_annotations(e1, e2)
            }

            // Tuple types
            (TypeAnnotation::Tuple(t1), TypeAnnotation::Tuple(t2)) => {
                if t1.len() != t2.len() {
                    return Ok(false);
                }

                for (elem1, elem2) in t1.iter().zip(t2.iter()) {
                    if !self.unify_annotations(elem1, elem2)? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }

            // Structural object types
            (TypeAnnotation::Object(f1), TypeAnnotation::Object(f2)) => {
                self.object_fields_compatible(f1, f2)
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
                    return Ok(false);
                }

                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    if !self.unify_annotations(&param1.type_annotation, &param2.type_annotation)? {
                        return Ok(false);
                    }
                }

                self.unify_annotations(r1, r2)
            }

            // Union types
            // A | B unifies with C | D if each type in one union can unify with at least one type in the other
            (TypeAnnotation::Union(u1), TypeAnnotation::Union(u2)) => {
                // Check that every type in u1 can unify with at least one type in u2
                for t1 in u1 {
                    let mut found_match = false;
                    for t2 in u2 {
                        if self.unify_annotations(t1, t2)? {
                            found_match = true;
                            break;
                        }
                    }
                    if !found_match {
                        return Ok(false);
                    }
                }
                // Check that every type in u2 can unify with at least one type in u1
                for t2 in u2 {
                    let mut found_match = false;
                    for t1 in u1 {
                        if self.unify_annotations(t1, t2)? {
                            found_match = true;
                            break;
                        }
                    }
                    if !found_match {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // Union with non-union: A | B unifies with C if either A or B unifies with C
            (TypeAnnotation::Union(union_types), other)
            | (other, TypeAnnotation::Union(union_types)) => {
                for union_type in union_types {
                    if self.unify_annotations(union_type, other)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            // Intersection types (order-independent)
            (TypeAnnotation::Intersection(i1), TypeAnnotation::Intersection(i2)) => {
                self.unify_annotation_sets(i1, i2)
            }

            // Void, Null, Undefined
            (TypeAnnotation::Void, TypeAnnotation::Void) => Ok(true),
            (TypeAnnotation::Null, TypeAnnotation::Null) => Ok(true),
            (TypeAnnotation::Undefined, TypeAnnotation::Undefined) => Ok(true),

            // Trait object types: dyn Trait1 + Trait2
            // Two trait objects unify if they have the same set of traits
            (TypeAnnotation::Dyn(traits1), TypeAnnotation::Dyn(traits2)) => {
                Ok(traits1.len() == traits2.len() && traits1.iter().all(|t| traits2.contains(t)))
            }

            // Array<T> (Generic) is equivalent to Vec<T> (Array)
            (TypeAnnotation::Generic { name, args }, TypeAnnotation::Array(elem))
            | (TypeAnnotation::Array(elem), TypeAnnotation::Generic { name, args })
                if name == "Array" && args.len() == 1 =>
            {
                self.unify_annotations(&args[0], elem)
            }

            // Different types don't unify
            _ => Ok(false),
        }
    }

    fn object_fields_compatible(
        &self,
        left: &[ObjectTypeField],
        right: &[ObjectTypeField],
    ) -> TypeResult<bool> {
        for left_field in left {
            let Some(right_field) = right.iter().find(|f| f.name == left_field.name) else {
                return Ok(false);
            };
            if left_field.optional != right_field.optional {
                return Ok(false);
            }
            if !self.unify_annotations(&left_field.type_annotation, &right_field.type_annotation)? {
                return Ok(false);
            }
        }
        if left.len() != right.len() {
            return Ok(false);
        }
        Ok(true)
    }

    fn unify_annotation_sets(
        &self,
        left: &[TypeAnnotation],
        right: &[TypeAnnotation],
    ) -> TypeResult<bool> {
        if left.len() != right.len() {
            return Ok(false);
        }

        let mut matched = vec![false; right.len()];
        for left_ann in left {
            let mut found = false;
            for (idx, right_ann) in right.iter().enumerate() {
                if matched[idx] {
                    continue;
                }
                if self.unify_annotations(left_ann, right_ann)? {
                    matched[idx] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Apply type variable bounds, propagating resolved field types back to type variables.
    ///
    /// When a `HasField` constraint is satisfied and the expected field type was a
    /// type variable, this binds that variable to the actual field type. This enables
    /// backward propagation: `let f = |obj| obj.x; f({x: 42})` resolves `obj.x` to `int`.
    fn apply_bounds(&mut self) -> TypeResult<()> {
        let mut new_bindings: Vec<(TypeVar, Type)> = Vec::new();

        for (var, constraint) in &self.bounds {
            // Use apply_substitutions to follow the full variable chain
            // (lookup only returns the direct binding, not the resolved type).
            let resolved = self
                .unifier
                .apply_substitutions(&Type::Variable(var.clone()));

            if let Type::Variable(_) = &resolved {
                // Still unresolved — skip for now
                continue;
            }

            self.check_constraint(&resolved, constraint)?;

            // Backward propagation: when a HasField constraint is satisfied,
            // bind the result type variable to the actual field type.
            if let TypeConstraint::HasField(field, expected_field_type) = constraint {
                if let Type::Variable(field_var) = expected_field_type.as_ref() {
                    // Also check if the field var is already resolved
                    let field_resolved = self
                        .unifier
                        .apply_substitutions(&Type::Variable(field_var.clone()));
                    if let Type::Variable(_) = &field_resolved {
                        // Field var still unresolved — try to bind it
                        if let Type::Concrete(TypeAnnotation::Object(fields)) = &resolved {
                            if let Some(found_field) = fields.iter().find(|f| f.name == *field) {
                                new_bindings.push((
                                    field_var.clone(),
                                    Type::Concrete(found_field.type_annotation.clone()),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Apply collected bindings
        for (var, ty) in new_bindings {
            self.unifier.bind(var, ty);
        }

        Ok(())
    }

    /// Check if a type satisfies a constraint
    fn check_constraint(&self, ty: &Type, constraint: &TypeConstraint) -> TypeResult<()> {
        match constraint {
            TypeConstraint::Numeric => match ty {
                Type::Concrete(TypeAnnotation::Basic(name))
                    if BuiltinTypes::is_numeric_type_name(name) =>
                {
                    Ok(())
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not numeric",
                    ty
                ))),
            },

            TypeConstraint::Comparable => match ty {
                Type::Concrete(TypeAnnotation::Basic(name))
                    if BuiltinTypes::is_numeric_type_name(name)
                        || name == "string"
                        || name == "bool" =>
                {
                    Ok(())
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not comparable",
                    ty
                ))),
            },

            TypeConstraint::Iterable => match ty {
                Type::Concrete(TypeAnnotation::Array(_)) => Ok(()),
                Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "string" || name == "rows" =>
                {
                    Ok(())
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not iterable",
                    ty
                ))),
            },

            TypeConstraint::HasField(field, expected_field_type) => {
                match ty {
                    Type::Concrete(TypeAnnotation::Object(fields)) => {
                        match fields.iter().find(|f| f.name == *field) {
                            Some(found_field) => {
                                // Check that field type matches expected type
                                if let Some(expected_ann) = expected_field_type.to_annotation() {
                                    if self.unify_annotations(
                                        &found_field.type_annotation,
                                        &expected_ann,
                                    )? {
                                        Ok(())
                                    } else {
                                        Err(TypeError::ConstraintViolation(format!(
                                            "field '{}' has type {:?}, expected {:?}",
                                            field, found_field.type_annotation, expected_ann
                                        )))
                                    }
                                } else {
                                    // Expected type is a type variable, accept any field type
                                    Ok(())
                                }
                            }
                            None => Err(TypeError::ConstraintViolation(format!(
                                "{:?} does not have field '{}'",
                                ty, field
                            ))),
                        }
                    }
                    Type::Concrete(TypeAnnotation::Basic(_name)) => {
                        // For named types, we assume property access was validated
                        // during inference using the schema registry. If a HasField
                        // constraint reaches here, it means the type wasn't a known
                        // schema type during inference, so we accept it tentatively.
                        // Runtime will do the final validation.
                        //
                        // Note: Previously this hardcoded "row" with OHLCV fields.
                        // Now schema validation happens in TypeInferenceEngine::infer_property_access.
                        Ok(())
                    }
                    _ => Err(TypeError::ConstraintViolation(format!(
                        "{:?} cannot have fields",
                        ty
                    ))),
                }
            }

            TypeConstraint::Callable {
                params: expected_params,
                returns: expected_returns,
            } => {
                match ty {
                    Type::Concrete(TypeAnnotation::Function {
                        params: actual_params,
                        returns: actual_returns,
                    }) => {
                        // Check parameter count matches
                        if expected_params.len() != actual_params.len() {
                            return Err(TypeError::ConstraintViolation(format!(
                                "function expects {} parameters, got {}",
                                expected_params.len(),
                                actual_params.len()
                            )));
                        }

                        // Check each parameter type (contravariant: expected <: actual)
                        for (expected, actual) in expected_params.iter().zip(actual_params.iter()) {
                            if let Some(expected_ann) = expected.to_annotation() {
                                if !self
                                    .unify_annotations(&expected_ann, &actual.type_annotation)?
                                {
                                    return Err(TypeError::ConstraintViolation(format!(
                                        "parameter type mismatch: expected {:?}, got {:?}",
                                        expected_ann, actual.type_annotation
                                    )));
                                }
                            }
                        }

                        // Check return type (covariant: actual <: expected)
                        if let Some(expected_ret_ann) = expected_returns.to_annotation() {
                            if !self.unify_annotations(actual_returns, &expected_ret_ann)? {
                                return Err(TypeError::ConstraintViolation(format!(
                                    "return type mismatch: expected {:?}, got {:?}",
                                    expected_ret_ann, actual_returns
                                )));
                            }
                        }

                        Ok(())
                    }
                    Type::Function {
                        params: actual_params,
                        returns: actual_returns,
                    } => {
                        if expected_params.len() != actual_params.len() {
                            return Err(TypeError::ConstraintViolation(format!(
                                "function expects {} parameters, got {}",
                                expected_params.len(),
                                actual_params.len()
                            )));
                        }
                        // Type::Function params are Type, not FunctionParam — compare directly
                        for (expected, actual) in expected_params.iter().zip(actual_params.iter()) {
                            if let (Some(e_ann), Some(a_ann)) =
                                (expected.to_annotation(), actual.to_annotation())
                            {
                                if !self.unify_annotations(&e_ann, &a_ann)? {
                                    return Err(TypeError::ConstraintViolation(format!(
                                        "parameter type mismatch: expected {:?}, got {:?}",
                                        e_ann, a_ann
                                    )));
                                }
                            }
                        }
                        if let (Some(e_ret), Some(a_ret)) = (
                            expected_returns.to_annotation(),
                            actual_returns.to_annotation(),
                        ) {
                            if !self.unify_annotations(&a_ret, &e_ret)? {
                                return Err(TypeError::ConstraintViolation(format!(
                                    "return type mismatch: expected {:?}, got {:?}",
                                    e_ret, a_ret
                                )));
                            }
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::ConstraintViolation(format!(
                        "{:?} is not callable",
                        ty
                    ))),
                }
            }

            TypeConstraint::OneOf(options) => {
                for option in options {
                    // If type matches any option, constraint is satisfied
                    if let Type::Concrete(ann) = option {
                        if let Type::Concrete(ty_ann) = ty {
                            if self.unify_annotations(ann, ty_ann).unwrap_or(false) {
                                return Ok(());
                            }
                        }
                    }
                }

                Err(TypeError::ConstraintViolation(format!(
                    "{:?} does not match any of {:?}",
                    ty, options
                )))
            }

            TypeConstraint::Extends(base) => {
                // Implement subtyping check
                self.is_subtype(ty, base)
            }

            TypeConstraint::ImplementsTrait { trait_name } => {
                match ty {
                    Type::Variable(_) => {
                        // Type variable not yet resolved — this is a compile error
                        // (no deferring per Sprint 2 spec)
                        Err(TypeError::TraitBoundViolation {
                            type_name: format!("{:?}", ty),
                            trait_name: trait_name.clone(),
                        })
                    }
                    Type::Concrete(ann) => {
                        let type_name = match ann {
                            TypeAnnotation::Basic(n) | TypeAnnotation::Reference(n) => n.clone(),
                            _ => format!("{:?}", ann),
                        };
                        if self.has_trait_impl(trait_name, &type_name) {
                            Ok(())
                        } else {
                            Err(TypeError::TraitBoundViolation {
                                type_name,
                                trait_name: trait_name.clone(),
                            })
                        }
                    }
                    Type::Generic { base, .. } => {
                        let type_name = if let Type::Concrete(
                            TypeAnnotation::Reference(n) | TypeAnnotation::Basic(n),
                        ) = base.as_ref()
                        {
                            n.clone()
                        } else {
                            format!("{:?}", base)
                        };
                        if self.has_trait_impl(trait_name, &type_name) {
                            Ok(())
                        } else {
                            Err(TypeError::TraitBoundViolation {
                                type_name,
                                trait_name: trait_name.clone(),
                            })
                        }
                    }
                    _ => Err(TypeError::TraitBoundViolation {
                        type_name: format!("{:?}", ty),
                        trait_name: trait_name.clone(),
                    }),
                }
            }

            TypeConstraint::HasMethod {
                method_name,
                arg_types: _,
                return_type: _,
            } => {
                // If we have a method table, enforce the constraint
                if let Some(method_table) = &self.method_table {
                    match ty {
                        Type::Variable(_) => Ok(()), // Unresolved type var, defer
                        Type::Concrete(ann) => {
                            let type_name = match ann {
                                TypeAnnotation::Basic(n) | TypeAnnotation::Reference(n) => {
                                    n.clone()
                                }
                                TypeAnnotation::Array(_) => "Vec".to_string(),
                                _ => return Ok(()), // Complex types: accept
                            };
                            if method_table.lookup(ty, method_name).is_some() {
                                Ok(())
                            } else {
                                Err(TypeError::MethodNotFound {
                                    type_name,
                                    method_name: method_name.clone(),
                                })
                            }
                        }
                        Type::Generic { base, .. } => {
                            if method_table.lookup(ty, method_name).is_some() {
                                Ok(())
                            } else {
                                let type_name =
                                    if let Type::Concrete(TypeAnnotation::Reference(n)) =
                                        base.as_ref()
                                    {
                                        n.clone()
                                    } else {
                                        format!("{:?}", base)
                                    };
                                Err(TypeError::MethodNotFound {
                                    type_name,
                                    method_name: method_name.clone(),
                                })
                            }
                        }
                        _ => Ok(()), // Function, Constrained: accept
                    }
                } else {
                    // No method table attached — accept all (backward compatible)
                    Ok(())
                }
            }
        }
    }

    /// Check if a type implements a trait, considering numeric widening.
    ///
    /// For example, `int` satisfies a trait bound if the trait is implemented for `number`,
    /// since `int` can widen to `number` in the type system.
    fn has_trait_impl(&self, trait_name: &str, type_name: &str) -> bool {
        let key = format!("{}::{}", trait_name, type_name);
        if self.trait_impls.contains(&key) {
            return true;
        }
        // Numeric widening: integer-family aliases can use number/float/f64 impls.
        if BuiltinTypes::is_integer_type_name(type_name) {
            for widen_to in &["number", "float", "f64"] {
                let widen_key = format!("{}::{}", trait_name, widen_to);
                if self.trait_impls.contains(&widen_key) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if ty is a subtype of base (ty <: base)
    /// Subtyping rules:
    /// - Same types are subtypes of each other
    /// - Any is a supertype of everything
    /// - Vec<A> <: Vec<B> if A <: B (covariant)
    /// - Function<P1, R1> <: Function<P2, R2> if P2 <: P1 (contravariant params) and R1 <: R2 (covariant return)
    fn is_subtype(&self, ty: &Type, base: &Type) -> TypeResult<()> {
        match (ty, base) {
            // Same types are subtypes
            (t1, t2) if t1 == t2 => Ok(()),

            // Type variables - if we can unify, it's compatible
            (Type::Variable(_), _) | (_, Type::Variable(_)) => Ok(()),

            // Array subtyping (covariant)
            (
                Type::Concrete(TypeAnnotation::Array(elem1)),
                Type::Concrete(TypeAnnotation::Array(elem2)),
            ) => {
                let t1 = Type::Concrete(*elem1.clone());
                let t2 = Type::Concrete(*elem2.clone());
                self.is_subtype(&t1, &t2)
            }

            // Function subtyping (contravariant params, covariant return)
            (
                Type::Concrete(TypeAnnotation::Function {
                    params: p1,
                    returns: r1,
                }),
                Type::Concrete(TypeAnnotation::Function {
                    params: p2,
                    returns: r2,
                }),
            ) => {
                // Check parameter count
                if p1.len() != p2.len() {
                    return Err(TypeError::ConstraintViolation(format!(
                        "function parameter count mismatch: {} vs {}",
                        p1.len(),
                        p2.len()
                    )));
                }

                // Contravariant: base params must be subtypes of ty params
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    let t1 = Type::Concrete(param2.type_annotation.clone());
                    let t2 = Type::Concrete(param1.type_annotation.clone());
                    self.is_subtype(&t1, &t2)?;
                }

                // Covariant: ty return must be subtype of base return
                let ret1 = Type::Concrete(*r1.clone());
                let ret2 = Type::Concrete(*r2.clone());
                self.is_subtype(&ret1, &ret2)
            }

            // Optional subtyping: T <: Option<T>
            (t, Type::Concrete(TypeAnnotation::Generic { name, args }))
                if name == "Option" && args.len() == 1 =>
            {
                let inner = Type::Concrete(args[0].clone());
                self.is_subtype(t, &inner)
            }

            // Type::Function subtyping (contravariant params, covariant return)
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
                if p1.len() != p2.len() {
                    return Err(TypeError::ConstraintViolation(format!(
                        "function parameter count mismatch: {} vs {}",
                        p1.len(),
                        p2.len()
                    )));
                }
                // Contravariant params
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    self.is_subtype(param2, param1)?;
                }
                // Covariant return
                self.is_subtype(r1, r2)
            }

            // Basic types - check if they unify
            (Type::Concrete(ann1), Type::Concrete(ann2)) => {
                if self.unify_annotations(ann1, ann2)? {
                    Ok(())
                } else {
                    Err(TypeError::ConstraintViolation(format!(
                        "{:?} is not a subtype of {:?}",
                        ty, base
                    )))
                }
            }

            // Default: not a subtype
            _ => Err(TypeError::ConstraintViolation(format!(
                "{:?} is not a subtype of {:?}",
                ty, base
            ))),
        }
    }

    /// Get the unifier for applying substitutions
    pub fn unifier(&self) -> &Unifier {
        &self.unifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::ObjectTypeField;

    #[test]
    fn test_hasfield_backward_propagation_binds_field_type() {
        // When a TypeVar has a HasField constraint and is resolved to a concrete
        // object type, the field's result type variable should be bound to the
        // actual field type. This enables backward type propagation.
        let mut solver = ConstraintSolver::new();

        let obj_var = TypeVar::fresh();
        let field_result_var = TypeVar::fresh();
        let bound_var = TypeVar::fresh();

        let mut constraints = vec![
            // obj_var ~ Constrained { var: bound_var, HasField("x", field_result_var) }
            // This records bound: bound_var → HasField("x", field_result_var)
            // and solves: bound_var ~ obj_var
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var,
                    constraint: Box::new(TypeConstraint::HasField(
                        "x".to_string(),
                        Box::new(Type::Variable(field_result_var.clone())),
                    )),
                },
            ),
            // obj_var = {x: int}
            (
                Type::Variable(obj_var),
                Type::Concrete(TypeAnnotation::Object(vec![ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                }])),
            ),
        ];

        solver.solve(&mut constraints).unwrap();

        // field_result_var should now be resolved to int via apply_bounds
        let resolved = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_result_var));
        match &resolved {
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                assert_eq!(name, "int", "field type should be int");
            }
            _ => panic!(
                "Expected field_result_var to be resolved to int, got {:?}",
                resolved
            ),
        }
    }

    #[test]
    fn test_hasfield_backward_propagation_multiple_fields() {
        // Test that multiple HasField constraints on the same object all propagate
        let mut solver = ConstraintSolver::new();

        let obj_var = TypeVar::fresh();
        let field_x_var = TypeVar::fresh();
        let field_y_var = TypeVar::fresh();
        let bound_var_x = TypeVar::fresh();
        let bound_var_y = TypeVar::fresh();

        let mut constraints = vec![
            // HasField("x", field_x_var)
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var_x,
                    constraint: Box::new(TypeConstraint::HasField(
                        "x".to_string(),
                        Box::new(Type::Variable(field_x_var.clone())),
                    )),
                },
            ),
            // HasField("y", field_y_var)
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var_y,
                    constraint: Box::new(TypeConstraint::HasField(
                        "y".to_string(),
                        Box::new(Type::Variable(field_y_var.clone())),
                    )),
                },
            ),
            // obj_var = {x: int, y: string}
            (
                Type::Variable(obj_var),
                Type::Concrete(TypeAnnotation::Object(vec![
                    ObjectTypeField {
                        name: "x".to_string(),
                        optional: false,
                        type_annotation: TypeAnnotation::Basic("int".to_string()),
                        annotations: vec![],
                    },
                    ObjectTypeField {
                        name: "y".to_string(),
                        optional: false,
                        type_annotation: TypeAnnotation::Basic("string".to_string()),
                        annotations: vec![],
                    },
                ])),
            ),
        ];

        solver.solve(&mut constraints).unwrap();

        let resolved_x = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_x_var));
        let resolved_y = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_y_var));

        match &resolved_x {
            Type::Concrete(TypeAnnotation::Basic(name)) => assert_eq!(name, "int"),
            _ => panic!("Expected x to be int, got {:?}", resolved_x),
        }
        match &resolved_y {
            Type::Concrete(TypeAnnotation::Basic(name)) => assert_eq!(name, "string"),
            _ => panic!("Expected y to be string, got {:?}", resolved_y),
        }
    }

    // ===== Fix 1: Numeric type preservation tests =====

    #[test]
    fn test_int_constrained_numeric_succeeds() {
        // Concrete(int) ~ Constrained(Numeric) should succeed
        let mut solver = ConstraintSolver::new();
        let bound_var = TypeVar::fresh();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::Numeric),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_numeric_widening_int_to_number() {
        // (Concrete(int), Concrete(number)) should succeed via widening
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_numeric_widening_width_aware_integer_to_float_family() {
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("i16".to_string())),
            Type::Concrete(TypeAnnotation::Basic("f32".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_no_widening_number_to_int() {
        // (Concrete(number), Concrete(int)) should fail — lossy
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_err());
    }

    #[test]
    fn test_decimal_constrained_numeric_succeeds() {
        let mut solver = ConstraintSolver::new();
        let bound_var = TypeVar::fresh();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::Numeric),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_comparable_accepts_int() {
        // int should be Comparable
        let mut solver = ConstraintSolver::new();
        let bound_var = TypeVar::fresh();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::Comparable),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    // ===== Fix 2: Type::Function tests =====

    #[test]
    fn test_function_type_preserves_variables() {
        // BuiltinTypes::function with Variable params should be Type::Function
        let param = Type::Variable(TypeVar::fresh());
        let ret = Type::Variable(TypeVar::fresh());
        let func = BuiltinTypes::function(vec![param.clone()], ret.clone());
        match func {
            Type::Function { params, returns } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0], param);
                assert_eq!(*returns, ret);
            }
            _ => panic!("Expected Type::Function, got {:?}", func),
        }
    }

    #[test]
    fn test_function_unification_binds_variables() {
        // (T1)->T2 ~ (number)->string should bind T1=number, T2=string
        let mut solver = ConstraintSolver::new();
        let t1 = TypeVar::fresh();
        let t2 = TypeVar::fresh();

        let mut constraints = vec![(
            Type::Function {
                params: vec![Type::Variable(t1.clone())],
                returns: Box::new(Type::Variable(t2.clone())),
            },
            Type::Function {
                params: vec![BuiltinTypes::number()],
                returns: Box::new(BuiltinTypes::string()),
            },
        )];

        solver.solve(&mut constraints).unwrap();

        let resolved_t1 = solver.unifier().apply_substitutions(&Type::Variable(t1));
        let resolved_t2 = solver.unifier().apply_substitutions(&Type::Variable(t2));
        assert_eq!(resolved_t1, BuiltinTypes::number());
        assert_eq!(resolved_t2, BuiltinTypes::string());
    }

    #[test]
    fn test_function_cross_unification_with_concrete() {
        // Type::Function ~ Concrete(TypeAnnotation::Function) should unify
        let mut solver = ConstraintSolver::new();
        let t1 = TypeVar::fresh();

        let concrete_func = Type::Concrete(TypeAnnotation::Function {
            params: vec![shape_ast::ast::FunctionParam {
                name: None,
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
            }],
            returns: Box::new(TypeAnnotation::Basic("string".to_string())),
        });

        let mut constraints = vec![(
            Type::Function {
                params: vec![Type::Variable(t1.clone())],
                returns: Box::new(BuiltinTypes::string()),
            },
            concrete_func,
        )];

        solver.solve(&mut constraints).unwrap();

        let resolved = solver.unifier().apply_substitutions(&Type::Variable(t1));
        assert_eq!(resolved, BuiltinTypes::number());
    }

    #[test]
    fn test_object_annotations_unify_structurally() {
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ])),
            Type::Concrete(TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ])),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_intersection_annotations_unify_order_independent() {
        let mut solver = ConstraintSolver::new();
        let obj_xy = TypeAnnotation::Object(vec![
            ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
            ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]);
        let obj_z = TypeAnnotation::Object(vec![ObjectTypeField {
            name: "z".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("int".to_string()),
            annotations: vec![],
        }]);

        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Intersection(vec![
                obj_xy.clone(),
                obj_z.clone(),
            ])),
            Type::Concrete(TypeAnnotation::Intersection(vec![obj_z, obj_xy])),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    // ===== Sprint 2: ImplementsTrait constraint tests =====

    #[test]
    fn test_implements_trait_satisfied() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Comparable::number".to_string());
        solver.set_trait_impls(impls);

        let bound_var = TypeVar::fresh();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Comparable".to_string(),
                }),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_implements_trait_violated() {
        let mut solver = ConstraintSolver::new();
        // No trait impls registered — string doesn't implement Comparable
        let bound_var = TypeVar::fresh();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("string".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Comparable".to_string(),
                }),
            },
        )];
        let result = solver.solve(&mut constraints);
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::TraitBoundViolation {
                type_name,
                trait_name,
            } => {
                assert_eq!(type_name, "string");
                assert_eq!(trait_name, "Comparable");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    #[test]
    fn test_implements_trait_via_variable_resolution() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Sortable::number".to_string());
        solver.set_trait_impls(impls);

        let type_var = TypeVar::fresh();
        let bound_var = TypeVar::fresh();

        let mut constraints = vec![
            // T: Sortable
            (
                Type::Variable(type_var.clone()),
                Type::Constrained {
                    var: bound_var,
                    constraint: Box::new(TypeConstraint::ImplementsTrait {
                        trait_name: "Sortable".to_string(),
                    }),
                },
            ),
            // T = number
            (
                Type::Variable(type_var),
                Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            ),
        ];
        assert!(
            solver.solve(&mut constraints).is_ok(),
            "T resolved to number which implements Sortable"
        );
    }
}
