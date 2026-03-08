//! Core Type Definitions
//!
//! Defines the fundamental types used in the type system:
//! - `Type`: The main type representation
//! - `TypeVar`: Type variables for inference
//! - `TypeScheme`: Polymorphic type schemes

use shape_ast::ast::TypeAnnotation;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::builtins::BuiltinTypes;
use super::constraints::TypeConstraint;
use crate::type_system::semantic::{SemanticType, TypeVarId};

/// Type variable for inference
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeVar(pub String);

impl TypeVar {
    pub fn new(name: String) -> Self {
        TypeVar(name)
    }

    pub fn fresh() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        TypeVar(format!("T{}", id))
    }
}

/// Inferred type representation
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Concrete type
    Concrete(TypeAnnotation),
    /// Type variable (unknown type to be inferred)
    Variable(TypeVar),
    /// Generic type instantiation
    Generic { base: Box<Type>, args: Vec<Type> },
    /// Constrained type variable
    Constrained {
        var: TypeVar,
        constraint: Box<TypeConstraint>,
    },
    /// Function type with inference-level params and return type.
    /// Unlike Concrete(TypeAnnotation::Function), this can hold Type::Variable
    /// for params/returns, enabling type variable propagation through function calls.
    Function {
        params: Vec<Type>,
        returns: Box<Type>,
    },
}

/// Type scheme for polymorphic types
#[derive(Debug, Clone)]
pub struct TypeScheme {
    /// Quantified type variables
    pub quantified: Vec<TypeVar>,
    /// The actual type
    pub ty: Type,
    /// Trait bounds for quantified type variables: var_name → list of trait names
    pub trait_bounds: HashMap<String, Vec<String>>,
    /// Default types for quantified type variables: var_name → default type
    pub default_types: HashMap<String, Type>,
}

impl TypeScheme {
    /// Create a monomorphic type scheme
    pub fn mono(ty: Type) -> Self {
        TypeScheme {
            quantified: vec![],
            ty,
            trait_bounds: HashMap::new(),
            default_types: HashMap::new(),
        }
    }

    /// Create a polymorphic type scheme with quantified type variables
    pub fn poly(quantified: Vec<TypeVar>, ty: Type) -> Self {
        TypeScheme {
            quantified,
            ty,
            trait_bounds: HashMap::new(),
            default_types: HashMap::new(),
        }
    }

    /// Create a polymorphic type scheme with trait bounds
    pub fn poly_bounded(
        quantified: Vec<TypeVar>,
        ty: Type,
        trait_bounds: HashMap<String, Vec<String>>,
    ) -> Self {
        TypeScheme {
            quantified,
            ty,
            trait_bounds,
            default_types: HashMap::new(),
        }
    }

    /// Create a polymorphic type scheme with trait bounds and default types
    pub fn poly_bounded_with_defaults(
        quantified: Vec<TypeVar>,
        ty: Type,
        trait_bounds: HashMap<String, Vec<String>>,
        default_types: HashMap<String, Type>,
    ) -> Self {
        TypeScheme {
            quantified,
            ty,
            trait_bounds,
            default_types,
        }
    }

    /// Instantiate a type scheme with fresh type variables.
    /// Returns the instantiated type, any ImplementsTrait constraints that need
    /// to be emitted for the fresh variables, and fallback default substitutions.
    pub fn instantiate_with_bounds(&self) -> (Type, Vec<(Type, Type)>, HashMap<TypeVar, Type>) {
        if self.quantified.is_empty() {
            return (self.ty.clone(), Vec::new(), HashMap::new());
        }

        let mut subst = HashMap::new();
        let mut constraints = Vec::new();
        let mut defaults = HashMap::new();

        for var in &self.quantified {
            let fresh = TypeVar::fresh();
            subst.insert(var.clone(), Type::Variable(fresh.clone()));

            // Emit ImplementsTrait constraints for trait bounds
            if let Some(bounds) = self.trait_bounds.get(&var.0) {
                for trait_name in bounds {
                    let bound_var = TypeVar::fresh();
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
            if let Some(default_ty) = self.default_types.get(&var.0) {
                defaults.insert(fresh_var.clone(), substitute(default_ty, &subst));
            }
        }

        (substitute(&self.ty, &subst), constraints, defaults)
    }

    /// Instantiate a type scheme with fresh type variables
    pub fn instantiate(&self) -> Type {
        if self.quantified.is_empty() {
            return self.ty.clone();
        }

        let mut subst = HashMap::new();
        for var in &self.quantified {
            subst.insert(var.clone(), Type::Variable(TypeVar::fresh()));
        }

        substitute(&self.ty, &subst)
    }

    /// Check if this scheme is polymorphic (has quantified type variables)
    pub fn is_polymorphic(&self) -> bool {
        !self.quantified.is_empty()
    }

    /// Get the quantified type variables
    pub fn type_params(&self) -> &[TypeVar] {
        &self.quantified
    }
}

/// Substitute type variables in a type
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
        Type::Function { params, returns } => Type::Function {
            params: params.iter().map(|p| substitute(p, subst)).collect(),
            returns: Box::new(substitute(returns, subst)),
        },
        Type::Concrete(_) => ty.clone(),
    }
}

