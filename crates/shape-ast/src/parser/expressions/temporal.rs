//! Temporal expression parsing
//!
//! This module handles parsing of time-related expressions:
//! - Time references (@today, @yesterday, "2024-01-01")
//! - DateTime expressions
//! - Duration expressions (1h, 5m30s, 2 days)
//! - Temporal navigation expressions
//! - Relative time expressions

use super::super::pair_span;
use crate::ast::{
    DateTimeExpr, Duration, DurationUnit, Expr, NamedTime, RelativeTime, TimeDirection,
    TimeReference, TimeUnit, Timeframe,
};
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use crate::parser::string_literals::parse_string_literal;
use pest::iterators::Pair;

/// Parse a time reference
pub fn parse_time_ref(pair: Pair<Rule>) -> Result<TimeReference> {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::quoted_time => Ok(TimeReference::Absolute(parse_string_literal(
            inner.as_str(),
        )?)),
        Rule::named_time => {
            let named = match inner.as_str() {
                "today" => NamedTime::Today,
                "yesterday" => NamedTime::Yesterday,
                "now" => NamedTime::Now,
                _ => {
                    return Err(ShapeError::ParseError {
                        message: format!("Unknown named time: {}", inner.as_str()),
                        location: None,
                    });
                }
            };
            Ok(TimeReference::Named(named))
        }
        Rule::relative_time => {
            // For now, store as string and parse later
            let s = inner.as_str();
            Ok(TimeReference::Relative(parse_relative_time(s)?))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected time reference: {:?}", inner.as_rule()),
            location: None,
        }),
    }
}

/// Parse relative time expression
pub fn parse_relative_time(s: &str) -> Result<RelativeTime> {
    // Simple parsing for now - this would be improved
    // Expected format: "1 week ago" or similar
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(ShapeError::ParseError {
            message: format!("Invalid relative time format: {}", s),
            location: None,
        });
    }

    let amount: i32 = parts[0].parse().map_err(|e| ShapeError::ParseError {
        message: format!("Invalid integer in relative time: {}", e),
        location: None,
    })?;
    let unit = match parts[1] {
        "minute" | "minutes" => TimeUnit::Minutes,
        "hour" | "hours" => TimeUnit::Hours,
        "day" | "days" => TimeUnit::Days,
        "week" | "weeks" => TimeUnit::Weeks,
        "month" | "months" => TimeUnit::Months,
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("Unknown time unit: {}", parts[1]),
                location: None,
            });
        }
    };

    let direction = match parts[2] {
        "ago" => TimeDirection::Ago,
        "future" | "ahead" => TimeDirection::Future,
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("Unknown time direction: {}", parts[2]),
                location: None,
            });
        }
    };

    Ok(RelativeTime {
        amount,
        unit,
        direction,
    })
}

/// Parse temporal navigation expression
/// Handles back(n) and forward(n) expressions
pub fn parse_temporal_nav(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::back_nav | Rule::forward_nav => {
            let is_back = inner.as_rule() == Rule::back_nav;
            let nav_amount = inner.into_inner().next().unwrap();
            let mut amount_inner = nav_amount.into_inner();

            // Parse the number
            let num_pair = amount_inner.next().unwrap();
            let value: f64 = num_pair
                .as_str()
                .parse()
                .map_err(|e| ShapeError::ParseError {
                    message: format!("Invalid navigation amount: {}", e),
                    location: None,
                })?;

            // Parse optional time unit (defaults to samples)
            let unit = if let Some(unit_pair) = amount_inner.next() {
                match unit_pair.as_str() {
                    "sample" | "samples" | "record" | "records" => DurationUnit::Samples,
                    "minute" | "minutes" => DurationUnit::Minutes,
                    "hour" | "hours" => DurationUnit::Hours,
                    "day" | "days" => DurationUnit::Days,
                    "week" | "weeks" => DurationUnit::Weeks,
                    "month" | "months" => DurationUnit::Months,
                    _ => DurationUnit::Samples,
                }
            } else {
                DurationUnit::Samples
            };

            // For back navigation, negate the value
            let final_value = if is_back { -value } else { value };

            Ok(Expr::Duration(
                Duration {
                    value: final_value,
                    unit,
                },
                span,
            ))
        }
        _ => Err(ShapeError::ParseError {
            message: format!(
                "Expected back_nav or forward_nav, got {:?}",
                inner.as_rule()
            ),
            location: None,
        }),
    }
}

