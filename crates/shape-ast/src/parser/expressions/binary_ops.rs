//! Binary operator expression parsing
//!
//! This module handles parsing of all binary operators with proper precedence:
//! - Pipe operator (|>)
//! - Ternary operator (?:)
//! - Null coalescing (??)
//! - Error context wrapping (!!)
//! - Logical operators (||, &&)
//! - Comparison operators (>, <, >=, <=, ==, !=, ~=, ~>, ~<)
//! - Range operator (..)
//! - Arithmetic operators (+, -, *, /, %, ^)

use super::super::pair_span;
use crate::ast::operators::{FuzzyOp, FuzzyTolerance};
use crate::ast::{AssignExpr, BinaryOp, Expr, IfExpr, Literal, RangeKind, Span, UnaryOp};
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

/// Parse pipe expression (a |> b |> c)
/// Pipes the left value into the right function
pub fn parse_pipe_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in pipe".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_ternary_expr(first)?;

    // Chain pipe operations: left |> right |> more
    for ternary_pair in inner {
        let right = parse_ternary_expr(ternary_pair)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::Pipe,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse ternary expression (condition ? then : else)
pub fn parse_ternary_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition expression in ternary".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let condition_expr = parse_null_coalesce_expr(condition_pair)?;

    // Check if we have a ternary operator
    if let Some(then_pair) = inner.next() {
        // We have ? expr : expr
        let then_expr = parse_ternary_branch(then_pair)?;
        let else_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected else expression after ':' in ternary".to_string(),
            location: Some(pair_loc),
        })?;
        let else_expr = parse_ternary_branch(else_pair)?;

        Ok(Expr::If(
            Box::new(IfExpr {
                condition: Box::new(condition_expr),
                then_branch: Box::new(then_expr),
                else_branch: Some(Box::new(else_expr)),
            }),
            span,
        ))
    } else {
        // No ternary, just return the null_coalesce_expr
        Ok(condition_expr)
    }
}

fn parse_ternary_branch(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::ternary_branch => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression in ternary branch".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_ternary_expr_no_range(inner)
        }
        Rule::ternary_expr_no_range => parse_ternary_expr_no_range(pair),
        Rule::assignment_expr_no_range => parse_assignment_expr_no_range(pair),
        _ => super::primary::parse_expression(pair),
    }
}

/// Parse ternary expression in no-range context (used inside ternary branches
/// for right-associative nesting: `a ? b : c ? d : e`).
fn parse_ternary_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition expression in ternary".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let condition_expr = parse_null_coalesce_expr_no_range(condition_pair)?;

    if let Some(then_pair) = inner.next() {
        let then_expr = parse_ternary_branch(then_pair)?;
        let else_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected else expression after ':' in ternary".to_string(),
            location: Some(pair_loc),
        })?;
        let else_expr = parse_ternary_branch(else_pair)?;

        Ok(Expr::If(
            Box::new(IfExpr {
                condition: Box::new(condition_expr),
                then_branch: Box::new(then_expr),
                else_branch: Some(Box::new(else_expr)),
            }),
            span,
        ))
    } else {
        Ok(condition_expr)
    }
}

/// Map compound assignment operator string to BinaryOp
fn compound_op_to_binary(op_str: &str) -> Option<BinaryOp> {
    match op_str {
        "+=" => Some(BinaryOp::Add),
        "-=" => Some(BinaryOp::Sub),
        "*=" => Some(BinaryOp::Mul),
        "/=" => Some(BinaryOp::Div),
        "%=" => Some(BinaryOp::Mod),
        "**=" => Some(BinaryOp::Pow),
        "^=" => Some(BinaryOp::BitXor),
        "&=" => Some(BinaryOp::BitAnd),
        "|=" => Some(BinaryOp::BitOr),
        "<<=" => Some(BinaryOp::BitShl),
        ">>=" => Some(BinaryOp::BitShr),
        _ => None,
    }
}

