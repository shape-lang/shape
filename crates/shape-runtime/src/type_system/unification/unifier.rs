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

    /// Clear all substitutions
    pub fn clear(&mut self) {
        self.substitutions.clear();
    }

    /// Get all substitutions
    pub fn substitutions(&self) -> &HashMap<TypeVar, Type> {
        &self.substitutions
    }
}
