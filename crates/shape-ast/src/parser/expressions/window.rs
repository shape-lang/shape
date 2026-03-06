//! Window function expression parser
//!
//! Parses window function expressions like:
//! - `lag(close, 1) over (partition by symbol order by timestamp)`
//! - `row_number() over (order by close desc)`
//! - `sum(volume) over (rows between 5 preceding and current row)`

use crate::ast::{
    Expr, Literal, OrderByClause, SortDirection, WindowBound, WindowExpr, WindowFrame,
    WindowFrameType, WindowFunction, WindowSpec,
};
use crate::error::{Result, ShapeError, SourceLocation};
use crate::parser::{Rule, pair_location, pair_span};
use pest::iterators::Pair;

use super::super::expressions;

/// Parse a window function call expression
///
/// Grammar: `window_function_name "(" window_function_args? ")" over_clause`
pub fn parse_window_function_call(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse function name
    let func_name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected window function name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let func_name = func_name_pair.as_str().to_lowercase();

    // Parse function arguments (optional)
    let mut args = Vec::new();
    let mut over_pair = None;

    for part in inner {
        match part.as_rule() {
            Rule::window_function_args => {
                for arg_pair in part.into_inner() {
                    if arg_pair.as_rule() == Rule::expression {
                        args.push(expressions::parse_expression(arg_pair)?);
                    }
                }
            }
            Rule::over_clause => {
                over_pair = Some(part);
            }
            _ => {}
        }
    }

    // Parse OVER clause
    let over_clause = over_pair.ok_or_else(|| ShapeError::ParseError {
        message: "window function requires OVER clause".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("add OVER (...) after the function call"),
        ),
    })?;
    let window_spec = parse_over_clause(over_clause)?;

    // Build WindowFunction based on name
    let function = build_window_function(&func_name, args, &pair_loc)?;

    Ok(Expr::WindowExpr(
        Box::new(WindowExpr {
            function,
            over: window_spec,
        }),
        span,
    ))
}

/// Build a WindowFunction from the parsed name and arguments
fn build_window_function(
    name: &str,
    args: Vec<Expr>,
    loc: &SourceLocation,
) -> Result<WindowFunction> {
    match name {
        "lag" => {
            let expr = args.first().cloned().unwrap_or(Expr::Identifier(
                "close".to_string(),
                crate::ast::Span::new(0, 0),
            ));
            let offset = extract_usize(&args.get(1).cloned()).unwrap_or(1);
            let default = args.get(2).map(|e| Box::new(e.clone()));
            Ok(WindowFunction::Lag {
                expr: Box::new(expr),
                offset,
                default,
            })
        }
        "lead" => {
            let expr = args.first().cloned().unwrap_or(Expr::Identifier(
                "close".to_string(),
                crate::ast::Span::new(0, 0),
            ));
            let offset = extract_usize(&args.get(1).cloned()).unwrap_or(1);
            let default = args.get(2).map(|e| Box::new(e.clone()));
            Ok(WindowFunction::Lead {
                expr: Box::new(expr),
                offset,
                default,
            })
        }
        "row_number" => Ok(WindowFunction::RowNumber),
        "rank" => Ok(WindowFunction::Rank),
        "dense_rank" => Ok(WindowFunction::DenseRank),
        "ntile" => {
            let n = extract_usize(&args.first().cloned()).unwrap_or(1);
            Ok(WindowFunction::Ntile(n))
        }
        "first_value" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "first_value requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::FirstValue(Box::new(expr)))
        }
        "last_value" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "last_value requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::LastValue(Box::new(expr)))
        }
        "nth_value" => {
            let mut iter = args.into_iter();
            let expr = iter.next().ok_or_else(|| ShapeError::ParseError {
                message: "nth_value requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            let n = extract_usize(&iter.next()).unwrap_or(1);
            Ok(WindowFunction::NthValue(Box::new(expr), n))
        }
        "sum" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "sum requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::Sum(Box::new(expr)))
        }
        "avg" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "avg requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::Avg(Box::new(expr)))
        }
        "min" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "min requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::Min(Box::new(expr)))
        }
        "max" => {
            let expr = args.into_iter().next().ok_or_else(|| ShapeError::ParseError {
                message: "max requires an expression argument".to_string(),
                location: Some(loc.clone()),
            })?;
            Ok(WindowFunction::Max(Box::new(expr)))
        }
        "count" => {
            let expr = args.into_iter().next().map(Box::new);
            Ok(WindowFunction::Count(expr))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unknown window function: '{}'", name),
            location: Some(
                loc.clone()
                    .with_hint("valid functions: lag, lead, row_number, rank, dense_rank, ntile, first_value, last_value, sum, avg, min, max, count"),
            ),
        }),
    }
}