/// Parse timeframe expression
pub fn parse_timeframe_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Parse the timeframe
    let timeframe_str = inner
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "Expected timeframe in on() expression".to_string(),
            location: None,
        })?
        .as_str();

    let timeframe = Timeframe::parse(timeframe_str).ok_or_else(|| ShapeError::ParseError {
        message: format!("Invalid timeframe: {}", timeframe_str),
        location: None,
    })?;

    // Parse the expression
    let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Expected expression in on() block".to_string(),
        location: None,
    })?;

    let expr = crate::parser::expressions::parse_expression(expr_pair)?;

    Ok(Expr::TimeframeContext {
        timeframe,
        expr: Box::new(expr),
        span,
    })
}

/// Parse datetime expression
pub fn parse_datetime_expr(pair: Pair<Rule>) -> Result<DateTimeExpr> {
    match pair.as_rule() {
        Rule::datetime_expr => {
            // Delegate to inner rule
            let inner = pair.into_inner().next().unwrap();
            parse_datetime_expr(inner)
        }
        Rule::datetime_primary => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().unwrap();

            match expr_pair.as_rule() {
                Rule::datetime_literal => {
                    let mut lit_inner = expr_pair.into_inner();
                    let string_pair = lit_inner.next().unwrap();
                    Ok(DateTimeExpr::Literal(parse_string_literal(
                        string_pair.as_str(),
                    )?))
                }
                Rule::named_time => {
                    let named = match expr_pair.as_str() {
                        "today" => NamedTime::Today,
                        "yesterday" => NamedTime::Yesterday,
                        "now" => NamedTime::Now,
                        _ => {
                            return Err(ShapeError::ParseError {
                                message: format!("Unknown named time: {}", expr_pair.as_str()),
                                location: None,
                            });
                        }
                    };
                    Ok(DateTimeExpr::Named(named))
                }
                _ => Err(ShapeError::ParseError {
                    message: format!("Unexpected datetime primary: {:?}", expr_pair.as_rule()),
                    location: None,
                }),
            }
        }
        Rule::datetime_arithmetic => {
            let mut inner = pair.into_inner();
            let base_pair = inner.next().unwrap();
            let mut result = parse_datetime_expr(base_pair)?;

            while let Some(op_pair) = inner.next() {
                let op = op_pair.as_str();
                if op != "+" && op != "-" {
                    return Err(ShapeError::ParseError {
                        message: format!("Invalid datetime arithmetic operator: {}", op),
                        location: None,
                    });
                }

                let duration_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                    message: "Datetime arithmetic missing duration".to_string(),
                    location: None,
                })?;
                let duration_expr = parse_duration(duration_pair)?;
                let duration = match duration_expr {
                    Expr::Duration(duration, _) => duration,
                    _ => {
                        return Err(ShapeError::ParseError {
                            message: "Datetime arithmetic expects a duration".to_string(),
                            location: None,
                        });
                    }
                };

                result = DateTimeExpr::Arithmetic {
                    base: Box::new(result),
                    operator: op.to_string(),
                    duration,
                };
            }

            Ok(result)
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected datetime expression: {:?}", pair.as_rule()),
            location: None,
        }),
    }
}