/// Parse assignment expression (target = value or target += value)
pub fn parse_assignment_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    if let Some(second) = inner.next() {
        // Check if second pair is a compound_assign_op
        if second.as_rule() == Rule::compound_assign_op {
            let target = super::primary::parse_postfix_expr(first)?;
            if !matches!(
                target,
                Expr::Identifier(_, _) | Expr::PropertyAccess { .. } | Expr::IndexAccess { .. }
            ) {
                return Err(ShapeError::ParseError {
                    message: "invalid assignment target".to_string(),
                    location: Some(pair_loc),
                });
            }
            let bin_op =
                compound_op_to_binary(second.as_str()).ok_or_else(|| ShapeError::ParseError {
                    message: format!("Unknown compound operator: {}", second.as_str()),
                    location: None,
                })?;
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value after compound assignment".to_string(),
                location: None,
            })?;
            let value = parse_assignment_expr(value_pair)?;
            // Desugar: x += v → x = x + v
            let desugared = Expr::BinaryOp {
                left: Box::new(target.clone()),
                op: bin_op,
                right: Box::new(value),
                span,
            };
            Ok(Expr::Assign(
                Box::new(AssignExpr {
                    target: Box::new(target),
                    value: Box::new(desugared),
                }),
                span,
            ))
        } else if second.as_rule() == Rule::assign_op {
            // Plain assignment: target assign_op value
            let target = super::primary::parse_postfix_expr(first)?;
            if !matches!(
                target,
                Expr::Identifier(_, _) | Expr::PropertyAccess { .. } | Expr::IndexAccess { .. }
            ) {
                return Err(ShapeError::ParseError {
                    message: "invalid assignment target".to_string(),
                    location: Some(pair_loc),
                });
            }
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value after assignment".to_string(),
                location: None,
            })?;
            let value = parse_assignment_expr(value_pair)?;
            Ok(Expr::Assign(
                Box::new(AssignExpr {
                    target: Box::new(target),
                    value: Box::new(value),
                }),
                span,
            ))
        } else {
            // Fallback: parse as pipe expression
            match first.as_rule() {
                Rule::pipe_expr => parse_pipe_expr(first),
                Rule::ternary_expr => parse_ternary_expr(first),
                _ => parse_pipe_expr(first),
            }
        }
    } else {
        // Check if this is a pipe_expr rule
        match first.as_rule() {
            Rule::pipe_expr => parse_pipe_expr(first),
            Rule::ternary_expr => parse_ternary_expr(first),
            _ => parse_pipe_expr(first),
        }
    }
}

fn parse_assignment_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    if let Some(second) = inner.next() {
        if second.as_rule() == Rule::compound_assign_op {
            let target = super::primary::parse_postfix_expr(first)?;
            if !matches!(
                target,
                Expr::Identifier(_, _) | Expr::PropertyAccess { .. } | Expr::IndexAccess { .. }
            ) {
                return Err(ShapeError::ParseError {
                    message: "invalid assignment target".to_string(),
                    location: Some(pair_loc),
                });
            }
            let bin_op =
                compound_op_to_binary(second.as_str()).ok_or_else(|| ShapeError::ParseError {
                    message: format!("Unknown compound operator: {}", second.as_str()),
                    location: None,
                })?;
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value after compound assignment".to_string(),
                location: None,
            })?;
            let value = parse_assignment_expr_no_range(value_pair)?;
            let desugared = Expr::BinaryOp {
                left: Box::new(target.clone()),
                op: bin_op,
                right: Box::new(value),
                span,
            };
            Ok(Expr::Assign(
                Box::new(AssignExpr {
                    target: Box::new(target),
                    value: Box::new(desugared),
                }),
                span,
            ))
        } else if second.as_rule() == Rule::assign_op {
            // Plain assignment: target assign_op value
            let target = super::primary::parse_postfix_expr(first)?;
            if !matches!(
                target,
                Expr::Identifier(_, _) | Expr::PropertyAccess { .. } | Expr::IndexAccess { .. }
            ) {
                return Err(ShapeError::ParseError {
                    message: "invalid assignment target".to_string(),
                    location: Some(pair_loc),
                });
            }
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value after assignment".to_string(),
                location: None,
            })?;
            let value = parse_assignment_expr_no_range(value_pair)?;
            Ok(Expr::Assign(
                Box::new(AssignExpr {
                    target: Box::new(target),
                    value: Box::new(value),
                }),
                span,
            ))
        } else {
            // Fallback: parse as null coalesce expression
            parse_null_coalesce_expr_no_range(first)
        }
    } else {
        parse_null_coalesce_expr_no_range(first)
    }
}

