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

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

/// Parse a left-associative binary chain: `first (op second)*`.
///
/// The Pest rule emits a flat list of children that are all the same sub-rule
/// (the operators are implicit).  `parse_child` is called for every child and
/// `op` is the `BinaryOp` that joins them.
fn parse_binary_chain(
    pair: Pair<Rule>,
    error_ctx: &str,
    op: BinaryOp,
    parse_child: fn(Pair<Rule>) -> Result<Expr>,
) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: format!("expected expression in {}", error_ctx),
        location: Some(pair_loc),
    })?;
    let mut left = parse_child(first)?;

    for child in inner {
        let right = parse_child(child)?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span,
        };
    }

    Ok(left)
}

/// Parse an expression that uses string-position-based operator extraction.
///
/// This covers `additive_expr`, `shift_expr`, and `multiplicative_expr` where
/// the Pest grammar emits only the operand sub-rules (no explicit operator
/// pairs) and the operators must be recovered from the raw source text between
/// operand spans.
fn parse_positional_op_chain(
    pair: Pair<Rule>,
    error_ctx: &str,
    parse_child: fn(Pair<Rule>) -> Result<Expr>,
    resolve_op: fn(&str) -> Result<BinaryOp>,
) -> Result<Expr> {
    let span = pair_span(&pair);
    let expr_str = pair.as_str();
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: format!("Empty {} expression", error_ctx),
            location: None,
        });
    }

    let mut left = parse_child(inner_pairs[0].clone())?;

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
        let op = resolve_op(op_str)?;
        let right = parse_child(inner_pairs[i].clone())?;

        left = Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span,
        };

        current_pos += expr_start + inner_pairs[i].as_str().len();
    }

    Ok(left)
}

// ---------------------------------------------------------------------------
// Precedence-level dispatch helpers (range / no-range)
// ---------------------------------------------------------------------------

/// The precedence chain is:
///
///   null_coalesce -> context -> or -> and -> bitwise_or -> bitwise_xor
///     -> bitwise_and -> comparison -> [range ->] additive -> shift
///     -> multiplicative -> exponential -> unary
///
/// The only difference between the range and no-range chains is that
/// comparison delegates to `parse_range_expr` (which then delegates to
/// additive) when ranges are allowed, and directly to `parse_additive_expr`
/// when they are not.

fn select_null_coalesce(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_null_coalesce_expr } else { parse_null_coalesce_expr_no_range }
}
fn child_of_null_coalesce(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_context_expr } else { parse_context_expr_no_range }
}
fn child_of_context(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_or_expr } else { parse_or_expr_no_range }
}
fn child_of_or(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_and_expr } else { parse_and_expr_no_range }
}
fn child_of_and(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_bitwise_or_expr } else { parse_bitwise_or_expr_no_range }
}
fn child_of_bitwise_or(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_bitwise_xor_expr } else { parse_bitwise_xor_expr_no_range }
}
fn child_of_bitwise_xor(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_bitwise_and_expr } else { parse_bitwise_and_expr_no_range }
}
fn child_of_bitwise_and(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_comparison_expr } else { parse_comparison_expr_no_range }
}
fn child_of_comparison(allow_range: bool) -> fn(Pair<Rule>) -> Result<Expr> {
    if allow_range { parse_range_expr } else { parse_additive_expr }
}

// ---------------------------------------------------------------------------
// Pipe (not duplicated -- ranges are allowed in pipe context)
// ---------------------------------------------------------------------------

/// Parse pipe expression (a |> b |> c)
/// Pipes the left value into the right function
pub fn parse_pipe_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "pipe", BinaryOp::Pipe, parse_ternary_expr)
}

// ---------------------------------------------------------------------------
// Ternary (condition ? then : else)
// ---------------------------------------------------------------------------

/// Parse ternary expression (condition ? then : else)
pub fn parse_ternary_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_ternary_impl(pair, true)
}

fn parse_ternary_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_ternary_impl(pair, false)
}

fn parse_ternary_impl(pair: Pair<Rule>, allow_range: bool) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition expression in ternary".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let condition_expr = (select_null_coalesce(allow_range))(condition_pair)?;

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

// ---------------------------------------------------------------------------
// Assignment (target = value, target += value)
// ---------------------------------------------------------------------------

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
    parse_assignment_impl(pair, true)
}

fn parse_assignment_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_assignment_impl(pair, false)
}

fn parse_assignment_impl(pair: Pair<Rule>, allow_range: bool) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    let recurse: fn(Pair<Rule>) -> Result<Expr> = if allow_range {
        parse_assignment_expr
    } else {
        parse_assignment_expr_no_range
    };

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
            let value = recurse(value_pair)?;
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
            let value = recurse(value_pair)?;
            Ok(Expr::Assign(
                Box::new(AssignExpr {
                    target: Box::new(target),
                    value: Box::new(value),
                }),
                span,
            ))
        } else if allow_range {
            match first.as_rule() {
                Rule::pipe_expr => parse_pipe_expr(first),
                Rule::ternary_expr => parse_ternary_expr(first),
                _ => parse_pipe_expr(first),
            }
        } else {
            (select_null_coalesce(false))(first)
        }
    } else if allow_range {
        match first.as_rule() {
            Rule::pipe_expr => parse_pipe_expr(first),
            Rule::ternary_expr => parse_ternary_expr(first),
            _ => parse_pipe_expr(first),
        }
    } else {
        (select_null_coalesce(false))(first)
    }
}

