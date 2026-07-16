//! Read-only semantic-freeze projections for compiler-owned queries.

use super::*;
use shape_runtime::type_system::{
    RecursiveCallableShape, SemanticPassingMode, SemanticTypeCandidate, SemanticTypePathSegment,
    Type, TypeVar, annotation_as_tyvar,
};

mod presentation;
use presentation::{canonical_type_presentation, format_identity};

/// Result of the semantic freeze's single type canonicalizer.
///
/// `presentation` is a deterministic human-readable semantic spelling issued
/// by the freeze and is diagnostic-only:
/// identity and ordering belong to the content-derived frozen identity plus
/// its exhaustive category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenSemanticTypeProjection {
    identity: FrozenTypeIdentity,
    category: FrozenTypeCategory,
    presentation: String,
}

impl FrozenSemanticTypeProjection {
    pub(crate) fn identity(&self) -> FrozenTypeIdentity {
        self.identity
    }

    pub(crate) fn category(&self) -> FrozenTypeCategory {
        self.category
    }

    pub(crate) fn presentation(&self) -> &str {
        &self.presentation
    }
}

impl FreezeOverlay {
    /// Canonicalize through the one ADR-009 semantic authority and return only
    /// its frozen identity. Composite payload evidence is memoized by
    /// [`Self::canonicalize_type_projection`].
    pub(crate) fn canonicalize_type(
        &self,
        annotation: &TypeAnnotation,
    ) -> std::result::Result<FrozenTypeIdentity, String> {
        self.canonicalize_type_projection(annotation)
            .map(|projection| projection.identity())
    }

    /// Run the one semantic canonicalizer and return its immutable compiler
    /// projection while interning the same composite payload evidence used by
    /// `category_of`/`payload_of`. No inference `Type`, registry ordinal, or
    /// independently rendered identity crosses this boundary.
    pub(crate) fn canonicalize_type_projection(
        &self,
        annotation: &TypeAnnotation,
    ) -> std::result::Result<FrozenSemanticTypeProjection, String> {
        let canonical = canonicalize_type_annotation(annotation, self)?;
        let entry = CompositeMemoEntry {
            category: canonical.category,
            callable: canonical.callable.clone(),
            applied_nominal: super::super::type_reflection::canonical_refine(&canonical.descriptor),
            tuple: canonical.tuple.clone(),
            record: canonical.record.clone(),
            reference: canonical.reference,
            union: canonical.union.clone(),
        };
        {
            let mut composites = self
                .composites
                .lock()
                .expect("freeze-overlay composite memo lock poisoned");
            if let Some(previous) = composites.insert(canonical.identity, entry) {
                assert_eq!(
                    previous.category, canonical.category,
                    "canonical type identity collision across semantic categories"
                );
            }
        }
        Ok(FrozenSemanticTypeProjection {
            identity: canonical.identity,
            category: canonical.category,
            // Presentation is diagnostic-only and must not gate the
            // authoritative identity/category: the renderer re-canonicalizes
            // sub-annotations context-free and cannot yet spell trait-context
            // members or existential witnesses, so fall back to the
            // already-computed canonical identity rather than failing.
            presentation: canonical_type_presentation(annotation, self)
                .unwrap_or_else(|_| format_identity(canonical.identity)),
        })
    }

    /// Convert an inference-engine [`Type`] into the annotation consumed by
    /// the one semantic-freeze canonicalizer.
    ///
    /// A raw `Type::Variable` carries no proof that it denotes a declared
    /// parameter rather than a same-spelled solver hole, so this entry point
    /// rejects every variable. Declared parameters are admitted only through
    /// the provenance-bearing projection seam; constrained variables remain
    /// partial inference state. Neither is fabricated as `unknown`.
    #[cfg(test)]
    pub(crate) fn inference_type_annotation(
        &self,
        ty: &Type,
    ) -> std::result::Result<TypeAnnotation, String> {
        match ty {
            Type::Concrete(annotation) => Ok(annotation.clone()),
            Type::Variable(variable) => self.semantic_variable_annotation(variable),
            Type::Constrained { var, .. } => Err(format!(
                "capture semantic type contains constrained inference variable '{}' and cannot be frozen",
                var.presentation_name()
            )),
            Type::Generic { base, args } => {
                let base = self.inference_type_annotation(&base)?;
                let args = args
                    .iter()
                    .map(|arg| self.inference_type_annotation(arg))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let TypeAnnotation::Reference(name) = base else {
                    return Err(
                        "capture semantic generic type has no resolved nominal head and cannot be frozen"
                            .to_string(),
                    );
                };
                if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 {
                    Ok(TypeAnnotation::Array(Box::new(
                        args.into_iter().next().expect("checked one generic argument"),
                    )))
                } else {
                    Ok(TypeAnnotation::Generic { name, args })
                }
            }
            Type::Function { .. } => Err(
                "capture semantic type contains an inference-level callable without exact parameter optionality and passing-mode evidence"
                    .to_string(),
            ),
        }
    }

