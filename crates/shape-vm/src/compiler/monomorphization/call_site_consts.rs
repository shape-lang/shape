//! Static resolution for explicit call-site const generic arguments.

use super::type_resolution::{ComptimeConstValue, comptime_const_value_from_literal_expr};
use shape_ast::ast::{Expr, Span, Spanned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallSiteConstArgError {
    pub(crate) span: Span,
    pub(crate) message: String,
}

pub(crate) fn no_const_params_error(func_name: &str, span: Span) -> CallSiteConstArgError {
    CallSiteConstArgError {
        span,
        message: format!("'{}' does not declare const generic parameters", func_name),
    }
}

pub(crate) fn resolve_explicit_const_args(
    func_name: &str,
    expected_const_params: usize,
    explicit_const_args: &[Expr],
    call_site_span: Span,
) -> Result<Vec<ComptimeConstValue>, CallSiteConstArgError> {
    if explicit_const_args.is_empty() {
        return Ok(Vec::new());
    }

    if explicit_const_args.len() != expected_const_params {
        return Err(CallSiteConstArgError {
            span: call_site_span,
            message: format!(
                "'{}' declares {} const generic parameters but {} const arguments were supplied",
                func_name,
                expected_const_params,
                explicit_const_args.len()
            ),
        });
    }

    explicit_const_args
        .iter()
        .map(|expr| {
            comptime_const_value_from_literal_expr(expr).ok_or_else(|| CallSiteConstArgError {
                span: expr.span(),
                message: format!(
                    "const generic arg must be a compile-time constant: '{}' call-site const arguments currently support literals only",
                    func_name
                ),
            })
        })
        .collect()
}
