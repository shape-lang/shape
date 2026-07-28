//! Structural projection of actual-argument callable sidecars onto a generic
//! scheme's fresh declared-variable occurrences.

use shape_ast::ast::TypeAnnotation;

use super::{SemanticTypeCandidate, SemanticTypePathSegment};
use crate::type_system::{Type, TypeVar, annotation_as_tyvar};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SemanticProjectionIssue {
    Unavailable(String),
    Conflict(String),
}

pub(super) fn project_declared_argument_candidates(
    instantiated_parameter: &Type,
    actual: &SemanticTypeCandidate,
    instantiated: &TypeVar,
) -> Result<Vec<SemanticTypeCandidate>, SemanticProjectionIssue> {
    let mut projected = Vec::new();
    project_type(
        instantiated_parameter,
        actual.ty(),
        actual,
        instantiated,
        &mut Vec::new(),
        &mut projected,
    )?;
    Ok(projected)
}

fn project_type(
    pattern: &Type,
    actual: &Type,
    candidate: &SemanticTypeCandidate,
    target: &TypeVar,
    path: &mut Vec<SemanticTypePathSegment>,
    projected: &mut Vec<SemanticTypeCandidate>,
) -> Result<(), SemanticProjectionIssue> {
    match pattern {
        Type::Variable(variable) if variable == target => {
            projected.push(
                candidate
                    .subtree(actual.clone(), path)
                    .map_err(SemanticProjectionIssue::Unavailable)?,
            );
            Ok(())
        }
        Type::Constrained { var, .. } if var == target => {
            Err(SemanticProjectionIssue::Unavailable(
                "declared semantic argument appears through a constrained carrier".to_string(),
            ))
        }
        Type::Variable(_) | Type::Constrained { .. } => Ok(()),
        Type::Concrete(pattern) => match actual {
            Type::Concrete(actual) => {
                project_annotation(pattern, actual, candidate, target, path, projected)
            }
            _ if annotation_mentions(pattern, target) => Err(SemanticProjectionIssue::Conflict(
                format!("generic argument annotation shape disagrees with actual type at {path:?}"),
            )),
            _ => Ok(()),
        },
        Type::Generic {
            base: pattern_base,
            args: pattern_args,
        } => match actual {
            Type::Generic {
                base: actual_base,
                args: actual_args,
            } if pattern_args.len() == actual_args.len() => {
                path.push(SemanticTypePathSegment::GenericBase);
                project_type(
                    pattern_base,
                    actual_base,
                    candidate,
                    target,
                    path,
                    projected,
                )?;
                path.pop();
                for (index, (pattern_arg, actual_arg)) in
                    pattern_args.iter().zip(actual_args).enumerate()
                {
                    path.push(SemanticTypePathSegment::GenericArgument(index_u16(index)?));
                    project_type(pattern_arg, actual_arg, candidate, target, path, projected)?;
                    path.pop();
                }
                Ok(())
            }
            _ if type_mentions(pattern, target) => Err(SemanticProjectionIssue::Conflict(format!(
                "generic argument container shape disagrees with actual type at {path:?}"
            ))),
            _ => Ok(()),
        },
        Type::Function {
            params: pattern_params,
            returns: pattern_returns,
            ..
        } => match actual {
            Type::Function {
                params: actual_params,
                returns: actual_returns,
                ..
            } if pattern_params.len() == actual_params.len() => {
                for (index, (pattern_param, actual_param)) in
                    pattern_params.iter().zip(actual_params).enumerate()
                {
                    path.push(SemanticTypePathSegment::CallableParameter(index_u16(
                        index,
                    )?));
                    project_type(
                        pattern_param,
                        actual_param,
                        candidate,
                        target,
                        path,
                        projected,
                    )?;
                    path.pop();
                }
                path.push(SemanticTypePathSegment::CallableReturn);
                project_type(
                    pattern_returns,
                    actual_returns,
                    candidate,
                    target,
                    path,
                    projected,
                )?;
                path.pop();
                Ok(())
            }
            _ if type_mentions(pattern, target) => Err(SemanticProjectionIssue::Conflict(format!(
                "generic argument callable shape disagrees with actual type at {path:?}"
            ))),
            _ => Ok(()),
        },
    }
}

