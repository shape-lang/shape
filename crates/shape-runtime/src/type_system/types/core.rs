//! Inference types, variables, and polymorphic schemes.

use shape_ast::ast::TypeAnnotation;
use std::collections::HashMap;

use super::builtins::BuiltinTypes;
use super::constraints::TypeConstraint;
pub use super::type_var::{
    DeclaredTypeVarOwner, DeclaredTypeVarProvenance, TYVAR_ANNOTATION_PREFIX, TypeVar, TypeVarGen,
    annotation_as_tyvar, annotation_contains_reserved_type_var_carrier, tyvar_to_annotation,
};
use crate::type_system::effects::EffectRow;
use crate::type_system::semantic::SemanticType;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Concrete(TypeAnnotation),
    Variable(TypeVar),
    Generic {
        base: Box<Type>,
        args: Vec<Type>,
    },
    Constrained {
        var: TypeVar,
        constraint: Box<TypeConstraint>,
    },
    /// ADR-014 §8.1: the effect row is a component of the function type. Two
    /// function types that differ only in row are different types, and the row
    /// participates in structural equality, unification, and subtyping.
    ///
    /// Construct through [`Type::function`] (row not derived) or
    /// [`Type::function_with_effects`] (row known) rather than by literal, so
    /// that every site that *does* know a row is greppable.
    Function {
        params: Vec<Type>,
        returns: Box<Type>,
        effects: EffectRow,
    },
}

impl Type {
    /// A function type whose row this call site does not derive. The row is
    /// [`EffectRow::Unproven`] — a proof gap, explicitly not a purity claim.
    pub fn function(params: Vec<Type>, returns: Type) -> Type {
        Type::Function {
            params,
            returns: Box::new(returns),
            effects: EffectRow::Unproven,
        }
    }

    /// A function type with a known row: a declared `!` clause, an inferred
    /// body row, or an `effect F` binder.
    pub fn function_with_effects(params: Vec<Type>, returns: Type, effects: EffectRow) -> Type {
        Type::Function {
            params,
            returns: Box::new(returns),
            effects,
        }
    }

