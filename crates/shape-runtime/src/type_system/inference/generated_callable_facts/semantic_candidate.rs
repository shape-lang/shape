//! Recursive callable-shape evidence paired with inference types.
//!
//! `Type` remains the sole authority for every leaf type. This sidecar records
//! only information erased by `Type::Function`: parameter optionality and
//! semantic passing mode at every callable node.

use std::collections::{BTreeMap, BTreeSet};

use shape_ast::ast::{FunctionParameter, TypeAnnotation};

use crate::type_system::Type;

mod passing_mode;
mod resolution;
#[cfg(test)]
mod tests;
pub use passing_mode::{SemanticCallableParameterShape, SemanticPassingMode};
pub(super) use resolution::type_is_semantically_resolved;

/// Structural route from a candidate's root type to a nested callable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticTypePathSegment {
    CallableParameter(u16),
    CallableReturn,
    GenericBase,
    GenericArgument(u16),
    ArrayElement,
    TupleItem(u16),
    ObjectField(u16),
    UnionMember(u16),
    IntersectionMember(u16),
    BorrowInner,
    ExistentialInner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCallableNodeShape {
    parameters: Vec<SemanticCallableParameterShape>,
}

impl SemanticCallableNodeShape {
    #[must_use]
    pub fn parameters(&self) -> &[SemanticCallableParameterShape] {
        &self.parameters
    }
}

/// Sparse recursive sidecar: only callable nodes occupy entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecursiveCallableShape {
    nodes: BTreeMap<Vec<SemanticTypePathSegment>, SemanticCallableNodeShape>,
}

impl RecursiveCallableShape {
    #[must_use]
    pub fn callable_at(
        &self,
        path: &[SemanticTypePathSegment],
    ) -> Option<&SemanticCallableNodeShape> {
        self.nodes.get(path)
    }

    #[must_use]
    pub fn nodes(&self) -> &BTreeMap<Vec<SemanticTypePathSegment>, SemanticCallableNodeShape> {
        &self.nodes
    }

    fn below(&self, prefix: &[SemanticTypePathSegment]) -> Self {
        let nodes = self
            .nodes
            .iter()
            .filter_map(|(path, node)| {
                path.strip_prefix(prefix)
                    .map(|suffix| (suffix.to_vec(), node.clone()))
            })
            .collect();
        Self { nodes }
    }

    fn from_generated_function(
        params: &[FunctionParameter],
        return_type: Option<&TypeAnnotation>,
    ) -> Result<Self, String> {
        let parameters = params
            .iter()
            .map(|parameter| SemanticCallableParameterShape {
                optional: parameter.default_value.is_some(),
                passing_mode: SemanticPassingMode::from_function_parameter(parameter),
            })
            .collect();
        let mut shape = Self::default();
        shape
            .nodes
            .insert(Vec::new(), SemanticCallableNodeShape { parameters });

        for (index, parameter) in params.iter().enumerate() {
            let Some(annotation) = parameter.type_annotation.as_ref() else {
                continue;
            };
            let mut path = vec![SemanticTypePathSegment::CallableParameter(
                u16::try_from(index)
                    .map_err(|_| "generated callable parameter index exceeds u16".to_string())?,
            )];
            shape.collect_annotation(annotation, &mut path)?;
        }
        if let Some(annotation) = return_type {
            let mut path = vec![SemanticTypePathSegment::CallableReturn];
            shape.collect_annotation(annotation, &mut path)?;
        }
        Ok(shape)
    }

    fn from_type_metadata(ty: &Type) -> Result<Self, String> {
        // A bare inference `Type` is not optionality/passing-mode authority.
        // In particular, `Type::Function::to_annotation` must not launder its
        // synthesized `optional = false` parameters into exact evidence.
        // Validation below accepts non-callables and rejects every callable
        // path unless an explicit syntax-bearing constructor populated it.
        let shape = Self::default();
        shape.validate(ty)?;
        Ok(shape)
    }

    fn from_annotation(annotation: &TypeAnnotation) -> Result<Self, String> {
        let mut shape = Self::default();
        shape.collect_annotation(annotation, &mut Vec::new())?;
        Ok(shape)
    }

