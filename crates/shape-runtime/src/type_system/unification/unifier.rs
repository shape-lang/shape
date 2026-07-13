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

        // Occurs check (full, including tyvar markers embedded inside a
        // `Concrete` annotation). Storing a cyclic binding `X |-> T` where `X`
        // occurs inside `T` — e.g. an object literal over its own unresolved
        // field var `X |-> Object({f: <tyvar X>})` — makes the mutually
        // recursive `apply_substitutions <-> apply_to_annotation` pair diverge
        // and overflow the stack. The trivial `Variable(v) == var` case above
        // catches only the direct self-binding; a marker buried in an
        // annotation slips past it. Refuse to store the cycle: the variable
        // stays unresolved and projects to `unknown` downstream — the same
        // honest "not inferred" outcome an unbound marker already produces
        // (see `tyvar_to_annotation` doc in types/core.rs). This is the
        // safety net for the many `bind` call sites that are not fronted by
        // the `occurs_in` guard in the constraint solver.
        if occurs_check(&var, &ty) {
            return;
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

            // ADR-009 B3 (S1): existential descriptor package type. Recurse into
            // the inner descriptor (it may carry tyvar markers); witnesses pass
            // through unchanged.
            TypeAnnotation::Existential { witnesses, inner } => TypeAnnotation::Existential {
                witnesses: witnesses.clone(),
                inner: Box::new(self.apply_to_annotation(inner)),
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

/// Occurs check: does `var` appear anywhere in `ty`?
///
/// Unlike a naive structural walk, this descends into `Type::Concrete`
/// annotations and decodes embedded `tyvar` markers (the SOH-prefixed
/// `Basic("\u{1}tyvar:..")` encoding used to keep an unresolved variable
/// recoverable inside an object-literal field type — see
/// `tyvar_to_annotation` in `types/core.rs`). A marker equal to `var` buried
/// inside `Object`/`Array`/`Function`/… must count as an occurrence, otherwise
/// a cyclic binding `X |-> Concrete(Object({f: <tyvar X>}))` is stored and the
/// substitution walk diverges.
pub fn occurs_check(var: &TypeVar, ty: &Type) -> bool {
    match ty {
        Type::Variable(v) => v == var,
        Type::Constrained { var: v, .. } => v == var,
        Type::Generic { base, args } => {
            occurs_check(var, base) || args.iter().any(|a| occurs_check(var, a))
        }
        Type::Function { params, returns } => {
            params.iter().any(|p| occurs_check(var, p)) || occurs_check(var, returns)
        }
        Type::Concrete(ann) => annotation_occurs(var, ann),
    }
}

/// Occurs check over a `TypeAnnotation`: does the tyvar marker for `var` appear
/// anywhere in `ann`? Walks every annotation form that can nest a marker.
fn annotation_occurs(var: &TypeVar, ann: &TypeAnnotation) -> bool {
    if let Some(v) = annotation_as_tyvar(ann) {
        return &v == var;
    }
    match ann {
        TypeAnnotation::Borrow { inner, .. } => annotation_occurs(var, inner),
        TypeAnnotation::Array(elem) => annotation_occurs(var, elem),
        TypeAnnotation::Tuple(elems) => elems.iter().any(|e| annotation_occurs(var, e)),
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|f| annotation_occurs(var, &f.type_annotation)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|p| annotation_occurs(var, &p.type_annotation))
                || annotation_occurs(var, returns)
        }
        TypeAnnotation::Union(types) | TypeAnnotation::Intersection(types) => {
            types.iter().any(|t| annotation_occurs(var, t))
        }
        TypeAnnotation::Generic { args, .. } => args.iter().any(|a| annotation_occurs(var, a)),
        // ADR-009 B3 (S1): existential descriptor package type — a tyvar marker
        // can be buried in the inner descriptor.
        TypeAnnotation::Existential { inner, .. } => annotation_occurs(var, inner),
        TypeAnnotation::Basic(_)
        | TypeAnnotation::Reference(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined
        | TypeAnnotation::Dyn(_) => false,
    }
}

#[cfg(test)]
mod occurs_check_tests {
    use super::*;
    use shape_ast::ast::ObjectTypeField;

    /// Build `Concrete(Object({ field_name: <tyvar var> }))` — the shape an
    /// object literal over an unresolved field variable freezes to.
    fn object_over_tyvar(field_name: &str, var: &TypeVar) -> Type {
        Type::Concrete(TypeAnnotation::Object(vec![ObjectTypeField {
            name: field_name.to_string(),
            optional: false,
            type_annotation: tyvar_to_annotation(var),
            annotations: vec![],
        }]))
    }

    #[test]
    fn occurs_check_sees_tyvar_marker_inside_concrete_object() {
        let x = TypeVar("X".to_string());
        let cyclic = object_over_tyvar("state", &x);
        assert!(
            occurs_check(&x, &cyclic),
            "occurs check must detect a tyvar marker embedded inside a \
             Concrete object annotation"
        );
        // A different variable must NOT be reported as occurring.
        let y = TypeVar("Y".to_string());
        assert!(!occurs_check(&y, &cyclic));
    }

    /// WF-6 regression: the std::finance compiler stack-overflow.
    ///
    /// Binding `X |-> Concrete(Object({ f: <tyvar X>}))` used to be stored
    /// because the occurs check treated every `Concrete(_)` as
    /// variable-free. The stored cycle then made
    /// `apply_substitutions <-> apply_to_annotation` recurse forever and blow
    /// the stack at compile time (surfaced by `from
    /// std::finance::backtest::engine use { backtest }`). `bind` must now
    /// refuse the cyclic binding, and substitution must terminate.
    #[test]
    fn bind_refuses_cyclic_object_binding_and_substitution_terminates() {
        let x = TypeVar("X".to_string());
        let cyclic = object_over_tyvar("state", &x);

        let mut unifier = Unifier::new();
        unifier.bind(x.clone(), cyclic);

        // The cycle must not have been stored.
        assert!(
            unifier.lookup(&x).is_none(),
            "cyclic binding X |-> Object({{state: X}}) must be refused, not stored"
        );

        // Substitution over the (still-unbound) variable must terminate and
        // leave it as an honest unresolved variable — no stack overflow.
        let resolved = unifier.apply_substitutions(&Type::Variable(x.clone()));
        assert_eq!(resolved, Type::Variable(x));
    }

    #[test]
    fn bind_still_stores_acyclic_object_binding() {
        // A non-self-referential object binding must still be stored.
        let x = TypeVar("X".to_string());
        let y = TypeVar("Y".to_string());
        let acyclic = object_over_tyvar("field", &y); // mentions Y, not X

        let mut unifier = Unifier::new();
        unifier.bind(x.clone(), acyclic.clone());
        assert_eq!(unifier.lookup(&x), Some(&acyclic));
    }
}
