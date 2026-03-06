//! Data reference expression parsing
//!
//! This module handles parsing of generic data/DataFrame references:
//! - Simple data references: data[0], data[-1]
//! - Timeframe-specific references: data(5m)[0]
//! - DateTime-based references: data[@2024-01-01]
//! - Relative access: data[@today][-1]
//! - Index parsing and range expressions

use super::super::pair_span;
use crate::ast::{DataDateTimeRef, DataIndex, DataRef, Expr, Literal, Timeframe, UnaryOp};
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use pest::iterators::Pair;

/// Parse a data reference
pub fn parse_data_ref(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();
    let mut timeframe: Option<Timeframe> = None;

    // Check if the first item is a timeframe specification
    if let Some(first) = inner.peek() {
        if first.as_rule() == Rule::timeframe_spec {
            let timeframe_spec = inner.next().unwrap();
            let timeframe_inner = timeframe_spec.into_inner().next().unwrap();

            // Parse the timeframe
            match timeframe_inner.as_rule() {
                Rule::timeframe => {
                    timeframe = Timeframe::parse(timeframe_inner.as_str());
                    if timeframe.is_none() {
                        return Err(ShapeError::ParseError {
                            message: format!("Invalid timeframe: {}", timeframe_inner.as_str()),
                            location: None,
                        });
                    }
                }
                Rule::expression => {
                    // Dynamic timeframe expressions require runtime evaluation
                    // The grammar allows them, but the AST DataRef only supports static timeframes
                    // This would require a DataRef variant with Box<Expr> for dynamic timeframes
                    return Err(ShapeError::ParseError {
                        message: "Dynamic timeframe expressions in data references require runtime evaluation. Use a static timeframe like data(5m)[0] instead.".to_string(),
                        location: None,
                    });
                }
                _ => {}
            }
        }
    }

    // Parse the access part (required - grammar enforces this)
    let access = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "data reference requires brackets: data[0], data[-1], data[@datetime]".to_string(),
        location: None,
    })?;

    match access.as_rule() {
        Rule::datetime_access => {
            // Parse datetime-based access
            let mut datetime_inner = access.into_inner();
            let datetime_expr_pair = datetime_inner.next().unwrap();
            let (start_expr, end_expr) = super::temporal::parse_datetime_range(datetime_expr_pair)?;
            if end_expr.is_some() {
                return Err(ShapeError::ParseError {
                    message: "Datetime ranges are not supported in data access".to_string(),
                    location: None,
                });
            }
            let datetime_expr = match start_expr {
                Expr::DateTime(expr, _) => expr,
                _ => {
                    return Err(ShapeError::ParseError {
                        message: "Expected datetime expression in data access".to_string(),
                        location: None,
                    });
                }
            };

            // Check for optional timeframe parameter
            let mut datetime_timeframe: Option<Timeframe> = None;
            let next_item = datetime_inner.peek();

            if let Some(item) = next_item {
                match item.as_rule() {
                    Rule::timeframe => {
                        let tf_pair = datetime_inner.next().unwrap();
                        datetime_timeframe = Timeframe::parse(tf_pair.as_str());
                        if datetime_timeframe.is_none() {
                            return Err(ShapeError::ParseError {
                                message: format!("Invalid timeframe: {}", tf_pair.as_str()),
                                location: None,
                            });
                        }
                    }
                    Rule::expression => {
                        // Dynamic timeframe expressions require runtime evaluation
                        return Err(ShapeError::ParseError {
                                message: "Dynamic timeframe expressions in data references require runtime evaluation. Use a static timeframe instead."
                                    .to_string(),
                                location: None,
                            });
                    }
                    Rule::index_access => {
                        // This is an index access, not a timeframe
                    }
                    _ => {}
                }
            }

            // Use the timeframe from datetime_access if present, otherwise use the one from timeframe_spec
            let final_timeframe = datetime_timeframe.or(timeframe);

            // Check if there's a subsequent index access
            if let Some(index_access) = datetime_inner.next() {
                if index_access.as_rule() == Rule::index_access {
                    // This is a relative access from a datetime reference
                    // Timeframe is already captured in the DataDateTimeRef
                    let (index, _) = parse_index_expr(index_access.into_inner().next().unwrap())?;
                    Ok(Expr::DataRelativeAccess {
                        reference: Box::new(Expr::DataDateTimeRef(
                            DataDateTimeRef {
                                datetime: datetime_expr,
                                timezone: None,
                                timeframe: final_timeframe,
                            },
                            span,
                        )),
                        index,
                        span,
                    })
                } else {
                    // Just a datetime reference
                    Ok(Expr::DataDateTimeRef(
                        DataDateTimeRef {
                            datetime: datetime_expr,
                            timezone: None,
                            timeframe: final_timeframe,
                        },
                        span,
                    ))
                }
            } else {
                // Just a datetime reference
                Ok(Expr::DataDateTimeRef(
                    DataDateTimeRef {
                        datetime: datetime_expr,
                        timezone: None,
                        timeframe: final_timeframe,
                    },
                    span,
                ))
            }
        }
        Rule::index_access => {
            // Traditional integer-based access
            let index_expr = access.into_inner().next().unwrap();
            let (index, index_timeframe) = parse_index_expr(index_expr)?;
            // Use the timeframe from index_expr if present, otherwise use the one from timeframe_spec
            let final_timeframe = index_timeframe.or(timeframe);
            Ok(Expr::DataRef(
                DataRef {
                    index,
                    timeframe: final_timeframe,
                },
                span,
            ))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected data access type: {:?}", access.as_rule()),
            location: None,
        }),
    }
}