    fn collect_annotation(
        &mut self,
        annotation: &TypeAnnotation,
        path: &mut Vec<SemanticTypePathSegment>,
    ) -> Result<(), String> {
        match annotation {
            TypeAnnotation::Function {
                params, returns, ..
            } => {
                let parameters = params
                    .iter()
                    .map(|parameter| SemanticCallableParameterShape {
                        optional: parameter.optional,
                        passing_mode: SemanticPassingMode::from_annotation(
                            &parameter.type_annotation,
                        ),
                    })
                    .collect();
                if self
                    .nodes
                    .insert(path.clone(), SemanticCallableNodeShape { parameters })
                    .is_some()
                {
                    return Err(format!(
                        "duplicate callable-shape evidence at structural path {path:?}"
                    ));
                }
                for (index, parameter) in params.iter().enumerate() {
                    path.push(SemanticTypePathSegment::CallableParameter(index_u16(
                        index,
                    )?));
                    self.collect_annotation(&parameter.type_annotation, path)?;
                    path.pop();
                }
                path.push(SemanticTypePathSegment::CallableReturn);
                self.collect_annotation(returns, path)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Array(inner) => {
                path.push(SemanticTypePathSegment::ArrayElement);
                self.collect_annotation(inner, path)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Tuple(items) => {
                self.collect_annotations(items, path, SemanticTypePathSegment::TupleItem)
            }
            TypeAnnotation::Object(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    path.push(SemanticTypePathSegment::ObjectField(index_u16(index)?));
                    self.collect_annotation(&field.type_annotation, path)?;
                    path.pop();
                }
                Ok(())
            }
            TypeAnnotation::Union(items) => {
                self.collect_annotations(items, path, SemanticTypePathSegment::UnionMember)
            }
            TypeAnnotation::Intersection(items) => {
                self.collect_annotations(items, path, SemanticTypePathSegment::IntersectionMember)
            }
            TypeAnnotation::Borrow { inner, .. } => {
                path.push(SemanticTypePathSegment::BorrowInner);
                self.collect_annotation(inner, path)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Generic { args, .. } => {
                self.collect_annotations(args, path, SemanticTypePathSegment::GenericArgument)
            }
            TypeAnnotation::Existential { inner, .. } => {
                path.push(SemanticTypePathSegment::ExistentialInner);
                self.collect_annotation(inner, path)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Basic(_)
            | TypeAnnotation::Reference(_)
            | TypeAnnotation::Dyn(_)
            | TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => Ok(()),
        }
    }

    fn collect_annotations(
        &mut self,
        annotations: &[TypeAnnotation],
        path: &mut Vec<SemanticTypePathSegment>,
        segment: fn(u16) -> SemanticTypePathSegment,
    ) -> Result<(), String> {
        for (index, annotation) in annotations.iter().enumerate() {
            path.push(segment(index_u16(index)?));
            self.collect_annotation(annotation, path)?;
            path.pop();
        }
        Ok(())
    }

    fn validate(&self, ty: &Type) -> Result<(), String> {
        let mut visited = BTreeSet::new();
        self.validate_type(ty, &mut Vec::new(), &mut visited)?;
        if visited.len() != self.nodes.len() {
            let extra = self
                .nodes
                .keys()
                .find(|path| !visited.contains(*path))
                .expect("node counts differ only when an extra path exists");
            return Err(format!(
                "callable-shape evidence has no matching inferred callable at {extra:?}"
            ));
        }
        Ok(())
    }

    fn validate_type(
        &self,
        ty: &Type,
        path: &mut Vec<SemanticTypePathSegment>,
        visited: &mut BTreeSet<Vec<SemanticTypePathSegment>>,
    ) -> Result<(), String> {
        match ty {
            Type::Concrete(annotation) => self.validate_annotation(annotation, path, visited),
            Type::Variable(_) | Type::Constrained { .. } => Ok(()),
            Type::Generic { base, args } => {
                path.push(SemanticTypePathSegment::GenericBase);
                self.validate_type(base, path, visited)?;
                path.pop();
                for (index, arg) in args.iter().enumerate() {
                    path.push(SemanticTypePathSegment::GenericArgument(index_u16(index)?));
                    self.validate_type(arg, path, visited)?;
                    path.pop();
                }
                Ok(())
            }
            Type::Function {
                params, returns, ..
            } => {
                self.validate_callable(params.len(), path, visited)?;
                for (index, parameter) in params.iter().enumerate() {
                    path.push(SemanticTypePathSegment::CallableParameter(index_u16(
                        index,
                    )?));
                    self.validate_type(parameter, path, visited)?;
                    path.pop();
                }
                path.push(SemanticTypePathSegment::CallableReturn);
                self.validate_type(returns, path, visited)?;
                path.pop();
                Ok(())
            }
        }
    }

    fn validate_annotation(
        &self,
        annotation: &TypeAnnotation,
        path: &mut Vec<SemanticTypePathSegment>,
        visited: &mut BTreeSet<Vec<SemanticTypePathSegment>>,
    ) -> Result<(), String> {
        match annotation {
            TypeAnnotation::Function {
                params, returns, ..
            } => {
                self.validate_callable(params.len(), path, visited)?;
                for (index, parameter) in params.iter().enumerate() {
                    path.push(SemanticTypePathSegment::CallableParameter(index_u16(
                        index,
                    )?));
                    self.validate_annotation(&parameter.type_annotation, path, visited)?;
                    path.pop();
                }
                path.push(SemanticTypePathSegment::CallableReturn);
                self.validate_annotation(returns, path, visited)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Array(inner) => {
                path.push(SemanticTypePathSegment::ArrayElement);
                self.validate_annotation(inner, path, visited)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Tuple(items) => {
                self.validate_annotations(items, path, visited, SemanticTypePathSegment::TupleItem)
            }
            TypeAnnotation::Object(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    path.push(SemanticTypePathSegment::ObjectField(index_u16(index)?));
                    self.validate_annotation(&field.type_annotation, path, visited)?;
                    path.pop();
                }
                Ok(())
            }
            TypeAnnotation::Union(items) => self.validate_annotations(
                items,
                path,
                visited,
                SemanticTypePathSegment::UnionMember,
            ),
            TypeAnnotation::Intersection(items) => self.validate_annotations(
                items,
                path,
                visited,
                SemanticTypePathSegment::IntersectionMember,
            ),
            TypeAnnotation::Borrow { inner, .. } => {
                path.push(SemanticTypePathSegment::BorrowInner);
                self.validate_annotation(inner, path, visited)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Generic { args, .. } => self.validate_annotations(
                args,
                path,
                visited,
                SemanticTypePathSegment::GenericArgument,
            ),
            TypeAnnotation::Existential { inner, .. } => {
                path.push(SemanticTypePathSegment::ExistentialInner);
                self.validate_annotation(inner, path, visited)?;
                path.pop();
                Ok(())
            }
            TypeAnnotation::Basic(_)
            | TypeAnnotation::Reference(_)
            | TypeAnnotation::Dyn(_)
            | TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => Ok(()),
        }
    }

    fn validate_annotations(
        &self,
        annotations: &[TypeAnnotation],
        path: &mut Vec<SemanticTypePathSegment>,
        visited: &mut BTreeSet<Vec<SemanticTypePathSegment>>,
        segment: fn(u16) -> SemanticTypePathSegment,
    ) -> Result<(), String> {
        for (index, annotation) in annotations.iter().enumerate() {
            path.push(segment(index_u16(index)?));
            self.validate_annotation(annotation, path, visited)?;
            path.pop();
        }
        Ok(())
    }

    fn validate_callable(
        &self,
        arity: usize,
        path: &[SemanticTypePathSegment],
        visited: &mut BTreeSet<Vec<SemanticTypePathSegment>>,
    ) -> Result<(), String> {
        let Some(node) = self.nodes.get(path) else {
            return Err(format!(
                "inferred callable at {path:?} has no optionality/passing-mode evidence"
            ));
        };
        if node.parameters.len() != arity {
            return Err(format!(
                "callable-shape arity mismatch at {path:?}: type={arity}, shape={}",
                node.parameters.len()
            ));
        }
        visited.insert(path.to_vec());
        Ok(())
    }
}

/// Inference type plus callable-only syntax information. The sidecar cannot
/// supply or override any leaf type.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticTypeCandidate {
    ty: Type,
    recursive_callable_shape: RecursiveCallableShape,
}

impl SemanticTypeCandidate {
    pub(super) fn generated_callable(
        ty: Type,
        params: &[FunctionParameter],
        return_type: Option<&TypeAnnotation>,
    ) -> Result<Self, String> {
        let candidate = Self {
            ty,
            recursive_callable_shape: RecursiveCallableShape::from_generated_function(
                params,
                return_type,
            )?,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub(super) fn monomorphic_binding(ty: Type) -> Result<Self, String> {
        let candidate = Self {
            recursive_callable_shape: RecursiveCallableShape::from_type_metadata(&ty)?,
            ty,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub(super) fn annotated_binding(ty: Type, annotation: &TypeAnnotation) -> Result<Self, String> {
        let candidate = Self {
            ty,
            recursive_callable_shape: RecursiveCallableShape::from_annotation(annotation)?,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub(super) fn with_resolved_type(&self, ty: Type) -> Result<Self, String> {
        let candidate = Self {
            ty,
            recursive_callable_shape: self.recursive_callable_shape.clone(),
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub(super) fn subtree(
        &self,
        ty: Type,
        path: &[SemanticTypePathSegment],
    ) -> Result<Self, String> {
        let candidate = Self {
            ty,
            recursive_callable_shape: self.recursive_callable_shape.below(path),
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        self.recursive_callable_shape.validate(&self.ty)
    }

    #[must_use]
    pub fn ty(&self) -> &Type {
        &self.ty
    }

    #[must_use]
    pub fn recursive_callable_shape(&self) -> &RecursiveCallableShape {
        &self.recursive_callable_shape
    }
}

fn index_u16(index: usize) -> Result<u16, String> {
    u16::try_from(index).map_err(|_| format!("semantic type path index {index} exceeds u16"))
}