/// Extract usize from an expression if it's a literal number
fn extract_usize(expr: &Option<Expr>) -> Option<usize> {
    match expr {
        Some(Expr::Literal(Literal::Number(n), _)) => Some(*n as usize),
        _ => None,
    }
}

/// Parse the OVER clause
///
/// Grammar: `"over" "(" window_spec? ")"`
fn parse_over_clause(pair: Pair<Rule>) -> Result<WindowSpec> {
    let mut partition_by = Vec::new();
    let mut order_by = None;
    let mut frame = None;

    // Look for window_spec inside over_clause
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::window_spec {
            for spec_part in inner.into_inner() {
                match spec_part.as_rule() {
                    Rule::partition_by_clause => {
                        partition_by = parse_partition_by_clause(spec_part)?;
                    }
                    Rule::order_by_clause => {
                        order_by = Some(parse_window_order_by(spec_part)?);
                    }
                    Rule::window_frame_clause => {
                        frame = Some(parse_window_frame_clause(spec_part)?);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(WindowSpec {
        partition_by,
        order_by,
        frame,
    })
}

/// Parse PARTITION BY clause
///
/// Grammar: `"partition" "by" expression ("," expression)*`
fn parse_partition_by_clause(pair: Pair<Rule>) -> Result<Vec<Expr>> {
    let mut exprs = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::expression {
            exprs.push(expressions::parse_expression(inner)?);
        }
    }
    Ok(exprs)
}

/// Parse ORDER BY clause for window functions
fn parse_window_order_by(pair: Pair<Rule>) -> Result<OrderByClause> {
    let mut columns = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::order_by_list {
            for item in inner.into_inner() {
                if item.as_rule() == Rule::order_by_item {
                    let mut item_inner = item.into_inner();

                    // Parse expression
                    let expr_pair = item_inner.next().ok_or_else(|| ShapeError::ParseError {
                        message: "expected expression in ORDER BY".to_string(),
                        location: None,
                    })?;
                    let expr = expressions::parse_expression(expr_pair)?;

                    // Parse optional direction
                    let direction = if let Some(dir_pair) = item_inner.next() {
                        match dir_pair.as_str().to_lowercase().as_str() {
                            "desc" => SortDirection::Descending,
                            _ => SortDirection::Ascending,
                        }
                    } else {
                        SortDirection::Ascending
                    };

                    columns.push((expr, direction));
                }
            }
        }
    }

    Ok(OrderByClause { columns })
}

/// Parse window frame clause
///
/// Grammar: `frame_type frame_extent`
fn parse_window_frame_clause(pair: Pair<Rule>) -> Result<WindowFrame> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse frame type (ROWS or RANGE)
    let frame_type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected frame type (ROWS or RANGE)".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let frame_type = match frame_type_pair.as_str().to_lowercase().as_str() {
        "rows" => WindowFrameType::Rows,
        "range" => WindowFrameType::Range,
        _ => WindowFrameType::Rows,
    };

    // Parse frame extent
    let extent_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected frame extent".to_string(),
        location: Some(pair_loc),
    })?;
    let (start, end) = parse_frame_extent(extent_pair)?;

    Ok(WindowFrame {
        frame_type,
        start,
        end,
    })
}

/// Parse frame extent
///
/// Grammar: `"between" frame_bound "and" frame_bound | frame_bound`
fn parse_frame_extent(pair: Pair<Rule>) -> Result<(WindowBound, WindowBound)> {
    let mut bounds = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::frame_bound {
            bounds.push(parse_frame_bound(inner)?);
        }
    }

    match bounds.len() {
        1 => {
            // Single bound means start..CURRENT ROW
            Ok((bounds.remove(0), WindowBound::CurrentRow))
        }
        2 => {
            // BETWEEN start AND end
            let end = bounds.remove(1);
            let start = bounds.remove(0);
            Ok((start, end))
        }
        _ => Ok((WindowBound::UnboundedPreceding, WindowBound::CurrentRow)),
    }
}