    /// The row component, for any function type; `None` for non-functions.
    pub fn effect_row(&self) -> Option<&EffectRow> {
        match self {
            Type::Function { effects, .. } => Some(effects),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypeScheme {
    pub quantified: Vec<TypeVar>,
    pub ty: Type,
    pub trait_bounds: HashMap<TypeVar, Vec<String>>,
    pub default_types: HashMap<TypeVar, Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredTypeVarInstantiation {
    declared: TypeVar,
    instantiated: TypeVar,
}

impl DeclaredTypeVarInstantiation {
    pub fn declared(&self) -> &TypeVar {
        &self.declared
    }

    pub fn instantiated(&self) -> &TypeVar {
        &self.instantiated
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeSchemeInstantiation {
    pub ty: Type,
    pub bound_constraints: Vec<(Type, Type)>,
    pub default_substitutions: HashMap<TypeVar, Type>,
    pub declared_instantiations: Vec<DeclaredTypeVarInstantiation>,
}

impl TypeScheme {
    pub fn mono(ty: Type) -> Self {
        TypeScheme {
            quantified: vec![],
            ty,
            trait_bounds: HashMap::new(),
            default_types: HashMap::new(),
        }
    }

    pub fn poly(quantified: Vec<TypeVar>, ty: Type) -> Self {
        TypeScheme {
            quantified,
            ty,
            trait_bounds: HashMap::new(),
            default_types: HashMap::new(),
        }
    }

    pub fn poly_bounded(
        quantified: Vec<TypeVar>,
        ty: Type,
        trait_bounds: HashMap<String, Vec<String>>,
    ) -> Self {
        let trait_bounds = Self::bind_named_metadata(&quantified, trait_bounds);
        TypeScheme {
            quantified,
            ty,
            trait_bounds,
            default_types: HashMap::new(),
        }
    }

    pub fn poly_bounded_with_defaults(
        quantified: Vec<TypeVar>,
        ty: Type,
        trait_bounds: HashMap<String, Vec<String>>,
        default_types: HashMap<String, Type>,
    ) -> Self {
        let trait_bounds = Self::bind_named_metadata(&quantified, trait_bounds);
        let default_types = Self::bind_named_metadata(&quantified, default_types);
        TypeScheme {
            quantified,
            ty,
            trait_bounds,
            default_types,
        }
    }

    pub fn poly_bounded_with_exact_defaults(
        quantified: Vec<TypeVar>,
        ty: Type,
        trait_bounds: HashMap<TypeVar, Vec<String>>,
        default_types: HashMap<TypeVar, Type>,
    ) -> Self {
        TypeScheme {
            quantified,
            ty,
            trait_bounds,
            default_types,
        }
    }

    fn bind_named_metadata<T>(
        quantified: &[TypeVar],
        mut metadata: HashMap<String, T>,
    ) -> HashMap<TypeVar, T> {
        quantified
            .iter()
            .filter_map(|var| {
                metadata
                    .remove(var.presentation_name().as_ref())
                    .map(|value| (var.clone(), value))
            })
            .collect()
    }

    pub fn instantiate_with_bounds(
        &self,
        var_gen: &mut TypeVarGen,
    ) -> (Type, Vec<(Type, Type)>, HashMap<TypeVar, Type>) {
        let instance = self.instantiate_with_metadata(var_gen);
        (
            instance.ty,
            instance.bound_constraints,
            instance.default_substitutions,
        )
    }

    pub fn instantiate_with_metadata(&self, var_gen: &mut TypeVarGen) -> TypeSchemeInstantiation {
        if self.quantified.is_empty() {
            return TypeSchemeInstantiation {
                ty: self.ty.clone(),
                bound_constraints: Vec::new(),
                default_substitutions: HashMap::new(),
                declared_instantiations: Vec::new(),
            };
        }

        let mut subst = HashMap::new();
        let mut constraints = Vec::new();
        let mut defaults = HashMap::new();
        let mut declared_instantiations = Vec::new();

        for var in &self.quantified {
            let fresh = var_gen.fresh_var();
            subst.insert(var.clone(), Type::Variable(fresh.clone()));
            if var.declared_provenance().is_some() {
                declared_instantiations.push(DeclaredTypeVarInstantiation {
                    declared: var.clone(),
                    instantiated: fresh.clone(),
                });
            }

            if let Some(bounds) = self.trait_bounds.get(var) {
                for trait_name in bounds {
                    let bound_var = var_gen.fresh_var();
                    constraints.push((
                        Type::Variable(fresh.clone()),
                        Type::Constrained {
                            var: bound_var,
                            constraint: Box::new(
                                super::constraints::TypeConstraint::ImplementsTrait {
                                    trait_name: trait_name.clone(),
                                },
                            ),
                        },
                    ));
                }
            }
        }

        for var in &self.quantified {
            let Some(Type::Variable(fresh_var)) = subst.get(var) else {
                continue;
            };
            if let Some(default_ty) = self.default_types.get(var) {
                defaults.insert(fresh_var.clone(), substitute(default_ty, &subst));
            }
        }

        TypeSchemeInstantiation {
            ty: substitute(&self.ty, &subst),
            bound_constraints: constraints,
            default_substitutions: defaults,
            declared_instantiations,
        }
    }

    pub fn instantiate(&self, var_gen: &mut TypeVarGen) -> Type {
        if self.quantified.is_empty() {
            return self.ty.clone();
        }

        let mut subst = HashMap::new();
        for var in &self.quantified {
            subst.insert(var.clone(), var_gen.fresh_type());
        }

        substitute(&self.ty, &subst)
    }

    pub fn is_polymorphic(&self) -> bool {
        !self.quantified.is_empty()
    }

    pub fn type_params(&self) -> &[TypeVar] {
        &self.quantified
    }
}

pub fn substitute(ty: &Type, subst: &HashMap<TypeVar, Type>) -> Type {
    match ty {
        Type::Variable(var) => subst.get(var).cloned().unwrap_or_else(|| ty.clone()),
        Type::Generic { base, args } => Type::Generic {
            base: Box::new(substitute(base, subst)),
            args: args.iter().map(|arg| substitute(arg, subst)).collect(),
        },
        Type::Constrained { var, constraint } => {
            if let Some(replacement) = subst.get(var) {
                replacement.clone()
            } else {
                Type::Constrained {
                    var: var.clone(),
                    constraint: constraint.clone(),
                }
            }
        }
        Type::Function {
            params, returns, ..
        } => Type::Function {
            params: params.iter().map(|p| substitute(p, subst)).collect(),
            returns: Box::new(substitute(returns, subst)),
            effects: EffectRow::Unproven,
        },
        Type::Concrete(_) => ty.clone(),
    }
}

impl Type {
    pub fn declared_type_var_provenance(&self) -> Option<DeclaredTypeVarProvenance<'_>> {
        match self {
            Self::Variable(var) => var.declared_provenance(),
            _ => None,
        }
    }

    /// Normalize collections and functions to the inference-level carrier.
    pub fn canonicalize(&self) -> Type {
        match self {
            Type::Concrete(ann) => Self::canonicalize_annotation(ann),
            Type::Variable(_) => self.clone(),
            Type::Constrained { .. } => self.clone(),
            Type::Generic { base, args } => Type::Generic {
                base: Box::new(Self::canonicalize_collection_base(base)),
                args: args.iter().map(|a| a.canonicalize()).collect(),
            },
            Type::Function {
                params, returns, ..
            } => Type::Function {
                params: params.iter().map(|p| p.canonicalize()).collect(),
                returns: Box::new(returns.canonicalize()),
                effects: EffectRow::Unproven,
            },
        }
    }

    /// Collapse the `Vec` alias to the canonical inference spelling `Array`.
    fn canonicalize_collection_base(base: &Type) -> Type {
        if let Type::Concrete(TypeAnnotation::Reference(tp)) = base {
            if tp.to_string() == "Vec" {
                return Type::Concrete(TypeAnnotation::Reference("Array".into()));
            }
        }
        base.canonicalize()
    }

    fn canonicalize_annotation(ann: &TypeAnnotation) -> Type {
        match ann {
            TypeAnnotation::Array(elem) => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference("Array".into()))),
                args: vec![Self::canonicalize_annotation(elem)],
            },
            TypeAnnotation::Generic { name, args } => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                    name.as_str().into(),
                ))),
                args: args.iter().map(Self::canonicalize_annotation).collect(),
            },
            TypeAnnotation::Function { params, returns } => Type::Function {
                params: params
                    .iter()
                    .map(|p| Self::canonicalize_annotation(&p.type_annotation))
                    .collect(),
                returns: Box::new(Self::canonicalize_annotation(returns)),
                effects: EffectRow::Unproven,
            },
            other => Type::Concrete(other.clone()),
        }
    }

    pub fn to_annotation(&self) -> Option<TypeAnnotation> {
        match self {
            Type::Concrete(ann) => Some(ann.clone()),
            Type::Variable(_) => None, // Cannot convert unresolved type var
            Type::Generic { base, args } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    let arg_annotations: Option<Vec<_>> =
                        args.iter().map(|arg| arg.to_annotation()).collect();

                    arg_annotations.map(|args| {
                        if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 {
                            TypeAnnotation::Array(Box::new(args.into_iter().next().unwrap()))
                        } else {
                            TypeAnnotation::Generic {
                                name: name.clone(),
                                args,
                            }
                        }
                    })
                } else {
                    None
                }
            }
            Type::Constrained { .. } => None, // Cannot convert constrained type
            Type::Function {
                params, returns, ..
            } => {
                let param_anns: Vec<_> = params
                    .iter()
                    .map(|p| shape_ast::ast::FunctionParam {
                        name: None,
                        optional: false,
                        type_annotation: p
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                    })
                    .collect();
                let ret_ann = returns
                    .to_annotation()
                    .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()));
                Some(TypeAnnotation::Function {
                    params: param_anns,
                    returns: Box::new(ret_ann),
                })
            }
        }
    }

    pub fn to_semantic(&self) -> Option<SemanticType> {
        match self {
            Type::Concrete(ann) if annotation_contains_reserved_type_var_carrier(ann) => None,
            Type::Concrete(ann) => Some(super::annotations::annotation_to_semantic(ann)),
            Type::Variable(var) => Some(SemanticType::TypeVar(var.legacy_semantic_id()?)),
            Type::Generic { base, args } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    let semantic_args: Vec<_> =
                        args.iter().map(Type::to_semantic).collect::<Option<_>>()?;

                    match name.as_str() {
                        "Option" if semantic_args.len() == 1 => {
                            Some(SemanticType::Option(Box::new(semantic_args[0].clone())))
                        }
                        "Result" if !semantic_args.is_empty() => Some(SemanticType::Result {
                            ok_type: Box::new(semantic_args[0].clone()),
                            err_type: semantic_args.get(1).cloned().map(Box::new),
                        }),
                        "Vec" | "Array" if semantic_args.len() == 1 => {
                            Some(SemanticType::Array(Box::new(semantic_args[0].clone())))
                        }
                        _ => Some(SemanticType::Generic {
                            name: name.to_string(),
                            args: semantic_args,
                        }),
                    }
                } else {
                    None
                }
            }
            Type::Constrained { var, .. } => Some(SemanticType::TypeVar(var.legacy_semantic_id()?)),
            Type::Function {
                params, returns, ..
            } => {
                let semantic_params: Vec<_> = params
                    .iter()
                    .map(|p| {
                        p.to_semantic()
                            .map(|st| crate::type_system::semantic::FunctionParam {
                                name: None,
                                param_type: st,
                                optional: false,
                            })
                    })
                    .collect::<Option<_>>()?;
                let return_type = returns.to_semantic()?;
                Some(SemanticType::Function(Box::new(
                    crate::type_system::semantic::FunctionSignature {
                        params: semantic_params,
                        return_type,
                        is_fallible: false,
                    },
                )))
            }
        }
    }
}