    /// Project a provenance-bearing inference candidate through the semantic
    /// freeze input boundary. `Type` supplies every leaf; the recursive
    /// sidecar supplies only callable optionality and passing modes.
    pub(crate) fn semantic_candidate_annotation(
        &self,
        candidate: &SemanticTypeCandidate,
    ) -> std::result::Result<TypeAnnotation, String> {
        self.semantic_type_annotation_at(
            candidate.ty(),
            candidate.recursive_callable_shape(),
            &mut Vec::new(),
        )
    }

    /// Resolve and freeze one exact inference candidate while its enclosing
    /// specialization evidence is still available.
    ///
    /// A nested specialization stores this sealed value, never the raw
    /// candidate. Consequently an inner body remains semantically closed
    /// after the outer overlay frame has been removed.
    pub(crate) fn close_semantic_candidate(
        &self,
        candidate: &SemanticTypeCandidate,
    ) -> std::result::Result<ClosedSemanticType, String> {
        let annotation = self.semantic_candidate_annotation(candidate)?;
        if annotation_contains_reserved_type_var_carrier(&annotation) {
            return Err(
                "semantic candidate retained a reserved type-variable carrier after projection and cannot be closed"
                    .to_string(),
            );
        }
        let projection = self.canonicalize_type_projection(&annotation)?;
        Ok(ClosedSemanticType::new(annotation, projection))
    }

    fn semantic_type_annotation_at(
        &self,
        ty: &Type,
        shape: &RecursiveCallableShape,
        path: &mut Vec<SemanticTypePathSegment>,
    ) -> std::result::Result<TypeAnnotation, String> {
        match ty {
            Type::Concrete(annotation) => self.semantic_annotation_at(annotation, shape, path),
            Type::Variable(variable) => self.semantic_variable_annotation(variable),
            Type::Constrained { var, .. } => Err(format!(
                "semantic candidate contains constrained inference variable '{}' and cannot be frozen",
                var.presentation_name()
            )),
            Type::Generic { base, args } => {
                path.push(SemanticTypePathSegment::GenericBase);
                let base = self.semantic_type_annotation_at(base, shape, path)?;
                path.pop();
                let mut projected_args = Vec::with_capacity(args.len());
                for (index, arg) in args.iter().enumerate() {
                    path.push(SemanticTypePathSegment::GenericArgument(path_index(index)?));
                    projected_args.push(self.semantic_type_annotation_at(arg, shape, path)?);
                    path.pop();
                }
                let TypeAnnotation::Reference(name) = base else {
                    return Err(
                        "semantic generic candidate has no resolved nominal head".to_string()
                    );
                };
                if (name.as_str() == "Array" || name.as_str() == "Vec") && projected_args.len() == 1
                {
                    Ok(TypeAnnotation::Array(Box::new(
                        projected_args
                            .into_iter()
                            .next()
                            .expect("checked one generic argument"),
                    )))
                } else {
                    Ok(TypeAnnotation::Generic {
                        name,
                        args: projected_args,
                    })
                }
            }
            Type::Function { params, returns } => {
                self.semantic_callable_annotation(params, returns, shape, path)
            }
        }
    }