/// Parse a single frame bound
///
/// Grammar: `"unbounded" "preceding" | "current" "row" | integer "preceding" | integer "following" | "unbounded" "following"`
fn parse_frame_bound(pair: Pair<Rule>) -> Result<WindowBound> {
    let text = pair.as_str().to_lowercase();
    let parts: Vec<&str> = text.split_whitespace().collect();

    match parts.as_slice() {
        ["unbounded", "preceding"] => Ok(WindowBound::UnboundedPreceding),
        ["unbounded", "following"] => Ok(WindowBound::UnboundedFollowing),
        ["current", "row"] => Ok(WindowBound::CurrentRow),
        [n, "preceding"] => {
            let num = n.parse::<usize>().map_err(|_| ShapeError::ParseError {
                message: format!("invalid frame bound number: '{}'", n),
                location: Some(pair_location(&pair)),
            })?;
            Ok(WindowBound::Preceding(num))
        }
        [n, "following"] => {
            let num = n.parse::<usize>().map_err(|_| ShapeError::ParseError {
                message: format!("invalid frame bound number: '{}'", n),
                location: Some(pair_location(&pair)),
            })?;
            Ok(WindowBound::Following(num))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("invalid frame bound: '{}'", text),
            location: Some(
                pair_location(&pair)
                    .with_hint("use: UNBOUNDED PRECEDING, n PRECEDING, CURRENT ROW, n FOLLOWING, or UNBOUNDED FOLLOWING"),
            ),
        }),
    }
}

/// Parse window function from a regular function call that has an OVER clause
/// This is called when we detect a function call followed by OVER
pub fn parse_window_from_function_call(
    name: String,
    args: Vec<Expr>,
    over_pair: Pair<Rule>,
    span: crate::ast::Span,
) -> Result<Expr> {
    let window_spec = parse_over_clause(over_pair)?;
    let loc = SourceLocation::new(1, 1); // Placeholder location

    let function = build_window_function(&name.to_lowercase(), args, &loc)?;

    Ok(Expr::WindowExpr(
        Box::new(WindowExpr {
            function,
            over: window_spec,
        }),
        span,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pest::Parser;

    fn parse_window_func(input: &str) -> Result<Expr> {
        let pairs =
            crate::parser::ShapeParser::parse(Rule::window_function_call, input).map_err(|e| {
                ShapeError::ParseError {
                    message: format!("parse error: {}", e),
                    location: None,
                }
            })?;
        let pair = pairs.into_iter().next().unwrap();
        parse_window_function_call(pair)
    }

    #[test]
    fn test_row_number() {
        let result = parse_window_func("row_number() over ()");
        assert!(result.is_ok());
        if let Ok(Expr::WindowExpr(w, _)) = result {
            assert!(matches!(w.function, WindowFunction::RowNumber));
        }
    }

    #[test]
    fn test_lag_with_args() {
        let result = parse_window_func("lag(close, 1) over (order by timestamp)");
        assert!(result.is_ok());
        if let Ok(Expr::WindowExpr(w, _)) = result {
            assert!(matches!(w.function, WindowFunction::Lag { offset: 1, .. }));
            assert!(w.over.order_by.is_some());
        }
    }

    #[test]
    fn test_sum_with_partition() {
        let result = parse_window_func("sum(volume) over (partition by symbol)");
        assert!(result.is_ok());
        if let Ok(Expr::WindowExpr(w, _)) = result {
            assert!(matches!(w.function, WindowFunction::Sum(_)));
            assert_eq!(w.over.partition_by.len(), 1);
        }
    }

    #[test]
    fn test_avg_with_frame() {
        let result =
            parse_window_func("avg(close) over (rows between 5 preceding and current row)");
        assert!(result.is_ok());
        if let Ok(Expr::WindowExpr(w, _)) = result {
            assert!(matches!(w.function, WindowFunction::Avg(_)));
            assert!(w.over.frame.is_some());
            let frame = w.over.frame.unwrap();
            assert!(matches!(frame.start, WindowBound::Preceding(5)));
            assert!(matches!(frame.end, WindowBound::CurrentRow));
        }
    }
}
