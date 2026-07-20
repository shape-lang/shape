//! Function-expression rebuilding for the `ctx.original` rewrite.

use std::collections::HashSet;

use shape_ast::ast::Expr;

use super::{extended, rewrite_in_expr, rewrite_statement_seq};

pub(super) fn rewrite(expr: &Expr, bound: &HashSet<String>, shadow: &str) -> Expr {
    let Expr::FunctionExpr {
        params,
        return_type,
        body,
        generated_origin,
        captures,
        annotations,
        span,
    } = expr
    else {
        unreachable!("function-expression rewrite called for another expression kind")
    };

    // Parameters bind only inside the body; their default expressions run in
    // the enclosing scope.
    let body_scope = extended(
        bound,
        params.iter().flat_map(|param| param.get_identifiers()),
    );
    Expr::FunctionExpr {
        // Generated provenance and declared modes are semantic carriers, not
        // names rewritten by `ctx.original`.
        generated_origin: generated_origin.clone(),
        captures: captures.clone(),
        // C3-G12 nested-fn annotation carrier: semantic carrier, not a name
        // rewritten by `ctx.original`.
        annotations: annotations.clone(),
        params: params
            .iter()
            .map(|param| {
                let mut rewritten = param.clone();
                rewritten.default_value = param
                    .default_value
                    .as_ref()
                    .map(|default| rewrite_in_expr(default, bound, shadow));
                rewritten
            })
            .collect(),
        return_type: return_type.clone(),
        body: rewrite_statement_seq(body, &body_scope, shadow),
        span: *span,
    }
}