    fn semantic_annotation_at(
        &self,
        annotation: &TypeAnnotation,
        shape: &RecursiveCallableShape,
        path: &mut Vec<SemanticTypePathSegment>,
    ) -> std::result::Result<TypeAnnotation, String> {
        if let Some(variable) = annotation_as_tyvar(annotation) {
            return self.semantic_variable_annotation(&variable);
        }
        match annotation {
            TypeAnnotation::Function { params, returns } => {
                let parameter_types: Vec<Type> = params
                    .iter()
                    .map(|parameter| Type::Concrete(parameter.type_annotation.clone()))
                    .collect();
                self.semantic_callable_annotation(
                    &parameter_types,
                    &Type::Concrete((**returns).clone()),
                    shape,
                    path,
                )
            }
            TypeAnnotation::Array(inner) => {
                path.push(SemanticTypePathSegment::ArrayElement);
                let inner = self.semantic_annotation_at(inner, shape, path)?;
                path.pop();
                Ok(TypeAnnotation::Array(Box::new(inner)))
            }
            TypeAnnotation::Tuple(items) => {
                Ok(TypeAnnotation::Tuple(self.semantic_annotations_at(
                    items,
                    shape,
                    path,
                    SemanticTypePathSegment::TupleItem,
                )?))
            }
            TypeAnnotation::Object(fields) => {
                let mut projected = Vec::with_capacity(fields.len());
                for (index, field) in fields.iter().enumerate() {
                    path.push(SemanticTypePathSegment::ObjectField(path_index(index)?));
                    let mut field = field.clone();
                    field.type_annotation =
                        self.semantic_annotation_at(&field.type_annotation, shape, path)?;
                    path.pop();
                    projected.push(field);
                }
                Ok(TypeAnnotation::Object(projected))
            }
            TypeAnnotation::Union(items) => {
                Ok(TypeAnnotation::Union(self.semantic_annotations_at(
                    items,
                    shape,
                    path,
                    SemanticTypePathSegment::UnionMember,
                )?))
            }
            TypeAnnotation::Intersection(items) => {
                Ok(TypeAnnotation::Intersection(self.semantic_annotations_at(
                    items,
                    shape,
                    path,
                    SemanticTypePathSegment::IntersectionMember,
                )?))
            }
            TypeAnnotation::Borrow { mutable, inner } => {
                path.push(SemanticTypePathSegment::BorrowInner);
                let inner = self.semantic_annotation_at(inner, shape, path)?;
                path.pop();
                Ok(TypeAnnotation::Borrow {
                    mutable: *mutable,
                    inner: Box::new(inner),
                })
            }
            TypeAnnotation::Generic { name, args } => Ok(TypeAnnotation::Generic {
                name: name.clone(),
                args: self.semantic_annotations_at(
                    args,
                    shape,
                    path,
                    SemanticTypePathSegment::GenericArgument,
                )?,
            }),
            TypeAnnotation::Existential { witnesses, inner } => {
                path.push(SemanticTypePathSegment::ExistentialInner);
                let inner = self.semantic_annotation_at(inner, shape, path)?;
                path.pop();
                Ok(TypeAnnotation::Existential {
                    witnesses: witnesses.clone(),
                    inner: Box::new(inner),
                })
            }
            TypeAnnotation::Basic(_)
            | TypeAnnotation::Reference(_)
            | TypeAnnotation::Dyn(_)
            | TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => Ok(annotation.clone()),
        }
    }

    fn semantic_annotations_at(
        &self,
        annotations: &[TypeAnnotation],
        shape: &RecursiveCallableShape,
        path: &mut Vec<SemanticTypePathSegment>,
        segment: fn(u16) -> SemanticTypePathSegment,
    ) -> std::result::Result<Vec<TypeAnnotation>, String> {
        let mut projected = Vec::with_capacity(annotations.len());
        for (index, annotation) in annotations.iter().enumerate() {
            path.push(segment(path_index(index)?));
            projected.push(self.semantic_annotation_at(annotation, shape, path)?);
            path.pop();
        }
        Ok(projected)
    }