// ---------------------------------------------------------------------------
// Null coalescing (a ?? b)
// ---------------------------------------------------------------------------

/// Parse null coalescing expression (a ?? b)
pub fn parse_null_coalesce_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "null coalesce", BinaryOp::NullCoalesce, child_of_null_coalesce(true))
}

fn parse_null_coalesce_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "null coalesce", BinaryOp::NullCoalesce, child_of_null_coalesce(false))
}

// ---------------------------------------------------------------------------
// Error context (a !! b) -- special TryOperator handling
// ---------------------------------------------------------------------------

/// Parse error context expression (lhs !! rhs).
pub fn parse_context_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_context_impl(pair, true)
}

fn parse_context_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_context_impl(pair, false)
}

fn parse_context_impl(pair: Pair<Rule>, allow_range: bool) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let parse_child = child_of_context(allow_range);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in error context".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_child(first)?;

    for or_expr in inner {
        let rhs_source = or_expr.as_str().trim().to_string();
        let right = parse_child(or_expr)?;
        let is_grouped_rhs = rhs_source.starts_with('(') && rhs_source.ends_with(')');

        match right {
            Expr::TryOperator(inner_try, try_span) if !is_grouped_rhs => {
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

// ---------------------------------------------------------------------------
// Logical OR / AND, Bitwise OR / XOR / AND
// ---------------------------------------------------------------------------

/// Parse logical OR expression (a || b)
pub fn parse_or_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "logical OR", BinaryOp::Or, child_of_or(true))
}
fn parse_or_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "logical OR", BinaryOp::Or, child_of_or(false))
}

/// Parse logical AND expression (a && b)
pub fn parse_and_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "logical AND", BinaryOp::And, child_of_and(true))
}
fn parse_and_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "logical AND", BinaryOp::And, child_of_and(false))
}

/// Parse bitwise OR expression (a | b)
fn parse_bitwise_or_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise OR", BinaryOp::BitOr, child_of_bitwise_or(true))
}
fn parse_bitwise_or_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise OR", BinaryOp::BitOr, child_of_bitwise_or(false))
}

/// Parse bitwise XOR expression (a ^ b)
fn parse_bitwise_xor_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise XOR", BinaryOp::BitXor, child_of_bitwise_xor(true))
}
fn parse_bitwise_xor_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise XOR", BinaryOp::BitXor, child_of_bitwise_xor(false))
}

/// Parse bitwise AND expression (a & b)
fn parse_bitwise_and_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise AND", BinaryOp::BitAnd, child_of_bitwise_and(true))
}
fn parse_bitwise_and_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_binary_chain(pair, "bitwise AND", BinaryOp::BitAnd, child_of_bitwise_and(false))
}

// ---------------------------------------------------------------------------
// Comparison (>, <, >=, <=, ==, !=, ~=, ~>, ~<, is)
// ---------------------------------------------------------------------------

/// Parse comparison expression (a > b, a == b, etc.)
pub fn parse_comparison_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_comparison_impl(pair, true)
}

fn parse_comparison_expr_no_range(pair: Pair<Rule>) -> Result<Expr> {
    parse_comparison_impl(pair, false)
}

fn parse_comparison_impl(pair: Pair<Rule>, allow_range: bool) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let parse_child = child_of_comparison(allow_range);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in comparison".to_string(),
        location: Some(pair_loc),
    })?;
    let mut left = parse_child(first)?;

    for tail in inner {
        left = apply_comparison_tail(left, tail, span, parse_child)?;
    }

    Ok(left)
}