/// Parse null coalescing expression (a ?? b)
pub fn parse_null_coalesce_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in null coalesce".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_context_expr(first)?;

    for context_expr in inner {
        let right = parse_context_expr(context_expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::NullCoalesce,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_null_coalesce_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_context_expr_no_range(first)?;

    for context_expr in inner {
        let right = parse_context_expr_no_range(context_expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::NullCoalesce,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse error context expression (lhs !! rhs).
pub fn parse_context_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in error context".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_or_expr(first)?;

    for or_expr in inner {
        let rhs_source = or_expr.as_str().trim().to_string();
        let right = parse_or_expr(or_expr)?;
        let is_grouped_rhs = rhs_source.starts_with('(') && rhs_source.ends_with(')');

        match right {
            Expr::TryOperator(inner_try, try_span) if !is_grouped_rhs => {
                // Ergonomic special-case: `lhs !! rhs?` means `(lhs !! rhs)?`.
                // Use explicit parentheses for `lhs !! (rhs?)`.
                let context_expr = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOp::ErrorContext,
                    right: inner_try,
                    span,
                };
                left = Expr::TryOperator(Box::new(context_expr), try_span);
            }
            right => {
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOp::ErrorContext,
                    right: Box::new(right),
                    span,
                };
            }
        }
    }

    Ok(left)
}

fn parse_context_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_or_expr_no_range(first)?;

    for or_expr in inner {
        let rhs_source = or_expr.as_str().trim().to_string();
        let right = parse_or_expr_no_range(or_expr)?;
        let is_grouped_rhs = rhs_source.starts_with('(') && rhs_source.ends_with(')');

        match right {
            Expr::TryOperator(inner_try, try_span) if !is_grouped_rhs => {
                // Keep context + try ergonomic in ternary branches too.
                let context_expr = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOp::ErrorContext,
                    right: inner_try,
                    span,
                };
                left = Expr::TryOperator(Box::new(context_expr), try_span);
            }
            right => {
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: BinaryOp::ErrorContext,
                    right: Box::new(right),
                    span,
                };
            }
        }
    }

    Ok(left)
}

