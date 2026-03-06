//! Alert query parsing

use crate::error::{Result, ShapeError};
use crate::parser::pair_location;
use pest::iterators::Pair;

use crate::ast::AlertQuery;
use crate::parser::string_literals::parse_string_literal;
use crate::parser::{Rule, expressions};

/// Parse an alert query
///
/// Syntax: `alert when <condition> [message <string>] [webhook <url>]`
pub fn parse_alert_query(pair: Pair<Rule>) -> Result<AlertQuery> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Skip "alert" keyword
    inner.next();
    // Skip "when" keyword
    inner.next();

    // Parse condition (required)
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition after 'alert when'".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("provide a boolean condition, e.g., alert when rsi(14) > 70"),
        ),
    })?;
    let condition = expressions::parse_expression(condition_pair)?;

    let mut message = None;
    let mut webhook = None;

    // Parse optional alert options
    if let Some(options) = inner.next() {
        let mut opt_inner = options.into_inner();
        if let Some(msg_pair) = opt_inner.next() {
            if msg_pair.as_rule() == Rule::string {
                message = Some(parse_string_literal(msg_pair.as_str())?);
            }
        }
        if let Some(webhook_pair) = opt_inner.next() {
            if webhook_pair.as_rule() == Rule::string {
                webhook = Some(parse_string_literal(webhook_pair.as_str())?);
            }
        }
    }

    Ok(AlertQuery {
        condition,
        message,
        webhook,
    })
}