fn apply_comparison_tail(
    left: Expr,
    tail: Pair<Rule>,
    span: Span,
    parse_rhs: fn(Pair<Rule>) -> Result<Expr>,
) -> Result<Expr> {
    let mut tail_inner = tail.into_inner();
    let first = tail_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Empty comparison tail".to_string(),
        location: None,
    })?;

    match first.as_rule() {
        Rule::fuzzy_comparison_tail | Rule::fuzzy_comparison_tail_no_range => {
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

            let tolerance = if let Some(within_clause) = fuzzy_inner.next() {
                parse_within_clause(within_clause)?
            } else {
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
        let num_str = text.trim_end_matches('%');
        let value: f64 = num_str.parse().map_err(|_| ShapeError::ParseError {
            message: format!("Invalid tolerance percentage: {}", text),
            location: None,
        })?;
        Ok(FuzzyTolerance::Percentage(value / 100.0))
    } else {
        let value: f64 = text.parse().map_err(|_| ShapeError::ParseError {
            message: format!("Invalid tolerance value: {}", text),
            location: None,
        })?;
        Ok(FuzzyTolerance::Absolute(value))
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

// ---------------------------------------------------------------------------
// Range (a..b, a..=b, ..b, ..=b, a.., ..)
// ---------------------------------------------------------------------------

/// Parse a range expression with Rust-style syntax.
/// Supports: start..end, start..=end, ..end, ..=end, start.., ..
pub fn parse_range_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner().peekable();

    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in range".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    match first.as_rule() {
        Rule::range_op => {
            let kind = parse_range_op(&first);
            if let Some(end_pair) = inner.next() {
                let end = parse_additive_expr(end_pair)?;
                Ok(Expr::Range { start: None, end: Some(Box::new(end)), kind, span })
            } else {
                Ok(Expr::Range { start: None, end: None, kind, span })
            }
        }
        Rule::additive_expr => {
            let start = parse_additive_expr(first)?;
            if let Some(next) = inner.next() {
                match next.as_rule() {
                    Rule::range_op => {
                        let kind = parse_range_op(&next);
                        if let Some(end_pair) = inner.next() {
                            let end = parse_additive_expr(end_pair)?;
                            Ok(Expr::Range {
                                start: Some(Box::new(start)),
                                end: Some(Box::new(end)),
                                kind,
                                span,
                            })
                        } else {
                            Ok(Expr::Range {
                                start: Some(Box::new(start)),
                                end: None,
                                kind,
                                span,
                            })
                        }
                    }
                    _ => Err(ShapeError::ParseError {
                        message: format!(
                            "unexpected token in range expression: {:?}",
                            next.as_rule()
                        ),
                        location: Some(pair_loc),
                    }),
                }
            } else {
                Ok(start)
            }
        }
        _ => parse_additive_expr(first),
    }
}

fn parse_range_op(pair: &Pair<Rule>) -> RangeKind {
    if pair.as_str() == "..=" { RangeKind::Inclusive } else { RangeKind::Exclusive }
}

// ---------------------------------------------------------------------------
// Additive / Shift / Multiplicative (positional-op-chain pattern)
// ---------------------------------------------------------------------------

fn resolve_additive_op(op_str: &str) -> Result<BinaryOp> {
    match op_str {
        "+" => Ok(BinaryOp::Add),
        "-" => Ok(BinaryOp::Sub),
        _ => Err(ShapeError::ParseError {
            message: format!("Unknown additive operator: '{}'", op_str),
            location: None,
        }),
    }
}

fn resolve_shift_op(op_str: &str) -> Result<BinaryOp> {
    match op_str {
        "<<" => Ok(BinaryOp::BitShl),
        ">>" => Ok(BinaryOp::BitShr),
        _ => Err(ShapeError::ParseError {
            message: format!("Unknown shift operator: '{}'", op_str),
            location: None,
        }),
    }
}

fn resolve_multiplicative_op(op_str: &str) -> Result<BinaryOp> {
    match op_str {
        "*" => Ok(BinaryOp::Mul),
        "/" => Ok(BinaryOp::Div),
        "%" => Ok(BinaryOp::Mod),
        _ => Err(ShapeError::ParseError {
            message: format!("Unknown multiplicative operator: '{}'", op_str),
            location: None,
        }),
    }
}

/// Parse additive expression (a + b, a - b)
pub fn parse_additive_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_positional_op_chain(pair, "additive", parse_shift_expr, resolve_additive_op)
}

/// Parse shift expression (a << b, a >> b)
pub fn parse_shift_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_positional_op_chain(pair, "shift", parse_multiplicative_expr, resolve_shift_op)
}

/// Parse multiplicative expression (a * b, a / b, a % b)
pub fn parse_multiplicative_expr(pair: Pair<Rule>) -> Result<Expr> {
    parse_positional_op_chain(pair, "multiplicative", parse_exponential_expr, resolve_multiplicative_op)
}

// ---------------------------------------------------------------------------
// Exponential (right-associative: a ** b ** c = a ** (b ** c))
// ---------------------------------------------------------------------------

/// Parse exponential expression (a ** b)
pub fn parse_exponential_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Empty exponential expression".to_string(),
            location: None,
        });
    }

    let mut exprs: Vec<Expr> = Vec::new();
    for p in inner_pairs {
        exprs.push(parse_unary_expr(p)?);
    }

    if exprs.len() == 1 {
        return Ok(exprs.into_iter().next().unwrap());
    }

    // Right-associative: a ** b ** c = a ** (b ** c)
    let mut result = exprs.pop().unwrap();
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

// ---------------------------------------------------------------------------
// Unary (!a, -a, ~a, &a, &mut a)
// ---------------------------------------------------------------------------

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
        super::primary::parse_postfix_expr(first)
    }
}