    fn semantic_callable_annotation(
        &self,
        params: &[Type],
        returns: &Type,
        shape: &RecursiveCallableShape,
        path: &mut Vec<SemanticTypePathSegment>,
    ) -> std::result::Result<TypeAnnotation, String> {
        let node = shape.callable_at(path).ok_or_else(|| {
            format!("semantic callable at {path:?} has no recursive shape evidence")
        })?;
        if node.parameters().len() != params.len() {
            return Err(format!(
                "semantic callable arity mismatch at {path:?}: type={}, shape={}",
                params.len(),
                node.parameters().len()
            ));
        }
        let mut projected_params = Vec::with_capacity(params.len());
        for (index, (parameter, parameter_shape)) in
            params.iter().zip(node.parameters()).enumerate()
        {
            path.push(SemanticTypePathSegment::CallableParameter(path_index(
                index,
            )?));
            let annotation = self.semantic_type_annotation_at(parameter, shape, path)?;
            path.pop();
            projected_params.push(shape_ast::ast::FunctionParam {
                name: None,
                optional: parameter_shape.optional(),
                type_annotation: apply_semantic_passing_mode(
                    annotation,
                    parameter_shape.passing_mode(),
                    path,
                    index,
                )?,
            });
        }
        path.push(SemanticTypePathSegment::CallableReturn);
        let returns = self.semantic_type_annotation_at(returns, shape, path)?;
        path.pop();
        Ok(TypeAnnotation::Function {
            params: projected_params,
            returns: Box::new(returns),
        })
    }

    fn semantic_variable_annotation(
        &self,
        variable: &TypeVar,
    ) -> std::result::Result<TypeAnnotation, String> {
        let Some(provenance) = variable.declared_provenance() else {
            return Err(format!(
                "semantic type contains provenance-free inference variable '{}' and cannot be frozen",
                variable.presentation_name()
            ));
        };
        let exact = self.exact_semantic_argument(variable).ok_or_else(|| {
            format!(
                "declared type parameter '{}' has no exact semantic specialization evidence",
                provenance.source_name()
            )
        })?;
        Ok(exact.annotation().clone())
    }
}

fn apply_semantic_passing_mode(
    annotation: TypeAnnotation,
    mode: SemanticPassingMode,
    path: &[SemanticTypePathSegment],
    index: usize,
) -> std::result::Result<TypeAnnotation, String> {
    let expected_mutability = match mode {
        SemanticPassingMode::ByValue => None,
        SemanticPassingMode::SharedBorrow => Some(false),
        SemanticPassingMode::ExclusiveBorrow => Some(true),
    };
    match (annotation, expected_mutability) {
        (TypeAnnotation::Borrow { .. }, None) => Err(format!(
            "semantic callable parameter {index} at {path:?} is borrowed but shape says by-value"
        )),
        (TypeAnnotation::Borrow { mutable, inner }, Some(expected)) if mutable == expected => {
            Ok(TypeAnnotation::Borrow { mutable, inner })
        }
        (TypeAnnotation::Borrow { mutable, .. }, Some(expected)) => Err(format!(
            "semantic callable parameter {index} at {path:?} borrow mutability {mutable} disagrees with shape {expected}"
        )),
        (annotation, None) => Ok(annotation),
        (annotation, Some(mutable)) => Ok(TypeAnnotation::Borrow {
            mutable,
            inner: Box::new(annotation),
        }),
    }
}

fn path_index(index: usize) -> std::result::Result<u16, String> {
    u16::try_from(index).map_err(|_| format!("semantic type path index {index} exceeds u16"))
}

/// Detect the literal `unknown` sentinel fabricated by `Type::to_annotation`
/// when any nested inference component remains unresolved.
///
/// This is deliberately separate from the semantic freeze's canonical tyvar
/// predicate: authored `type_ref(unknown)` retains its existing unknown-name
/// diagnostic, while lossy inference conversion is rejected before it can be
/// mistaken for exact capture evidence.
pub(crate) fn annotation_has_lossy_unknown_sentinel(annotation: &TypeAnnotation) -> bool {
    match annotation {
        TypeAnnotation::Basic(name) => name == "unknown",
        TypeAnnotation::Reference(path) => path.as_str() == "unknown",
        TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
            annotation_has_lossy_unknown_sentinel(inner)
        }
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => {
            items.iter().any(annotation_has_lossy_unknown_sentinel)
        }
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|field| annotation_has_lossy_unknown_sentinel(&field.type_annotation)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|param| annotation_has_lossy_unknown_sentinel(&param.type_annotation))
                || annotation_has_lossy_unknown_sentinel(returns)
        }
        TypeAnnotation::Generic { name, args } => {
            name.as_str() == "unknown" || args.iter().any(annotation_has_lossy_unknown_sentinel)
        }
        TypeAnnotation::Dyn(paths) => paths.iter().any(|path| path.as_str() == "unknown"),
        TypeAnnotation::Existential { inner, .. } => annotation_has_lossy_unknown_sentinel(inner),
        TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => false,
    }
}

#[cfg(test)]
mod tests;
