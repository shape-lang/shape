//! Main query parsing implementation

use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

use crate::ast::Query;

use super::alert::parse_alert_query;
use super::with::parse_with_query;

/// Parse a query
///
/// Dispatches to the appropriate query parser based on the query type.
pub fn parse_query(pair: Pair<Rule>) -> Result<Query> {
    let pair_loc = pair_location(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected query content".to_string(),
            location: Some(
                pair_loc
                    .clone()
                    .with_hint("provide a query like 'alert' or 'with'"),
            ),
        })?;

    match inner.as_rule() {
        Rule::with_query => Ok(Query::With(parse_with_query(inner)?)),
        Rule::alert_query => Ok(Query::Alert(parse_alert_query(inner)?)),
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected query type: {:?}", inner.as_rule()),
            location: Some(pair_location(&inner)),
        }),
    }
}

/// Parse an inner query (used in subqueries and WITH clauses)
///
/// Inner queries don't have the outer query wrapper, so we dispatch directly.
pub fn parse_inner_query(pair: Pair<Rule>) -> Result<Query> {
    match pair.as_rule() {
        Rule::inner_query => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected inner query content".to_string(),
                    location: None,
                })?;
            parse_inner_query(inner)
        }
        Rule::alert_query => Ok(Query::Alert(parse_alert_query(pair)?)),
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected inner query type: {:?}", pair.as_rule()),
            location: Some(pair_location(&pair)),
        }),
    }
}
