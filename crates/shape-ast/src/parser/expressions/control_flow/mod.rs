//! Control flow expression parsing
//!
//! This module handles parsing of control flow expressions:
//! - If/else expressions (conditionals)
//! - While loops (loops)
//! - For loops (loops)
//! - Infinite loops (loops)
//! - Let expressions (loops)
//! - Match expressions (pattern_matching)
//! - Try/catch expressions (conditionals)
//! - Break/continue/return (loops)
//! - Block expressions (loops)
//! - Pattern matching (pattern_matching)
//! - LINQ-style query expressions (this module)

use crate::ast::{Expr, FromQueryExpr, OrderBySpec, QueryClause};
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

use super::super::pair_span;

mod conditionals;
mod loops;
mod pattern_matching;

// Re-export all public functions
pub use conditionals::parse_if_expr;
pub use loops::{
    parse_async_let_expr, parse_async_scope_expr, parse_block_expr, parse_break_expr,
    parse_for_expr, parse_let_expr, parse_loop_expr, parse_return_expr, parse_while_expr,
};
pub use pattern_matching::{parse_match_expr, parse_pattern};

/// Parse LINQ-style from query expression
/// Syntax: from var in source [clauses...] select expr
pub fn parse_from_query_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse: from variable in source
    let variable_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected variable name in from clause".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let variable = variable_pair.as_str().to_string();

    let source_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected source expression in from clause".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let source = Box::new(parse_query_source_expr(source_pair)?);

    // Parse clauses
    let mut clauses = Vec::new();
    let mut select = None;

    for part in inner {
        match part.as_rule() {
            Rule::query_clause => {
                let clause_inner =
                    part.into_inner()
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected query clause content".to_string(),
                            location: Some(pair_loc.clone()),
                        })?;
                let clause = parse_query_clause(clause_inner)?;
                clauses.push(clause);
            }
            Rule::where_query_clause => {
                clauses.push(parse_where_query_clause(part)?);
            }
            Rule::order_by_query_clause => {
                clauses.push(parse_order_by_query_clause(part)?);
            }
            Rule::group_by_query_clause => {
                clauses.push(parse_group_by_query_clause(part)?);
            }
            Rule::join_query_clause => {
                clauses.push(parse_join_query_clause(part)?);
            }
            Rule::let_query_clause => {
                clauses.push(parse_let_query_clause(part)?);
            }
            Rule::select_query_clause => {
                let select_inner =
                    part.into_inner()
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected expression after select".to_string(),
                            location: Some(pair_loc.clone()),
                        })?;
                select = Some(Box::new(parse_query_expr_inner(select_inner)?));
            }
            _ => {}
        }
    }

    let select = select.ok_or_else(|| ShapeError::ParseError {
        message: "from query requires a select clause".to_string(),
        location: Some(pair_loc),
    })?;

    Ok(Expr::FromQuery(
        Box::new(FromQueryExpr {
            variable,
            source,
            clauses,
            select,
        }),
        span,
    ))
}

/// Parse a query source expression (postfix_expr)
fn parse_query_source_expr(pair: Pair<Rule>) -> Result<Expr> {
    match pair.as_rule() {
        Rule::query_source_expr => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected source expression".to_string(),
                    location: None,
                })?;
            super::parse_postfix_expr(inner)
        }
        _ => super::parse_postfix_expr(pair),
    }
}

/// Parse a query inner expression (comparison_expr)
fn parse_query_expr_inner(pair: Pair<Rule>) -> Result<Expr> {
    match pair.as_rule() {
        Rule::query_expr_inner => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression".to_string(),
                    location: None,
                })?;
            super::binary_ops::parse_comparison_expr(inner)
        }
        _ => super::binary_ops::parse_comparison_expr(pair),
    }
}

fn parse_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    match pair.as_rule() {
        Rule::where_query_clause => parse_where_query_clause(pair),
        Rule::order_by_query_clause => parse_order_by_query_clause(pair),
        Rule::group_by_query_clause => parse_group_by_query_clause(pair),
        Rule::join_query_clause => parse_join_query_clause(pair),
        Rule::let_query_clause => parse_let_query_clause(pair),
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected query clause: {:?}", pair.as_rule()),
            location: Some(pair_location(&pair)),
        }),
    }
}

fn parse_where_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    let pair_loc = pair_location(&pair);
    let condition_pair = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected condition expression in where clause".to_string(),
            location: Some(pair_loc),
        })?;
    let condition = parse_query_expr_inner(condition_pair)?;
    Ok(QueryClause::Where(Box::new(condition)))
}

fn parse_order_by_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    let mut specs = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::order_by_spec {
            specs.push(parse_order_by_spec(inner)?);
        }
    }
    Ok(QueryClause::OrderBy(specs))
}

fn parse_order_by_spec(pair: Pair<Rule>) -> Result<OrderBySpec> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let key_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected key expression in order by".to_string(),
        location: Some(pair_loc),
    })?;
    let key = Box::new(super::parse_postfix_expr(key_pair)?);
    let descending = inner
        .next()
        .map(|dir| dir.as_str() == "desc")
        .unwrap_or(false);
    Ok(OrderBySpec { key, descending })
}

fn parse_group_by_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let element_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected element expression in group by".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let element = Box::new(super::parse_postfix_expr(element_pair)?);

    let key_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected key expression in group by".to_string(),
        location: Some(pair_loc),
    })?;
    let key = Box::new(super::parse_postfix_expr(key_pair)?);

    let into_var = inner.next().map(|p| p.as_str().to_string());

    Ok(QueryClause::GroupBy {
        element,
        key,
        into_var,
    })
}

fn parse_join_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let variable = inner
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected variable name in join".to_string(),
            location: Some(pair_loc.clone()),
        })?
        .as_str()
        .to_string();

    let source_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected source expression in join".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let source = Box::new(super::parse_postfix_expr(source_pair)?);

    let left_key_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected left key expression in join".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let left_key = Box::new(super::parse_postfix_expr(left_key_pair)?);

    let right_key_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected right key expression in join".to_string(),
        location: Some(pair_loc),
    })?;
    let right_key = Box::new(super::parse_postfix_expr(right_key_pair)?);

    let into_var = inner.next().map(|p| p.as_str().to_string());

    Ok(QueryClause::Join {
        variable,
        source,
        left_key,
        right_key,
        into_var,
    })
}

fn parse_let_query_clause(pair: Pair<Rule>) -> Result<QueryClause> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let variable = inner
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected variable name in let clause".to_string(),
            location: Some(pair_loc.clone()),
        })?
        .as_str()
        .to_string();

    let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected value expression in let clause".to_string(),
        location: Some(pair_loc),
    })?;
    let value = Box::new(parse_query_expr_inner(value_pair)?);

    Ok(QueryClause::Let { variable, value })
}
