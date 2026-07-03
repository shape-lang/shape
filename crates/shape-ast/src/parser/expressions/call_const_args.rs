//! Parser support for explicit call-site const generic arguments.

use crate::ast::Expr;
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

pub(super) fn parse_call_const_args(pair: Pair<Rule>) -> Result<Vec<Expr>> {
    let pair_loc = pair_location(&pair);
    let mut args = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::call_const_arg_list => {
                for arg in inner.into_inner() {
                    if arg.as_rule() == Rule::call_const_arg {
                        let expr_pair =
                            arg.into_inner()
                                .next()
                                .ok_or_else(|| ShapeError::ParseError {
                                    message: "expected const generic argument".to_string(),
                                    location: Some(pair_loc.clone()),
                                })?;
                        args.push(super::binary_ops::parse_unary_expr(expr_pair)?);
                    }
                }
            }
            Rule::call_const_arg => {
                let expr_pair =
                    inner
                        .into_inner()
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected const generic argument".to_string(),
                            location: Some(pair_loc.clone()),
                        })?;
                args.push(super::binary_ops::parse_unary_expr(expr_pair)?);
            }
            _ => {}
        }
    }
    Ok(args)
}