/// Parse datetime range
pub fn parse_datetime_range(pair: Pair<Rule>) -> Result<(Expr, Option<Expr>)> {
    // Parse datetime_range: datetime_expr ("to" datetime_expr)?
    let mut inner = pair.into_inner();
    let first_pair = inner.next().unwrap();
    let first_span = pair_span(&first_pair);
    let first_datetime = parse_datetime_expr(first_pair)?;

    // Check if there's a "to" and second datetime
    if let Some(second_pair) = inner.next() {
        let second_span = pair_span(&second_pair);
        let second_datetime = parse_datetime_expr(second_pair)?;
        Ok((
            Expr::DateTime(first_datetime, first_span),
            Some(Expr::DateTime(second_datetime, second_span)),
        ))
    } else {
        Ok((Expr::DateTime(first_datetime, first_span), None))
    }
}

/// Parse duration expression
pub fn parse_duration(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    // Since duration is now atomic, parse the string directly
    let duration_str = pair.as_str();

    // Check if it's a compound duration (contains multiple units)
    let mut components = Vec::new();
    let mut current_number = String::new();
    let mut chars = duration_str.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_numeric() || ch == '.' || (ch == '-' && current_number.is_empty()) {
            current_number.push(ch);
        } else {
            // We've hit a unit character
            if !current_number.is_empty() {
                let value: f64 = current_number.parse().map_err(|e| ShapeError::ParseError {
                    message: format!("Invalid duration value: {}", e),
                    location: None,
                })?;

                // Collect the unit string
                let mut unit_str = String::new();
                unit_str.push(ch);

                // For long unit names like "minutes", "hours", etc.
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_alphabetic() {
                        unit_str.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                let unit = match unit_str.as_str() {
                    "s" | "seconds" => DurationUnit::Seconds,
                    "m" | "minutes" => DurationUnit::Minutes,
                    "h" | "hours" => DurationUnit::Hours,
                    "d" | "days" => DurationUnit::Days,
                    "w" | "weeks" => DurationUnit::Weeks,
                    "M" | "months" => DurationUnit::Months,
                    "y" | "years" => DurationUnit::Years,
                    "samples" => DurationUnit::Samples,
                    _ => {
                        return Err(ShapeError::ParseError {
                            message: format!("Unknown duration unit: {}", unit_str),
                            location: None,
                        });
                    }
                };

                components.push((value, unit));
                current_number.clear();
            }
        }
    }

    // If there's only one component, return it directly
    if components.len() == 1 {
        let (value, unit) = components.into_iter().next().unwrap();
        return Ok(Expr::Duration(Duration { value, unit }, span));
    }

    // For compound durations, convert to seconds and find appropriate unit
    let mut total_seconds = 0.0;
    for (value, unit) in components {
        let seconds = match unit {
            DurationUnit::Seconds => value,
            DurationUnit::Minutes => value * 60.0,
            DurationUnit::Hours => value * 3600.0,
            DurationUnit::Days => value * 86400.0,
            DurationUnit::Weeks => value * 604800.0,
            DurationUnit::Months => value * 2592000.0, // Approximate: 30 days
            DurationUnit::Years => value * 31536000.0, // Approximate: 365 days
            DurationUnit::Samples => {
                return Err(ShapeError::ParseError {
                    message: "Cannot use 'samples' in compound duration".to_string(),
                    location: None,
                });
            }
        };
        total_seconds += seconds;
    }

    // Convert back to the most appropriate unit
    let (value, unit) = if total_seconds < 60.0 {
        (total_seconds, DurationUnit::Seconds)
    } else if total_seconds < 3600.0 {
        (total_seconds / 60.0, DurationUnit::Minutes)
    } else if total_seconds < 86400.0 {
        (total_seconds / 3600.0, DurationUnit::Hours)
    } else if total_seconds < 604800.0 {
        (total_seconds / 86400.0, DurationUnit::Days)
    } else if total_seconds < 2592000.0 {
        (total_seconds / 604800.0, DurationUnit::Weeks)
    } else if total_seconds < 31536000.0 {
        (total_seconds / 2592000.0, DurationUnit::Months)
    } else {
        (total_seconds / 31536000.0, DurationUnit::Years)
    };

    Ok(Expr::Duration(Duration { value, unit }, span))
}
