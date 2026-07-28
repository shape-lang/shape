//! Static specialization helpers for receiver-parametric `extend Number`.

use shape_ast::ast::{FunctionDef, TypeAnnotation};
use shape_value::v2::ConcreteType;
use std::collections::HashMap;

pub(super) fn number_receiver_generic_substitutions(
    func_name: &str,
    func_def: &FunctionDef,
    receiver_ct: &ConcreteType,
) -> Option<HashMap<String, TypeAnnotation>> {
    if !(func_name.starts_with("Number.") || func_name.starts_with("number.")) {
        return None;
    }
    let type_params = func_def.type_params.as_ref()?;
    if type_params.len() != 1 || type_params[0].name() != "N" {
        return None;
    }
    let receiver_ann =
        crate::compiler::expressions::closures::concrete_type_to_type_annotation(receiver_ct)?;
    Some(HashMap::from([("N".to_string(), receiver_ann)]))
}

pub(super) fn substitute_type_params_in_annotation(
    ann: &TypeAnnotation,
    substitutions: &HashMap<String, TypeAnnotation>,
) -> TypeAnnotation {
    match ann {
        TypeAnnotation::Basic(name) => substitutions
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| ann.clone()),
        TypeAnnotation::Reference(path) => substitutions
            .get(path.as_str())
            .cloned()
            .unwrap_or_else(|| ann.clone()),
        TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(
            substitute_type_params_in_annotation(inner, substitutions),
        )),
        TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
            items
                .iter()
                .map(|item| substitute_type_params_in_annotation(item, substitutions))
                .collect(),
        ),
        TypeAnnotation::Object(fields) => TypeAnnotation::Object(
            fields
                .iter()
                .cloned()
                .map(|mut field| {
                    field.type_annotation =
                        substitute_type_params_in_annotation(&field.type_annotation, substitutions);
                    field
                })
                .collect(),
        ),
        TypeAnnotation::Function {
            params, returns, ..
        } => TypeAnnotation::Function {
            params: params
                .iter()
                .cloned()
                .map(|mut param| {
                    param.type_annotation =
                        substitute_type_params_in_annotation(&param.type_annotation, substitutions);
                    param
                })
                .collect(),
            returns: Box::new(substitute_type_params_in_annotation(returns, substitutions)),
            effects: None,
        },
        TypeAnnotation::Union(items) => TypeAnnotation::Union(
            items
                .iter()
                .map(|item| substitute_type_params_in_annotation(item, substitutions))
                .collect(),
        ),
        TypeAnnotation::Intersection(items) => TypeAnnotation::Intersection(
            items
                .iter()
                .map(|item| substitute_type_params_in_annotation(item, substitutions))
                .collect(),
        ),
        TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_type_params_in_annotation(arg, substitutions))
                .collect(),
        },
        TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
            mutable: *mutable,
            inner: Box::new(substitute_type_params_in_annotation(inner, substitutions)),
        },
        // ADR-009 B3 (S1): existential descriptor package. Witnesses are locally
        // bound, so shadow them out of the substitution before recursing.
        TypeAnnotation::Existential { witnesses, inner } => {
            let mut inner_subs = substitutions.clone();
            for w in witnesses {
                inner_subs.remove(w);
            }
            TypeAnnotation::Existential {
                witnesses: witnesses.clone(),
                inner: Box::new(substitute_type_params_in_annotation(inner, &inner_subs)),
            }
        }
        TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined
        | TypeAnnotation::Dyn(_) => ann.clone(),
    }
}