fn project_annotation(
    pattern: &TypeAnnotation,
    actual: &TypeAnnotation,
    candidate: &SemanticTypeCandidate,
    target: &TypeVar,
    path: &mut Vec<SemanticTypePathSegment>,
    projected: &mut Vec<SemanticTypeCandidate>,
) -> Result<(), SemanticProjectionIssue> {
    if annotation_as_tyvar(pattern).as_ref() == Some(target) {
        projected.push(
            candidate
                .subtree(Type::Concrete(actual.clone()), path)
                .map_err(SemanticProjectionIssue::Unavailable)?,
        );
        return Ok(());
    }
    match (pattern, actual) {
        (TypeAnnotation::Array(pattern), TypeAnnotation::Array(actual)) => {
            path.push(SemanticTypePathSegment::ArrayElement);
            project_annotation(pattern, actual, candidate, target, path, projected)?;
            path.pop();
            Ok(())
        }
        (
            TypeAnnotation::Borrow { inner: pattern, .. },
            TypeAnnotation::Borrow { inner: actual, .. },
        ) => {
            path.push(SemanticTypePathSegment::BorrowInner);
            project_annotation(pattern, actual, candidate, target, path, projected)?;
            path.pop();
            Ok(())
        }
        (TypeAnnotation::Tuple(pattern), TypeAnnotation::Tuple(actual))
            if pattern.len() == actual.len() =>
        {
            project_annotations(
                pattern,
                actual,
                candidate,
                target,
                path,
                projected,
                SemanticTypePathSegment::TupleItem,
            )
        }
        (TypeAnnotation::Union(pattern), TypeAnnotation::Union(actual))
            if pattern.len() == actual.len() =>
        {
            project_annotations(
                pattern,
                actual,
                candidate,
                target,
                path,
                projected,
                SemanticTypePathSegment::UnionMember,
            )
        }
        (TypeAnnotation::Intersection(pattern), TypeAnnotation::Intersection(actual))
            if pattern.len() == actual.len() =>
        {
            project_annotations(
                pattern,
                actual,
                candidate,
                target,
                path,
                projected,
                SemanticTypePathSegment::IntersectionMember,
            )
        }
        (
            TypeAnnotation::Generic {
                args: pattern_args, ..
            },
            TypeAnnotation::Generic {
                args: actual_args, ..
            },
        ) if pattern_args.len() == actual_args.len() => project_annotations(
            pattern_args,
            actual_args,
            candidate,
            target,
            path,
            projected,
            SemanticTypePathSegment::GenericArgument,
        ),
        (TypeAnnotation::Object(pattern), TypeAnnotation::Object(actual))
            if pattern.len() == actual.len()
                && pattern
                    .iter()
                    .zip(actual)
                    .all(|(left, right)| left.name == right.name) =>
        {
            for (index, (pattern, actual)) in pattern.iter().zip(actual).enumerate() {
                path.push(SemanticTypePathSegment::ObjectField(index_u16(index)?));
                project_annotation(
                    &pattern.type_annotation,
                    &actual.type_annotation,
                    candidate,
                    target,
                    path,
                    projected,
                )?;
                path.pop();
            }
            Ok(())
        }
        (
            TypeAnnotation::Function {
                params: pattern_params,
                returns: pattern_returns,
            },
            TypeAnnotation::Function {
                params: actual_params,
                returns: actual_returns,
            },
        ) if pattern_params.len() == actual_params.len() => {
            for (index, (pattern, actual)) in pattern_params.iter().zip(actual_params).enumerate() {
                path.push(SemanticTypePathSegment::CallableParameter(index_u16(
                    index,
                )?));
                project_annotation(
                    &pattern.type_annotation,
                    &actual.type_annotation,
                    candidate,
                    target,
                    path,
                    projected,
                )?;
                path.pop();
            }
            path.push(SemanticTypePathSegment::CallableReturn);
            project_annotation(
                pattern_returns,
                actual_returns,
                candidate,
                target,
                path,
                projected,
            )?;
            path.pop();
            Ok(())
        }
        (
            TypeAnnotation::Existential { inner: pattern, .. },
            TypeAnnotation::Existential { inner: actual, .. },
        ) => {
            path.push(SemanticTypePathSegment::ExistentialInner);
            project_annotation(pattern, actual, candidate, target, path, projected)?;
            path.pop();
            Ok(())
        }
        _ if annotation_mentions(pattern, target) => Err(SemanticProjectionIssue::Conflict(
            format!("generic argument annotation structure disagrees at {path:?}"),
        )),
        _ => Ok(()),
    }
}

fn project_annotations(
    patterns: &[TypeAnnotation],
    actuals: &[TypeAnnotation],
    candidate: &SemanticTypeCandidate,
    target: &TypeVar,
    path: &mut Vec<SemanticTypePathSegment>,
    projected: &mut Vec<SemanticTypeCandidate>,
    segment: fn(u16) -> SemanticTypePathSegment,
) -> Result<(), SemanticProjectionIssue> {
    for (index, (pattern, actual)) in patterns.iter().zip(actuals).enumerate() {
        path.push(segment(index_u16(index)?));
        project_annotation(pattern, actual, candidate, target, path, projected)?;
        path.pop();
    }
    Ok(())
}

fn type_mentions(ty: &Type, target: &TypeVar) -> bool {
    match ty {
        Type::Variable(variable) | Type::Constrained { var: variable, .. } => variable == target,
        Type::Concrete(annotation) => annotation_mentions(annotation, target),
        Type::Generic { base, args } => {
            type_mentions(base, target) || args.iter().any(|arg| type_mentions(arg, target))
        }
        Type::Function {
            params, returns, ..
        } => {
            params.iter().any(|param| type_mentions(param, target))
                || type_mentions(returns, target)
        }
    }
}

fn annotation_mentions(annotation: &TypeAnnotation, target: &TypeVar) -> bool {
    if annotation_as_tyvar(annotation).as_ref() == Some(target) {
        return true;
    }
    match annotation {
        TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
            annotation_mentions(inner, target)
        }
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => {
            items.iter().any(|item| annotation_mentions(item, target))
        }
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|field| annotation_mentions(&field.type_annotation, target)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|param| annotation_mentions(&param.type_annotation, target))
                || annotation_mentions(returns, target)
        }
        TypeAnnotation::Generic { args, .. } => {
            args.iter().any(|arg| annotation_mentions(arg, target))
        }
        TypeAnnotation::Existential { inner, .. } => annotation_mentions(inner, target),
        TypeAnnotation::Basic(_)
        | TypeAnnotation::Reference(_)
        | TypeAnnotation::Dyn(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => false,
    }
}

fn index_u16(index: usize) -> Result<u16, SemanticProjectionIssue> {
    u16::try_from(index).map_err(|_| {
        SemanticProjectionIssue::Unavailable(format!(
            "semantic projection index {index} exceeds u16"
        ))
    })
}

#[cfg(test)]
mod tests;