/// Parse logical OR expression (a || b)
pub fn parse_or_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in logical OR".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_and_expr(first)?;

    for and_expr in inner {
        let right = parse_and_expr(and_expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::Or,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_or_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_and_expr_no_range(first)?;

    for and_expr in inner {
        let right = parse_and_expr_no_range(and_expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::Or,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse logical AND expression (a && b)
pub fn parse_and_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in logical AND".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_or_expr(first)?;

    for expr in inner {
        let right = parse_bitwise_or_expr(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::And,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_and_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_or_expr_no_range(first)?;

    for expr in inner {
        let right = parse_bitwise_or_expr_no_range(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::And,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse bitwise OR expression (a | b)
fn parse_bitwise_or_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in bitwise OR".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_xor_expr(first)?;

    for expr in inner {
        let right = parse_bitwise_xor_expr(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitOr,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_bitwise_or_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_xor_expr_no_range(first)?;

    for expr in inner {
        let right = parse_bitwise_xor_expr_no_range(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitOr,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse bitwise XOR expression (a ^ b)
fn parse_bitwise_xor_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in bitwise XOR".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_and_expr(first)?;

    for expr in inner {
        let right = parse_bitwise_and_expr(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitXor,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_bitwise_xor_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_bitwise_and_expr_no_range(first)?;

    for expr in inner {
        let right = parse_bitwise_and_expr_no_range(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitXor,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse bitwise AND expression (a & b)
fn parse_bitwise_and_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in bitwise AND".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_comparison_expr(first)?;

    for expr in inner {
        let right = parse_comparison_expr(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitAnd,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

fn parse_bitwise_and_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_comparison_expr_no_range(first)?;

    for expr in inner {
        let right = parse_comparison_expr_no_range(expr)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::BitAnd,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse comparison expression (a > b, a == b, etc.)
pub fn parse_comparison_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in comparison".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_range_expr(first)?;

    for tail in inner {
        left = apply_comparison_tail(left, tail, span, parse_range_expr)?;
    }

    Ok(left)
}

fn parse_comparison_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_additive_expr(first)?;

    for tail in inner {
        left = apply_comparison_tail(left, tail, span, parse_additive_expr)?;
    }

    Ok(left)
}

fn apply_comparison_tail<F>(left: Expr, tail: Pair<Rule>, span: Span, parse_rhs: F) -> Result<Expr>
where
    F: Fn(Pair<Rule>) -> Result<Expr>,
{
    let mut tail_inner = tail.into_inner();
    let first = tail_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Empty comparison tail".to_string(),
        location: None,
    })?;

    match first.as_rule() {
        Rule::fuzzy_comparison_tail | Rule::fuzzy_comparison_tail_no_range => {
            // Parse fuzzy_comparison_tail: fuzzy_op ~ range_expr ~ within_clause?
            let mut fuzzy_inner = first.into_inner();

            let fuzzy_op_pair = fuzzy_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "Fuzzy comparison missing operator".to_string(),
                location: None,
            })?;
            let op = parse_fuzzy_op(fuzzy_op_pair)?;

            let rhs_pair = fuzzy_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "Fuzzy comparison missing right-hand side".to_string(),
                location: None,
            })?;
            let right = parse_rhs(rhs_pair)?;

            // Parse optional within_clause
            let tolerance = if let Some(within_clause) = fuzzy_inner.next() {
                parse_within_clause(within_clause)?
            } else {
                // Default to 2% tolerance if no explicit tolerance specified
                FuzzyTolerance::Percentage(0.02)
            };

            Ok(Expr::FuzzyComparison {
                left: Box::new(left),
                op,
                right: Box::new(right),
                tolerance,
                span,
            })
        }
        Rule::comparison_op => {
            let op = parse_comparison_op(first)?;
            let rhs_pair = tail_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "Comparison operator missing right-hand side".to_string(),
                location: None,
            })?;
            let right = parse_rhs(rhs_pair)?;
            Ok(Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            })
        }
        Rule::type_annotation => {
            let type_annotation = crate::parser::parse_type_annotation(first)?;
            Ok(Expr::InstanceOf {
                expr: Box::new(left),
                type_annotation,
                span,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected comparison tail: {:?}", first.as_rule()),
            location: None,
        }),
    }
}

/// Parse fuzzy operator (~=, ~<, ~>)
fn parse_fuzzy_op(pair: Pair<Rule>) -> Result<FuzzyOp> {
    match pair.as_str() {
        "~=" => Ok(FuzzyOp::Equal),
        "~>" => Ok(FuzzyOp::Greater),
        "~<" => Ok(FuzzyOp::Less),
        _ => Err(ShapeError::ParseError {
            message: format!("Unknown fuzzy operator: {}", pair.as_str()),
            location: None,
        }),
    }
}

/// Parse within_clause: "within" ~ tolerance_spec
fn parse_within_clause(pair: Pair<Rule>) -> Result<FuzzyTolerance> {
    let mut inner = pair.into_inner();
    let tolerance_spec = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Within clause missing tolerance value".to_string(),
        location: None,
    })?;
    parse_tolerance_spec(tolerance_spec)
}

/// Parse tolerance_spec: number ~ "%"?
fn parse_tolerance_spec(pair: Pair<Rule>) -> Result<FuzzyTolerance> {
    let text = pair.as_str().trim();

    if text.ends_with('%') {
        // Percentage tolerance: "2%" or "0.5%"
        let num_str = text.trim_end_matches('%');
        let value: f64 = num_str.parse().map_err(|_| ShapeError::ParseError {
            message: format!("Invalid tolerance percentage: {}", text),
            location: None,
        })?;
        // Convert percentage to fraction (e.g., 2% -> 0.02)
        Ok(FuzzyTolerance::Percentage(value / 100.0))
    } else {
        // Absolute tolerance: "0.02" or "5"
        let value: f64 = text.parse().map_err(|_| ShapeError::ParseError {
            message: format!("Invalid tolerance value: {}", text),
            location: None,
        })?;
        Ok(FuzzyTolerance::Absolute(value))
    }
}

/// Parse range expression (a..b)
/// Parse a range expression with Rust-style syntax
/// Supports: start..end, start..=end, ..end, ..=end, start.., ..
pub fn parse_range_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner().peekable();

    // Check if first token is a range_op (for ..end, ..=end, or .. forms)
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in range".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    match first.as_rule() {
        Rule::range_op => {
            // Forms: ..end, ..=end, or .. (full range)
            let kind = parse_range_op(&first);
            if let Some(end_pair) = inner.next() {
                // ..end or ..=end
                let end = parse_additive_expr(end_pair)?;
                Ok(Expr::Range {
                    start: None,
                    end: Some(Box::new(end)),
                    kind,
                    span,
                })
            } else {
                // Full range: ..
                Ok(Expr::Range {
                    start: None,
                    end: None,
                    kind,
                    span,
                })
            }
        }
        Rule::additive_expr => {
            // Forms: start..end, start..=end, start.., or just expr
            let start = parse_additive_expr(first)?;

            if let Some(next) = inner.next() {
                match next.as_rule() {
                    Rule::range_op => {
                        // start..end or start..=end or start..
                        let kind = parse_range_op(&next);
                        if let Some(end_pair) = inner.next() {
                            // start..end or start..=end
                            let end = parse_additive_expr(end_pair)?;
                            Ok(Expr::Range {
                                start: Some(Box::new(start)),
                                end: Some(Box::new(end)),
                                kind,
                                span,
                            })
                        } else {
                            // start.. (range from)
                            Ok(Expr::Range {
                                start: Some(Box::new(start)),
                                end: None,
                                kind,
                                span,
                            })
                        }
                    }
                    _ => {
                        // Unexpected token after start expression
                        Err(ShapeError::ParseError {
                            message: format!(
                                "unexpected token in range expression: {:?}",
                                next.as_rule()
                            ),
                            location: Some(pair_loc),
                        })
                    }
                }
            } else {
                // Just a single expression (not a range)
                Ok(start)
            }
        }
        _ => {
            // Try to parse as additive_expr anyway (fallback)
            parse_additive_expr(first)
        }
    }
}

/// Parse range operator and return RangeKind
fn parse_range_op(pair: &Pair<Rule>) -> RangeKind {
    if pair.as_str() == "..=" {
        RangeKind::Inclusive
    } else {
        RangeKind::Exclusive
    }
}

/// Parse comparison operator
pub fn parse_comparison_op(pair: Pair<Rule>) -> Result<BinaryOp> {
    match pair.as_str() {
        ">" => Ok(BinaryOp::Greater),
        "<" => Ok(BinaryOp::Less),
        ">=" => Ok(BinaryOp::GreaterEq),
        "<=" => Ok(BinaryOp::LessEq),
        "==" => Ok(BinaryOp::Equal),
        "!=" => Ok(BinaryOp::NotEqual),
        "~=" => Ok(BinaryOp::FuzzyEqual),
        "~>" => Ok(BinaryOp::FuzzyGreater),
        "~<" => Ok(BinaryOp::FuzzyLess),
        _ => Err(ShapeError::ParseError {
            message: format!("Unknown comparison operator: {}", pair.as_str()),
            location: None,
        }),
    }
}

/// Parse additive expression (a + b, a - b)
pub fn parse_additive_expr(pair: Pair<Rule>) -> Result<Expr> {
    // In Pest, the entire additive_expr contains the full string
    // We need to parse it by extracting operators from the original string
    let span = pair_span(&pair);
    let expr_str = pair.as_str();
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Empty additive expression".to_string(),
            location: None,
        });
    }

    // Parse the first shift expression
    let mut left = parse_shift_expr(inner_pairs[0].clone())?;

    // If there's only one pair, no operators
    if inner_pairs.len() == 1 {
        return Ok(left);
    }

    // For expressions with operators, we need to find operators in the original string
    // between the shift expressions
    let mut current_pos = inner_pairs[0].as_str().len();

    for i in 1..inner_pairs.len() {
        // Find the operator between previous and current expression
        let expr_start = expr_str[current_pos..]
            .find(inner_pairs[i].as_str())
            .ok_or_else(|| ShapeError::ParseError {
                message: "Cannot find expression in string".to_string(),
                location: None,
            })?;
        let op_str = expr_str[current_pos..current_pos + expr_start].trim();

        let right = parse_shift_expr(inner_pairs[i].clone())?;

        left = Expr::BinaryOp {
            left: Box::new(left),
            op: match op_str {
                "+" => BinaryOp::Add,
                "-" => BinaryOp::Sub,
                _ => {
                    return Err(ShapeError::ParseError {
                        message: format!("Unknown additive operator: '{}'", op_str),
                        location: None,
                    });
                }
            },
            right: Box::new(right),
            span,
        };

        current_pos += expr_start + inner_pairs[i].as_str().len();
    }

    Ok(left)
}

/// Parse shift expression (a << b, a >> b)
pub fn parse_shift_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let expr_str = pair.as_str();
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Empty shift expression".to_string(),
            location: None,
        });
    }

    let mut left = parse_multiplicative_expr(inner_pairs[0].clone())?;

    if inner_pairs.len() == 1 {
        return Ok(left);
    }

    let mut current_pos = inner_pairs[0].as_str().len();

    for i in 1..inner_pairs.len() {
        let expr_start = expr_str[current_pos..]
            .find(inner_pairs[i].as_str())
            .ok_or_else(|| ShapeError::ParseError {
                message: "Cannot find expression in string".to_string(),
                location: None,
            })?;
        let op_str = expr_str[current_pos..current_pos + expr_start].trim();

        let right = parse_multiplicative_expr(inner_pairs[i].clone())?;

        left = Expr::BinaryOp {
            left: Box::new(left),
            op: match op_str {
                "<<" => BinaryOp::BitShl,
                ">>" => BinaryOp::BitShr,
                _ => {
                    return Err(ShapeError::ParseError {
                        message: format!("Unknown shift operator: '{}'", op_str),
                        location: None,
                    });
                }
            },
            right: Box::new(right),
            span,
        };

        current_pos += expr_start + inner_pairs[i].as_str().len();
    }

    Ok(left)
}