impl Type {
    /// Convert Type back to TypeAnnotation for AST
    pub fn to_annotation(&self) -> Option<TypeAnnotation> {
        match self {
            Type::Concrete(ann) => Some(ann.clone()),
            Type::Variable(_) => None, // Cannot convert unresolved type var
            Type::Generic { base, args } => {
                // Try to reconstruct generic type annotation
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    let arg_annotations: Option<Vec<_>> =
                        args.iter().map(|arg| arg.to_annotation()).collect();

                    arg_annotations.map(|args| TypeAnnotation::Generic {
                        name: name.clone(),
                        args,
                    })
                } else {
                    None
                }
            }
            Type::Constrained { .. } => None, // Cannot convert constrained type
            Type::Function { params, returns } => {
                let param_anns: Vec<_> = params
                    .iter()
                    .map(|p| shape_ast::ast::FunctionParam {
                        name: None,
                        optional: false,
                        type_annotation: p.to_annotation().unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                    })
                    .collect();
                let ret_ann = returns.to_annotation().unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()));
                Some(TypeAnnotation::Function {
                    params: param_anns,
                    returns: Box::new(ret_ann),
                })
            }
        }
    }

    /// Convert inference Type to SemanticType
    ///
    /// This bridges the inference engine (Type) with the user-facing type system (SemanticType).
    /// Returns None for unresolved type variables.
    pub fn to_semantic(&self) -> Option<SemanticType> {
        match self {
            Type::Concrete(ann) => Some(super::annotations::annotation_to_semantic(ann)),
            Type::Variable(var) => {
                // Extract numeric ID from type variable name (e.g., "T42" -> 42)
                let id_str = var.0.trim_start_matches('T');
                let id = id_str.parse::<u32>().unwrap_or(0);
                Some(SemanticType::TypeVar(TypeVarId(id)))
            }
            Type::Generic { base, args } => {
                // Handle known generic types
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    let semantic_args: Vec<_> =
                        args.iter().filter_map(|arg| arg.to_semantic()).collect();

                    match name.as_str() {
                        "Option" if semantic_args.len() == 1 => {
                            Some(SemanticType::Option(Box::new(semantic_args[0].clone())))
                        }
                        "Result" if !semantic_args.is_empty() => Some(SemanticType::Result {
                            ok_type: Box::new(semantic_args[0].clone()),
                            err_type: semantic_args.get(1).cloned().map(Box::new),
                        }),
                        "Vec" if semantic_args.len() == 1 => {
                            Some(SemanticType::Array(Box::new(semantic_args[0].clone())))
                        }
                        _ => Some(SemanticType::Generic {
                            name: name.clone(),
                            args: semantic_args,
                        }),
                    }
                } else {
                    None
                }
            }
            Type::Constrained { var, .. } => {
                // For constrained types, return the variable
                let id_str = var.0.trim_start_matches('T');
                let id = id_str.parse::<u32>().unwrap_or(0);
                Some(SemanticType::TypeVar(TypeVarId(id)))
            }
            Type::Function { params, returns } => {
                let semantic_params: Vec<_> = params
                    .iter()
                    .filter_map(|p| {
                        p.to_semantic()
                            .map(|st| crate::type_system::semantic::FunctionParam {
                                name: None,
                                param_type: st,
                                optional: false,
                            })
                    })
                    .collect();
                let return_type = returns.to_semantic().unwrap_or(SemanticType::Void);
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
    /// Convert SemanticType to inference Type
    ///
    /// This allows using semantic types in the inference engine.
    pub fn to_inference_type(&self) -> Type {
        match self {
            SemanticType::Number => Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            SemanticType::Integer => Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            SemanticType::Bool => Type::Concrete(TypeAnnotation::Basic("bool".to_string())),
            SemanticType::String => Type::Concrete(TypeAnnotation::Basic("string".to_string())),
            SemanticType::Option(inner) => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                    "Option".to_string(),
                ))),
                args: vec![inner.to_inference_type()],
            },
            SemanticType::Result { ok_type, err_type } => {
                let mut args = vec![ok_type.to_inference_type()];
                if let Some(err) = err_type {
                    args.push(err.to_inference_type());
                }
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                        "Result".to_string(),
                    ))),
                    args,
                }
            }
            SemanticType::Array(elem) => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".to_string()))),
                args: vec![elem.to_inference_type()],
            },
            SemanticType::TypeVar(id) => Type::Variable(TypeVar(format!("T{}", id.0))),
            SemanticType::Named(name) => {
                if BuiltinTypes::is_number_type_name(name)
                    || BuiltinTypes::is_integer_type_name(name)
                    || BuiltinTypes::is_bool_type_name(name)
                    || BuiltinTypes::is_string_type_name(name)
                {
                    Type::Concrete(TypeAnnotation::Basic(name.clone()))
                } else {
                    Type::Concrete(TypeAnnotation::Reference(name.clone()))
                }
            }
            SemanticType::Generic { name, args } => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
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
                    Type::Concrete(TypeAnnotation::Reference(name.clone()))
                }
            }
            SemanticType::Enum { name, .. } => {
                Type::Concrete(TypeAnnotation::Reference(name.clone()))
            }
            SemanticType::Interface { name, .. } => {
                Type::Concrete(TypeAnnotation::Reference(name.clone()))
            }
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
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".to_string(),
            ))),
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
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
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
