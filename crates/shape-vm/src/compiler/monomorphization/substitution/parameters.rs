//! Type substitution for function parameters.

use super::{substitute_destructure_pattern, substitute_expr, substitute_type_annotation};
use shape_ast::ast::functions::FunctionParameter;
use shape_value::v2::ConcreteType;
use std::collections::HashMap;

pub(super) fn substitute_function_parameter(
    p: &FunctionParameter,
    subs: &HashMap<String, ConcreteType>,
) -> FunctionParameter {
    FunctionParameter {
        pattern: substitute_destructure_pattern(&p.pattern, subs),
        is_const: p.is_const,
        is_reference: p.is_reference,
        is_mut_reference: p.is_mut_reference,
        is_out: p.is_out,
        type_annotation: p
            .type_annotation
            .as_ref()
            .map(|t| substitute_type_annotation(t, subs)),
        default_value: p.default_value.as_ref().map(|e| substitute_expr(e, subs)),
    }
}
