//! Type Unification
//!
//! Implements unification algorithm for type inference,
//! maintaining substitutions and applying them to types.

use crate::type_system::{Type, TypeVar, annotation_as_tyvar, tyvar_to_annotation};
use shape_ast::ast::TypeAnnotation;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Unifier {
    /// Substitution map from type variables to types
    substitutions: HashMap<TypeVar, Type>,
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Unifier {
    pub fn new() -> Self {
        Unifier {
            substitutions: HashMap::new(),
        }
    }

    /// Bind a type variable to a type
    pub fn bind(&mut self, var: TypeVar, ty: Type) {
        // Apply existing substitutions to the type
        let ty = self.apply_substitutions(&ty);

        // Don't bind a variable to itself
        if let Type::Variable(v) = &ty {
            if v == &var {
                return;
            }
        }

        self.substitutions.insert(var, ty);
    }

    /// Look up a type variable's binding
    pub fn lookup(&self, var: &TypeVar) -> Option<&Type> {
        self.substitutions.get(var)
    }

    /// Apply substitutions to a type
    pub fn apply_substitutions(&self, ty: &Type) -> Type {
        match ty {
            Type::Variable(var) => {
                if let Some(subst_ty) = self.substitutions.get(var) {
                    // Recursively apply substitutions
                    self.apply_substitutions(subst_ty)
                } else {
                    ty.clone()
                }
            }

            Type::Generic { base, args } => Type::Generic {
                base: Box::new(self.apply_substitutions(base)),
                args: args
                    .iter()
                    .map(|arg| self.apply_substitutions(arg))
                    .collect(),
            },

            Type::Constrained { var, constraint: _ } => {
                if let Some(subst_ty) = self.substitutions.get(var) {
                    self.apply_substitutions(subst_ty)
                } else {
                    ty.clone()
                }
            }

            Type::Function { params, returns } => Type::Function {
                params: params.iter().map(|p| self.apply_substitutions(p)).collect(),
                returns: Box::new(self.apply_substitutions(returns)),
            },

            // A concrete annotation can still embed `tyvar` markers — an
            // object literal over an unresolved parameter freezes to
            // `Object({field: <tyvar>})`. Recurse so the marker resolves
            // once the underlying variable is bound.
            Type::Concrete(ann) => Type::Concrete(self.apply_to_annotation(ann)),
        }
    }

    /// Apply substitutions to a type annotation
    pub fn apply_to_annotation(&self, ann: &TypeAnnotation) -> TypeAnnotation {
        // A `tyvar` marker (an object-literal field over an unresolved
        // variable) resolves through the substitution store. If the binding
        // is itself an unresolved variable the marker is re-encoded so a
        // later pass can finish the job; an unbound marker stays as-is and
        // projects to `unknown` downstream — an honest "not inferred".
        if let Some(var) = annotation_as_tyvar(ann) {
            return match self.substitutions.get(&var) {
                Some(_) => {
                    let resolved = self.apply_substitutions(&Type::Variable(var));
                    match &resolved {
                        Type::Variable(v) | Type::Constrained { var: v, .. } => {
                            tyvar_to_annotation(v)
                        }
                        _ => resolved.to_annotation().unwrap_or_else(|| ann.clone()),
                    }
                }
                None => ann.clone(),
            };
        }
        match ann {
            TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
                mutable: *mutable,
                inner: Box::new(self.apply_to_annotation(inner)),
            },

            TypeAnnotation::Array(elem) => {
                TypeAnnotation::Array(Box::new(self.apply_to_annotation(elem)))
            }

            TypeAnnotation::Tuple(elems) => TypeAnnotation::Tuple(
                elems
                    .iter()
                    .map(|elem| self.apply_to_annotation(elem))
                    .collect(),
            ),

            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|field| shape_ast::ast::ObjectTypeField {
                        name: field.name.clone(),
                        optional: field.optional,
                        type_annotation: self.apply_to_annotation(&field.type_annotation),
                        annotations: vec![],
                    })
                    .collect(),
            ),

            TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
                params: params
                    .iter()
                    .map(|param| shape_ast::ast::FunctionParam {
                        name: param.name.clone(),
                        optional: param.optional,
                        type_annotation: self.apply_to_annotation(&param.type_annotation),
                    })
                    .collect(),
                returns: Box::new(self.apply_to_annotation(returns)),
            },

            TypeAnnotation::Union(types) => TypeAnnotation::Union(
                types
                    .iter()
                    .map(|ty| self.apply_to_annotation(ty))
                    .collect(),
            ),

            TypeAnnotation::Intersection(types) => TypeAnnotation::Intersection(
                types
                    .iter()
                    .map(|ty| self.apply_to_annotation(ty))
                    .collect(),
            ),

            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.apply_to_annotation(arg))
                    .collect(),
            },

            // No substitutions needed for these
            TypeAnnotation::Basic(_)
            | TypeAnnotation::Reference(_)
            | TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => ann.clone(),
        }
    }

    /// Merge another unifier's substitutions
    pub fn merge(&mut self, other: &Unifier) {
        for (var, ty) in &other.substitutions {
            if !self.substitutions.contains_key(var) {
                self.bind(var.clone(), ty.clone());
            }
        }
    }

    /// Clear all substitutions
    pub fn clear(&mut self) {
        self.substitutions.clear();
    }

    /// Get all substitutions
    pub fn substitutions(&self) -> &HashMap<TypeVar, Type> {
        &self.substitutions
    }

    /// Try to unify two types without modifying this unifier's state
    ///
    /// Returns Ok(()) if the types can be unified, Err otherwise.
    /// This is useful for soft constraints in bidirectional type checking.
    pub fn try_unify(&self, t1: &Type, t2: &Type) -> Result<(), ()> {
        use crate::type_system::unification::types_equal;

        // Apply existing substitutions first
        let t1 = self.apply_substitutions(t1);
        let t2 = self.apply_substitutions(t2);

        // Check if types are structurally equal after substitution
        if types_equal(&t1, &t2) {
            return Ok(());
        }

        // If either is a type variable, they can potentially unify
        match (&t1, &t2) {
            (Type::Variable(_), _) | (_, Type::Variable(_)) => Ok(()),
            (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) => {
                let is_result_base = |base: &Type| match base {
                    Type::Concrete(TypeAnnotation::Reference(name)) => name == "Result",
                    Type::Concrete(TypeAnnotation::Basic(name)) => name == "Result",
                    _ => false,
                };

                self.try_unify(b1, b2)?;
                if a1.len() != a2.len() {
                    if is_result_base(b1) && is_result_base(b2) {
                        if (a1.len() == 1 && a2.len() == 2) || (a1.len() == 2 && a2.len() == 1) {
                            return self.try_unify(&a1[0], &a2[0]);
                        }
                        return Err(());
                    }
                    return Err(());
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.try_unify(arg1, arg2)?;
                }
                Ok(())
            }
            (Type::Concrete(ann1), Type::Concrete(ann2)) => {
                // `AnyError` is the top of the error lattice (the default `E` for
                // bare `Ok(..)`/`?`); it unifies with any concrete error type.
                // Bounded to the `AnyError` name so two distinct concrete named
                // error types still mismatch. Mirrors ConstraintSolver::is_any_error.
                let is_any_error = |ann: &TypeAnnotation| {
                    matches!(ann.as_type_name_str(), Some("AnyError"))
                };
                if crate::type_system::unification::annotations_equal(ann1, ann2)
                    || is_any_error(ann1)
                    || is_any_error(ann2)
                {
                    Ok(())
                } else {
                    Err(())
                }
            }
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
                    return Err(());
                }
                for (a, b) in p1.iter().zip(p2.iter()) {
                    self.try_unify(a, b)?;
                }
                self.try_unify(r1, r2)
            }
            _ => Err(()),
        }
    }
}