/// Parse index expression (with optional timeframe)
pub fn parse_index_expr(pair: Pair<Rule>) -> Result<(DataIndex, Option<Timeframe>)> {
    // This parses index_expr which can be:
    // - expression (single index)
    // - expression:expression (range)
    // - expression, timeframe (single index with timeframe)
    // - expression:expression, timeframe (range with timeframe)

    let span = pair_span(&pair);
    let mut inner = pair.into_inner();
    let first_expr = inner.next().unwrap();

    // First, try to parse as an integer for optimization
    let index = if first_expr.as_rule() == Rule::integer {
        let first_val: i32 = first_expr
            .as_str()
            .parse()
            .map_err(|e| ShapeError::ParseError {
                message: format!("Invalid integer: {}", e),
                location: None,
            })?;

        // Check if there's a colon (range indicator)
        let mut has_range = false;
        let mut range_end = None;

        if let Some(next) = inner.peek() {
            if next.as_rule() == Rule::expression {
                // Could be a range
                has_range = true;
                let second_expr = inner.next().unwrap();
                if second_expr.as_rule() == Rule::integer {
                    let second_val: i32 =
                        second_expr
                            .as_str()
                            .parse()
                            .map_err(|e| ShapeError::ParseError {
                                message: format!("Invalid integer: {}", e),
                                location: None,
                            })?;
                    range_end = Some(second_val);
                } else {
                    // Expression range
                    let expr = super::parse_expression(second_expr)?;
                    return Ok((
                        DataIndex::ExpressionRange(
                            Box::new(Expr::Literal(Literal::Number(first_val as f64), span)),
                            Box::new(expr),
                        ),
                        parse_optional_timeframe(&mut inner)?,
                    ));
                }
            }
        }

        if has_range && range_end.is_some() {
            DataIndex::Range(first_val, range_end.unwrap())
        } else {
            DataIndex::Single(first_val)
        }
    } else {
        // Parse as expression
        let expr = super::parse_expression(first_expr)?;
        if let Expr::Range {
            ref start, ref end, ..
        } = expr
        {
            // Range expression inside index, treat as data range.
            // Both start and end must be present for data ranges
            if let (Some(start_expr), Some(end_expr)) = (start, end) {
                if let (Some(start_const), Some(end_const)) = (
                    try_evaluate_constant_index(start_expr),
                    try_evaluate_constant_index(end_expr),
                ) {
                    let timeframe = parse_optional_timeframe(&mut inner)?;
                    return Ok((DataIndex::Range(start_const, end_const), timeframe));
                }

                let timeframe = parse_optional_timeframe(&mut inner)?;
                return Ok((
                    DataIndex::ExpressionRange(start_expr.clone(), end_expr.clone()),
                    timeframe,
                ));
            }
        }

        // Check if it's a constant
        if let Some(const_val) = try_evaluate_constant_index(&expr) {
            // Check for range
            if let Some(next) = inner.peek() {
                if next.as_rule() == Rule::expression {
                    let second_expr = super::parse_expression(inner.next().unwrap())?;
                    if let Some(second_const) = try_evaluate_constant_index(&second_expr) {
                        DataIndex::Range(const_val, second_const)
                    } else {
                        DataIndex::ExpressionRange(
                            Box::new(Expr::Literal(Literal::Number(const_val as f64), span)),
                            Box::new(second_expr),
                        )
                    }
                } else {
                    DataIndex::Single(const_val)
                }
            } else {
                DataIndex::Single(const_val)
            }
        } else {
            // Dynamic expression
            if let Some(next) = inner.peek() {
                if next.as_rule() == Rule::expression {
                    let second_expr = super::parse_expression(inner.next().unwrap())?;
                    DataIndex::ExpressionRange(Box::new(expr), Box::new(second_expr))
                } else {
                    DataIndex::Expression(Box::new(expr))
                }
            } else {
                DataIndex::Expression(Box::new(expr))
            }
        }
    };

    // Now parse the optional timeframe
    let timeframe = parse_optional_timeframe(&mut inner)?;

    Ok((index, timeframe))
}

