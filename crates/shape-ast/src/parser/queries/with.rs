//! WITH query (CTE) parsing implementation

use crate::ast::{Cte, Query, WithQuery};
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

use super::alert::parse_alert_query;

/// Parse an inner query (used in CTEs and as the main query)
fn parse_inner_query(pair: Pair<Rule>) -> Result<Query> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected query in CTE".to_string(),
            location: None,
        })?;

    match inner.as_rule() {
        Rule::alert_query => Ok(Query::Alert(parse_alert_query(inner)?)),
        Rule::with_query => Ok(Query::With(parse_with_query(inner)?)),
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected query type in CTE: {:?}", inner.as_rule()),
            location: Some(pair_location(&inner)),
        }),
    }
}

/// Parse a single CTE definition
fn parse_cte_def(pair: Pair<Rule>) -> Result<Cte> {
    let inner = pair.into_inner();
    let mut recursive = false;
    let mut name = String::new();
    let mut columns = None;
    let mut query = None;

    for item in inner {
        match item.as_rule() {
            Rule::recursive_keyword => {
                recursive = true;
            }
            Rule::ident => {
                name = item.as_str().to_string();
            }
            Rule::cte_columns => {
                let cols: Vec<String> = item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::ident)
                    .map(|p| p.as_str().to_string())
                    .collect();
                columns = Some(cols);
            }
            Rule::inner_query => {
                query = Some(parse_inner_query(item)?);
            }
            _ => {}
        }
    }

    let query = query.ok_or_else(|| ShapeError::ParseError {
        message: "CTE definition missing query".to_string(),
        location: None,
    })?;

    Ok(Cte {
        name,
        columns,
        query: Box::new(query),
        recursive,
    })
}

/// Parse a list of CTEs
fn parse_cte_list(pair: Pair<Rule>) -> Result<Vec<Cte>> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::cte_def)
        .map(parse_cte_def)
        .collect()
}

/// Parse a WITH query (query with Common Table Expressions)
///
/// Syntax:
/// ```text
/// WITH
///     cte1 AS (query1),
///     cte2 AS (query2),
///     RECURSIVE cte3(col1, col2) AS (query3)
/// main_query
/// ```
pub fn parse_with_query(pair: Pair<Rule>) -> Result<WithQuery> {
    let inner = pair.into_inner();
    let mut ctes = Vec::new();
    let mut main_query = None;

    for item in inner {
        match item.as_rule() {
            Rule::cte_list => {
                ctes = parse_cte_list(item)?;
            }
            Rule::inner_query => {
                main_query = Some(parse_inner_query(item)?);
            }
            _ => {}
        }
    }

    let query = main_query.ok_or_else(|| ShapeError::ParseError {
        message: "WITH clause missing main query".to_string(),
        location: None,
    })?;

    Ok(WithQuery {
        ctes,
        query: Box::new(query),
    })
}