/// Parse multiplicative expression (a * b, a / b, a % b)
pub fn parse_multiplicative_expr(pair: Pair<Rule>) -> Result<Expr> {
    // Similar to additive_expr, we need to extract operators from the original string
    let span = pair_span(&pair);
    let expr_str = pair.as_str();
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Empty multiplicative expression".to_string(),
            location: None,
        });
    }

    // Parse the first exponential expression
    let mut left = parse_exponential_expr(inner_pairs[0].clone())?;

    // If there's only one pair, no operators
    if inner_pairs.len() == 1 {
        return Ok(left);
    }

    // For expressions with operators, we need to find operators in the original string
    // between the unary expressions
    let mut current_pos = inner_pairs[0].as_str().len();

    for i in 1..inner_pairs.len() {
        // Find the operator between previous and current expression
        let expr_start = expr_str[current_pos..]
            .find(inner_pairs[i].as_str())
            .ok_or_else(|| ShapeError::ParseError {
                message: "Cannot find expression in string".to_string(),
                location: None,
            })?;
        let op_str = expr_str[current_pos..current_pos + expr_start].trim();

        let right = parse_exponential_expr(inner_pairs[i].clone())?;

        left = Expr::BinaryOp {
            left: Box::new(left),
            op: match op_str {
                "*" => BinaryOp::Mul,
                "/" => BinaryOp::Div,
                "%" => BinaryOp::Mod,
                _ => {
                    return Err(ShapeError::ParseError {
                        message: format!("Unknown multiplicative operator: '{}'", op_str),
                        location: None,
                    });
                }
            },
            right: Box::new(right),
            span,
        };

        current_pos += expr_start + inner_pairs[i].as_str().len();
    }

    Ok(left)
}

