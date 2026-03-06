//! Conditional expression parsing
//!
//! This module handles parsing of conditional expressions:
//! - If/else expressions

use crate::ast::Expr;
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use pest::iterators::Pair;

use super::super::super::pair_span;
use crate::parser::pair_location;

/// Parse if expression
pub fn parse_if_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition in if expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let condition = super::super::parse_expression(condition_pair)?;

    let then_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected then branch in if expression".to_string(),
        location: Some(pair_loc),
    })?;
    let then_branch = parse_if_branch(then_pair)?;

    // Check if there's an else branch
    let else_branch = if let Some(else_pair) = inner.next() {
        let else_inner = else_pair
            .into_inner()
            .next()
            .ok_or_else(|| ShapeError::ParseError {
                message: "expected else branch in if expression".to_string(),
                location: None,
            })?;
        Some(Box::new(parse_if_branch(else_inner)?))
    } else {
        None
    };

    Ok(Expr::Conditional {
        condition: Box::new(condition),
        then_expr: Box::new(then_branch),
        else_expr: else_branch,
        span,
    })
}

fn parse_if_branch(pair: Pair<Rule>) -> Result<Expr> {
    match pair.as_rule() {
        Rule::block_expr => super::super::primary::parse_primary_expr(pair),
        Rule::if_expr => parse_if_expr(pair),
        _ => super::super::parse_expression(pair),
    }
}
