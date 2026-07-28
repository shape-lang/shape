//! Completeness checks for post-solve semantic inference types.

use shape_ast::ast::TypeAnnotation;

use crate::type_system::{Type, annotation_as_tyvar};

pub(in crate::type_system::inference::generated_callable_facts) fn type_is_semantically_resolved(
    ty: &Type,
    allow_declared: bool,
) -> bool {
    match ty {
        Type::Variable(variable) => allow_declared && variable.declared_provenance().is_some(),
        Type::Constrained { .. } => false,
        Type::Concrete(annotation) => {
            annotation_is_semantically_resolved(annotation, allow_declared)
        }
        Type::Generic { base, args } => {
            type_is_semantically_resolved(base, allow_declared)
                && args
                    .iter()
                    .all(|arg| type_is_semantically_resolved(arg, allow_declared))
        }
        Type::Function {
            params, returns, ..
        } => {
            params
                .iter()
                .all(|param| type_is_semantically_resolved(param, allow_declared))
                && type_is_semantically_resolved(returns, allow_declared)
        }
    }
}

fn annotation_is_semantically_resolved(annotation: &TypeAnnotation, allow_declared: bool) -> bool {
    if let Some(variable) = annotation_as_tyvar(annotation) {
        return allow_declared && variable.declared_provenance().is_some();
    }
    match annotation {
        TypeAnnotation::Array(inner) | TypeAnnotation::Borrow { inner, .. } => {
            annotation_is_semantically_resolved(inner, allow_declared)
        }
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => items
            .iter()
            .all(|item| annotation_is_semantically_resolved(item, allow_declared)),
        TypeAnnotation::Object(fields) => fields.iter().all(|field| {
            annotation_is_semantically_resolved(&field.type_annotation, allow_declared)
        }),
        TypeAnnotation::Function { params, returns } => {
            params.iter().all(|param| {
                annotation_is_semantically_resolved(&param.type_annotation, allow_declared)
            }) && annotation_is_semantically_resolved(returns, allow_declared)
        }
        TypeAnnotation::Generic { args, .. } => args
            .iter()
            .all(|arg| annotation_is_semantically_resolved(arg, allow_declared)),
        TypeAnnotation::Existential { inner, .. } => {
            annotation_is_semantically_resolved(inner, allow_declared)
        }
        TypeAnnotation::Basic(_)
        | TypeAnnotation::Reference(_)
        | TypeAnnotation::Dyn(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => true,
    }
}

#[cfg(test)]
mod tests;