/// Parse exponential expression (a ** b)
pub fn parse_exponential_expr(pair: Pair<Rule>) -> Result<Expr> {
    // Exponentiation is right-associative, so we need to parse differently
    let span = pair_span(&pair);
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Empty exponential expression".to_string(),
            location: None,
        });
    }

    // Parse all unary expressions
    let mut exprs: Vec<Expr> = Vec::new();
    for p in inner_pairs {
        exprs.push(parse_unary_expr(p)?);
    }

    // If there's only one expression, return it
    if exprs.len() == 1 {
        return Ok(exprs.into_iter().next().unwrap());
    }

    // For right-associative parsing, we build from right to left
    // Example: a ** b ** c should be parsed as a ** (b ** c)
    let mut result = exprs.pop().unwrap(); // Start with the rightmost expression

    while let Some(left_expr) = exprs.pop() {
        result = Expr::BinaryOp {
            left: Box::new(left_expr),
            op: BinaryOp::Pow,
            right: Box::new(result),
            span,
        };
    }

    Ok(result)
}

/// Parse unary expression (!a, -a)
pub fn parse_unary_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_str = pair.as_str();
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in unary operation".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    // Check if this is a ref_expr rule (& or &mut prefix)
    if first.as_rule() == Rule::ref_expr {
        let ref_span = pair_span(&first);
        let ref_inner = first.into_inner();
        let mut is_mutable = false;
        let mut expr_pair = None;
        for child in ref_inner {
            match child.as_rule() {
                Rule::ref_mut_keyword => {
                    is_mutable = true;
                }
                _ => {
                    // The postfix_expr (the referenced expression)
                    expr_pair = Some(child);
                }
            }
        }
        let inner_expr = expr_pair.ok_or_else(|| ShapeError::ParseError {
            message: "expected expression after &".to_string(),
            location: Some(pair_loc),
        })?;
        let operand = super::primary::parse_postfix_expr(inner_expr)?;
        return Ok(Expr::Reference {
            expr: Box::new(operand),
            is_mutable,
            span: ref_span,
        });
    }

    // Check if this unary expression starts with an operator
    if pair_str.starts_with('!') {
        Ok(Expr::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(parse_unary_expr(first)?),
            span,
        })
    } else if pair_str.starts_with('~') {
        Ok(Expr::UnaryOp {
            op: UnaryOp::BitNot,
            operand: Box::new(parse_unary_expr(first)?),
            span,
        })
    } else if pair_str.starts_with('-') {
        let operand = parse_unary_expr(first)?;
        // Fold negation into typed integer literals so that `-128i8` parses
        // as a single `TypedInt(-128, I8)` instead of `Neg(TypedInt(128, I8))`.
        // Without this, 128i8 would be out of range and rejected at parse time.
        match &operand {
            Expr::Literal(Literal::TypedInt(value, width), lit_span) => {
                let neg = value.wrapping_neg();
                if width.in_range_i64(neg) {
                    return Ok(Expr::Literal(Literal::TypedInt(neg, *width), *lit_span));
                }
            }
            Expr::Literal(Literal::Int(value), lit_span) => {
                return Ok(Expr::Literal(Literal::Int(-value), *lit_span));
            }
            Expr::Literal(Literal::Number(value), lit_span) => {
                return Ok(Expr::Literal(Literal::Number(-value), *lit_span));
            }
            _ => {}
        }
        Ok(Expr::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(operand),
            span,
        })
    } else {
        // No unary operator, parse as postfix expression
        super::primary::parse_postfix_expr(first)
    }
}
