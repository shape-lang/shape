//! Time-related parsing module

use crate::error::{Result, ShapeError};
use crate::parser::pair_location;
use crate::parser::string_literals::parse_string_literal;
use pest::iterators::Pair;

use super::Rule;
use crate::ast::{TimeReference, TimeUnit, TimeWindow};

/// Parse a time window
///
/// Supports: `last N units`, `between start and end`, `window(...)`, `session "start" to "end"`
pub fn parse_time_window(pair: Pair<Rule>) -> Result<TimeWindow> {
    let pair_loc = pair_location(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected time window specification".to_string(),
            location: Some(
                pair_loc
                    .clone()
                    .with_hint("use 'last N bars', 'last N days', 'between start and end', etc."),
            ),
        })?;

    match inner.as_rule() {
        Rule::last_window => parse_last_window(inner),
        Rule::between_window => parse_between_window(inner),
        Rule::window_range => parse_window_range(inner),
        Rule::session_window => parse_session_window(inner),
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected time window type: {:?}", inner.as_rule()),
            location: Some(pair_location(&inner)),
        }),
    }
}

fn parse_last_window(pair: Pair<Rule>) -> Result<TimeWindow> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let amount_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected amount in 'last' window".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("specify an amount, e.g., 'last 100 bars'"),
        ),
    })?;

    let amount: i32 = amount_pair
        .as_str()
        .parse()
        .map_err(|e| ShapeError::ParseError {
            message: format!("invalid number in time window: {}", e),
            location: Some(
                pair_location(&amount_pair).with_hint("amount must be a positive integer"),
            ),
        })?;

    let unit_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected time unit after amount".to_string(),
        location: Some(pair_loc.with_hint("add a time unit like 'bars', 'days', 'hours', 'weeks'")),
    })?;
    let unit = parse_time_unit(unit_pair)?;

    Ok(TimeWindow::Last { amount, unit })
}

fn parse_between_window(pair: Pair<Rule>) -> Result<TimeWindow> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let start_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected start time in 'between' window".to_string(),
        location: Some(pair_loc.clone().with_hint(
            "use 'between @yesterday and @today' or 'between \"2023-01-01\" and \"2023-12-31\"'",
        )),
    })?;
    let start = parse_time_ref(start_pair)?;

    let end_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected end time in 'between' window".to_string(),
        location: Some(pair_loc.with_hint("add 'and <end_time>' after start time")),
    })?;
    let end = parse_time_ref(end_pair)?;

    Ok(TimeWindow::Between { start, end })
}

fn parse_window_range(pair: Pair<Rule>) -> Result<TimeWindow> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Skip "window" keyword
    inner.next();

    let args = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected window arguments".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("use 'window(start, end)' with row indices or time references"),
        ),
    })?;

    let args_loc = pair_location(&args);
    let mut args_inner = args.into_inner();

    let first = args_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected first window argument".to_string(),
        location: Some(args_loc.clone()),
    })?;

    match first.as_rule() {
        Rule::number => {
            let start: i32 = first.as_str().parse().map_err(|e| ShapeError::ParseError {
                message: format!("invalid start index in window: {}", e),
                location: Some(pair_location(&first)),
            })?;

            let end = args_inner
                .next()
                .map(|p| {
                    p.as_str()
                        .parse::<i32>()
                        .map_err(|e| ShapeError::ParseError {
                            message: format!("invalid end index in window: {}", e),
                            location: Some(pair_location(&p)),
                        })
                })
                .transpose()?;

            Ok(TimeWindow::Window { start, end })
        }
        Rule::timeframe => Err(ShapeError::ParseError {
            message: "timeframe windows not yet implemented".to_string(),
            location: Some(
                pair_location(&first).with_hint("use row indices or time references instead"),
            ),
        }),
        Rule::time_ref => {
            let start = parse_time_ref(first)?;
            let end_pair = args_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected end time reference in window".to_string(),
                location: Some(
                    args_loc.with_hint("provide two time references: window(@start, @end)"),
                ),
            })?;
            let end = parse_time_ref(end_pair)?;
            Ok(TimeWindow::Between { start, end })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected window argument type: {:?}", first.as_rule()),
            location: Some(pair_location(&first)),
        }),
    }
}

fn parse_session_window(pair: Pair<Rule>) -> Result<TimeWindow> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Skip "session" keyword
    inner.next();

    let start_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected start time string in session window".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("use 'session \"09:30\" to \"16:00\"'"),
        ),
    })?;
    let start = parse_string_literal(start_pair.as_str())?;

    let end_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected end time string in session window".to_string(),
        location: Some(pair_loc.with_hint("add 'to \"HH:MM\"' after start time")),
    })?;
    let end = parse_string_literal(end_pair.as_str())?;

    Ok(TimeWindow::Session { start, end })
}

/// Parse a time unit (samples, records, minutes, hours, days, weeks, months)
pub fn parse_time_unit(pair: Pair<Rule>) -> Result<TimeUnit> {
    let unit_str = pair.as_str();

    match unit_str {
        "sample" | "samples" | "record" | "records" => Ok(TimeUnit::Samples),
        "minute" | "minutes" => Ok(TimeUnit::Minutes),
        "hour" | "hours" => Ok(TimeUnit::Hours),
        "day" | "days" => Ok(TimeUnit::Days),
        "week" | "weeks" => Ok(TimeUnit::Weeks),
        "month" | "months" => Ok(TimeUnit::Months),
        _ => {
            Err(ShapeError::ParseError {
                message: format!("unknown time unit: '{}'", unit_str),
                location: Some(pair_location(&pair).with_hint(
                    "valid units: samples, records, minutes, hours, days, weeks, months",
                )),
            })
        }
    }
}

fn parse_time_ref(pair: Pair<Rule>) -> Result<TimeReference> {
    crate::parser::expressions::temporal::parse_time_ref(pair)
}
