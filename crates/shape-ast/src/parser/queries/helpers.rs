//! Helper functions for query parsing

use crate::error::{Result, ShapeError};
use crate::parser::pair_location;
use pest::iterators::Pair;

use crate::ast::{Expr, OrderByClause, QueryModifiers, SortDirection};
use crate::parser::{Rule, expressions};

/// Parse query modifiers (LIMIT, ORDER BY)
pub fn parse_query_modifiers(pair: Pair<Rule>, modifiers: &mut QueryModifiers) -> Result<()> {
    match pair.as_rule() {
        Rule::limit_clause => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected integer after 'limit'".to_string(),
                    location: None,
                })?;
            let limit_str = inner.as_str();
            let limit_val = limit_str
                .parse::<usize>()
                .map_err(|_| ShapeError::ParseError {
                    message: format!(
                        "invalid limit value: '{}' - must be a positive integer",
                        limit_str
                    ),
                    location: Some(pair_location(&inner)),
                })?;
            modifiers.limit = Some(limit_val);
        }
        Rule::order_by_clause => {
            let mut columns = Vec::new();
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::order_by_list {
                    for item in inner.into_inner() {
                        if item.as_rule() == Rule::order_by_item {
                            let (expr, direction) = parse_order_by_item(item)?;
                            columns.push((expr, direction));
                        }
                    }
                }
            }
            modifiers.order_by = Some(OrderByClause { columns });
        }
        _ => {}
    }
    Ok(())
}

/// Parse a single ORDER BY item (expression + optional direction)
fn parse_order_by_item(pair: Pair<Rule>) -> Result<(Expr, SortDirection)> {
    let mut inner = pair.into_inner();

    // Parse the expression
    let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in ORDER BY clause".to_string(),
        location: None,
    })?;
    let expr = expressions::parse_expression(expr_pair)?;

    // Parse optional sort direction (defaults to ascending)
    let direction = if let Some(dir_pair) = inner.next() {
        match dir_pair.as_str().to_lowercase().as_str() {
            "asc" => SortDirection::Ascending,
            "desc" => SortDirection::Descending,
            _ => SortDirection::Ascending,
        }
    } else {
        SortDirection::Ascending
    };

    Ok((expr, direction))
}