/// Parse optional timeframe
pub fn parse_optional_timeframe(
    inner: &mut pest::iterators::Pairs<Rule>,
) -> Result<Option<Timeframe>> {
    if let Some(next) = inner.next() {
        match next.as_rule() {
            Rule::timeframe => {
                let tf = Timeframe::parse(next.as_str());
                if tf.is_none() {
                    return Err(ShapeError::ParseError {
                        message: format!("Invalid timeframe: {}", next.as_str()),
                        location: None,
                    });
                }
                Ok(tf)
            }
            Rule::expression => {
                // Dynamic timeframe expression - not supported yet
                Err(ShapeError::ParseError {
                    message: "Dynamic timeframe expressions not yet supported".to_string(),
                    location: None,
                })
            }
            _ => Err(ShapeError::ParseError {
                message: format!("Expected timeframe or expression, got {:?}", next.as_rule()),
                location: None,
            }),
        }
    } else {
        Ok(None)
    }
}

/// Try to evaluate an expression as a constant integer at parse time
fn try_evaluate_constant_index(expr: &Expr) -> Option<i32> {
    match expr {
        Expr::Literal(Literal::Number(n), _) => Some(*n as i32),
        Expr::UnaryOp {
            op: UnaryOp::Neg,
            operand,
            ..
        } => {
            if let Expr::Literal(Literal::Number(n), _) = operand.as_ref() {
                Some(-(*n as i32))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parse a general index expression (can be any expression, not just integers)
pub fn parse_index_expr_general(pair: Pair<Rule>) -> Result<(Expr, Option<Expr>)> {
    // The pair here is the full index_expr, we need to look at its contents
    match pair.as_rule() {
        Rule::index_expr => {
            // Handle the actual parsing of index_expr contents
            let mut inner = pair.into_inner();
            let first_pair = inner.next().unwrap();

            // Check if first element is a datetime_range
            let (first_expr, mut second_expr) = match first_pair.as_rule() {
                Rule::datetime_range => {
                    // Parse datetime range directly
                    super::temporal::parse_datetime_range(first_pair)?
                }
                _ => {
                    // Parse as regular expression
                    let expr = super::parse_expression(first_pair)?;
                    (expr, None)
                }
            };

            // Check if there's a colon and second part (range)
            if let Some(next_pair) = inner.next() {
                // This should be the second part of the range
                match next_pair.as_rule() {
                    Rule::datetime_range => {
                        let (range_end, _) = super::temporal::parse_datetime_range(next_pair)?;
                        second_expr = Some(range_end);
                    }
                    Rule::expression => {
                        second_expr = Some(super::parse_expression(next_pair)?);
                    }
                    _ => {
                        // Skip timeframe or other tokens
                    }
                }
            }

            if second_expr.is_none() {
                if let Expr::Range {
                    ref start, ref end, ..
                } = first_expr
                {
                    // For ranges with both start and end, extract them
                    if let (Some(s), Some(e)) = (start, end) {
                        return Ok((*s.clone(), Some(*e.clone())));
                    }
                }
            }

            Ok((first_expr, second_expr))
        }
        _ => {
            // Fallback for when called with other rules
            let mut inner = pair.into_inner();
            let first = super::parse_expression(inner.next().unwrap())?;

            // Check if there's a second expression (range)
            if let Some(second) = inner.next() {
                let end = super::parse_expression(second)?;
                Ok((first, Some(end)))
            } else {
                Ok((first, None))
            }
        }
    }
}