impl SemanticType {
    pub fn to_inference_type(&self) -> Type {
        match self {
            SemanticType::Number => Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            SemanticType::Integer => Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            SemanticType::Bool => Type::Concrete(TypeAnnotation::Basic("bool".to_string())),
            SemanticType::String => Type::Concrete(TypeAnnotation::Basic("string".to_string())),
            SemanticType::Option(inner) => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
                args: vec![inner.to_inference_type()],
            },
            SemanticType::Result { ok_type, err_type } => {
                let mut args = vec![ok_type.to_inference_type()];
                if let Some(err) = err_type {
                    args.push(err.to_inference_type());
                }
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
                    args,
                }
            }
            SemanticType::Array(elem) => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
                args: vec![elem.to_inference_type()],
            },
            SemanticType::TypeVar(id) => Type::Variable(TypeVar::new(format!("T{}", id.0))),
            SemanticType::Named(name) => {
                if BuiltinTypes::is_number_type_name(name)
                    || BuiltinTypes::is_integer_type_name(name)
                    || BuiltinTypes::is_bool_type_name(name)
                    || BuiltinTypes::is_string_type_name(name)
                {
                    Type::Concrete(TypeAnnotation::Basic(name.clone()))
                } else {
                    Type::Concrete(TypeAnnotation::Reference(name.as_str().into()))
                }
            }
            SemanticType::Generic { name, args } => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                    name.as_str().into(),
                ))),
                args: args.iter().map(|a| a.to_inference_type()).collect(),
            },
            SemanticType::Void => Type::Concrete(TypeAnnotation::Void),
            SemanticType::Never => Type::Concrete(TypeAnnotation::Never),
            SemanticType::Function(sig) => {
                let param_types: Vec<_> = sig
                    .params
                    .iter()
                    .map(|p| p.param_type.to_inference_type())
                    .collect();
                Type::Function {
                    params: param_types,
                    returns: Box::new(sig.return_type.to_inference_type()),
                    effects: EffectRow::Unproven,
                }
            }
            SemanticType::Struct { name, fields } => {
                let obj_fields: Vec<_> = fields
                    .iter()
                    .map(|(n, t)| shape_ast::ast::ObjectTypeField {
                        name: n.clone(),
                        optional: false,
                        type_annotation: super::annotations::semantic_to_annotation(t),
                        annotations: vec![],
                    })
                    .collect();
                if name == "Object" || name == "Tuple" {
                    Type::Concrete(TypeAnnotation::Object(obj_fields))
                } else {
                    Type::Concrete(TypeAnnotation::Reference(name.as_str().into()))
                }
            }
            SemanticType::Enum { name, .. } => {
                Type::Concrete(TypeAnnotation::Reference(name.as_str().into()))
            }
            SemanticType::Ref(inner) | SemanticType::RefMut(inner) => inner.to_inference_type(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::BuiltinTypes;

    #[test]
    fn test_type_to_semantic_primitives() {
        let num = BuiltinTypes::number();
        let semantic = num.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::Number);

        let string = BuiltinTypes::string();
        let semantic = string.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::String);

        let boolean = BuiltinTypes::boolean();
        let semantic = boolean.to_semantic().unwrap();
        assert_eq!(semantic, SemanticType::Bool);
    }

    #[test]
    fn test_type_to_semantic_option() {
        let option_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let semantic = option_num.to_semantic().unwrap();
        assert_eq!(
            semantic,
            SemanticType::Option(Box::new(SemanticType::Number))
        );
    }

    #[test]
    fn test_type_to_semantic_result() {
        let result_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let semantic = result_num.to_semantic().unwrap();
        assert_eq!(
            semantic,
            SemanticType::Result {
                ok_type: Box::new(SemanticType::Number),
                err_type: None
            }
        );
    }

    #[test]
    fn test_semantic_to_inference_roundtrip() {
        let original = SemanticType::Option(Box::new(SemanticType::Number));
        let inference = original.to_inference_type();
        let roundtrip = inference.to_semantic().unwrap();
        assert_eq!(original, roundtrip);
    }
}
