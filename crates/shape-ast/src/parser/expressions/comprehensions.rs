//! List comprehension parsing
//!
//! This module handles parsing of list comprehensions:
//! - Basic comprehensions: [expr for var in iterable]
//! - Filtered comprehensions: [expr for var in iterable if condition]
//! - Nested comprehensions: [expr for x in xs for y in ys]

use crate::ast::{ComprehensionClause, Expr, ListComprehension};
use crate::error::Result;
use crate::parser::Rule;
use pest::iterators::Pair;

use super::super::pair_span;

/// Parse a list comprehension [expr for var in iterable if condition]
pub fn parse_list_comprehension(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // First is the element expression
    let element = Box::new(super::parse_expression(inner.next().unwrap())?);

    // Rest are comprehension clauses
    let mut clauses = vec![];

    for clause_pair in inner {
        if clause_pair.as_rule() == Rule::comprehension_clause {
            let mut clause_inner = clause_pair.into_inner();

            // Pattern
            let pattern = crate::parser::parse_pattern(clause_inner.next().unwrap())?;

            // Iterable expression
            let iterable = Box::new(super::parse_expression(clause_inner.next().unwrap())?);

            // Optional filter
            let filter = clause_inner
                .next()
                .map(|expr| super::parse_expression(expr))
                .transpose()?
                .map(Box::new);

            clauses.push(ComprehensionClause {
                pattern,
                iterable,
                filter,
            });
        }
    }

    Ok(Expr::ListComprehension(
        Box::new(ListComprehension { element, clauses }),
        span,
    ))
}
