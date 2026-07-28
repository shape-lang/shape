//! Function expression parsing
//!
//! This module handles parsing of function-related expressions:
//! - Function expressions (anonymous functions)
//! - Arrow functions
//! - Function calls
//! - Method calls
//! - Argument lists
//! - Type annotations

use super::super::pair_span;
use crate::ast::{
    CaptureClause, CaptureEntry, CaptureMode, Expr, FunctionParameter, Span, Statement,
    TypeAnnotation,
};
use crate::error::Result;
use crate::parser::Rule;
use pest::iterators::Pair;

/// ADR-009 C1 (slice 3) — parse the declared capture clause
/// `; move cfg, share total` from a closure's parameter pipe.
///
/// The clause is the PARSER's half of the "one carrier, two producers"
/// layering: it populates `Expr::FunctionExpr::captures`, the same field C2's
/// `CheckedBody` staging will populate. There is no second carrier.
fn parse_capture_clause(pair: Pair<Rule>) -> Result<CaptureClause> {
    let span = pair_span(&pair);
    let mut entries = Vec::new();
    for entry_pair in pair.into_inner() {
        if entry_pair.as_rule() != Rule::capture_entry {
            continue;
        }
        let entry_span = pair_span(&entry_pair);
        let mut mode: Option<CaptureMode> = None;
        let mut name: Option<(String, Span)> = None;
        for part in entry_pair.into_inner() {
            match part.as_rule() {
                Rule::capture_mode => {
                    // `&` / `&mut` / `move` / `share`. The reference spellings
                    // parse — and are then a TOTAL named rejection at the gate
                    // ([C0902]); they are spellable so the diagnostic can be a
                    // sentence about regions rather than a parse error.
                    let text = part.as_str();
                    let has_mut = part
                        .into_inner()
                        .any(|inner| inner.as_rule() == Rule::param_mut_keyword);
                    mode = Some(if text.starts_with('&') {
                        if has_mut {
                            CaptureMode::ExclusiveBorrow
                        } else {
                            CaptureMode::SharedBorrow
                        }
                    } else if text.starts_with("share") {
                        CaptureMode::Share
                    } else {
                        CaptureMode::Move
                    });
                }
                Rule::ident => {
                    name = Some((part.as_str().to_string(), pair_span(&part)));
                }
                _ => {}
            }
        }
        // The grammar (`capture_entry = capture_mode ~ ident`) makes both parts
        // mandatory, so a missing half is a parser bug, not a user error.
        let (Some(mode), Some((name, name_span))) = (mode, name) else {
            continue;
        };
        entries.push(CaptureEntry {
            mode,
            name,
            span: entry_span,
            name_span,
        });
    }
    Ok(CaptureClause { entries, span })
}

/// Parse function expression (anonymous function)
pub fn parse_function_expr(pair: Pair<Rule>) -> Result<Expr> {
    match pair.as_rule() {
        Rule::function_expr => {
            // Check if it's a pipe lambda or regular function
            let inner_pairs: Vec<_> = pair.clone().into_inner().collect();
            if !inner_pairs.is_empty() && inner_pairs[0].as_rule() == Rule::pipe_lambda {
                parse_pipe_lambda(inner_pairs.into_iter().next().unwrap())
            } else {
                // For regular function, pass the entire pair with all its parts
                parse_regular_function_expr(pair)
            }
        }
        Rule::pipe_lambda => parse_pipe_lambda(pair),
        _ => parse_regular_function_expr(pair),
    }
}

/// Parse pipe lambda: |x| expr, |x, y| x + y, || 42, |x| { ... }
/// Reuses the same AST node as arrow functions (Expr::FunctionExpr).
pub fn parse_pipe_lambda(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let inner = pair.into_inner();
    let mut params = vec![];
    let mut captures = None;
    let mut body_expr = None;
    let mut body_stmts = vec![];

    for part in inner {
        match part.as_rule() {
            Rule::closure_params => {
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::closure_param {
                        params.push(crate::parser::functions::parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::capture_clause => {
                captures = Some(Box::new(parse_capture_clause(part)?));
            }
            Rule::expression => {
                body_expr = Some(super::parse_expression(part)?);
            }
            Rule::function_body => {
                body_stmts = crate::parser::statements::parse_statements(part.into_inner())?;
            }
            _ => {}
        }
    }

    let body = if let Some(expr) = body_expr {
        vec![Statement::Return(Some(expr), Span::DUMMY)]
    } else {
        body_stmts
    };

    Ok(Expr::FunctionExpr {
        params,
        return_type: None,
        body,
        // ADR-009 C1 (slice 2): ordinary user source — never a generated node.
        generated_origin: None,
        // ADR-009 C1 (slice 3): producer #1 of the declared-capture carrier.
        captures,
        // Closure literals carry no annotations (C3-G12 nested-fn carrier).
        annotations: None,
        span,
    })
}

/// Parse arrow function
pub fn parse_arrow_function(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let inner = pair.into_inner();
    let mut params = vec![];
    let mut return_type: Option<TypeAnnotation> = None;
    let mut body_expr = None;
    let mut body_stmts = vec![];

    for part in inner {
        match part.as_rule() {
            Rule::ident => {
                // Single parameter without parentheses: x => x + 1
                let pattern = crate::ast::DestructurePattern::Identifier(
                    part.as_str().to_string(),
                    pair_span(&part),
                );
                params.push(FunctionParameter {
                    pattern,
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    buffer_share: crate::ast::BufferShare::Copied,
                    type_annotation: None,
                    default_value: None,
                });
            }
            Rule::function_params => {
                // Multiple parameters in parentheses: (x, y) => x + y
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(crate::parser::functions::parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::function_param => {
                // Parenthesized single parameter may be emitted directly.
                params.push(crate::parser::functions::parse_function_param(part)?);
            }
            Rule::return_type => {
                return_type = Some(crate::parser::parse_type_annotation(
                    part.into_inner().next().unwrap(),
                )?);
            }
            Rule::expression => {
                // Arrow function with expression body
                body_expr = Some(super::parse_expression(part)?);
            }
            Rule::function_body => {
                // Arrow function with block body
                body_stmts = crate::parser::statements::parse_statements(part.into_inner())?;
            }
            _ => {}
        }
    }

    // If we have an expression body, convert it to a return statement
    let body = if let Some(expr) = body_expr {
        vec![Statement::Return(Some(expr), Span::DUMMY)]
    } else {
        body_stmts
    };

    Ok(Expr::FunctionExpr {
        params,
        return_type,
        body,
        // ADR-009 C1 (slice 2): ordinary user source — never a generated node.
        generated_origin: None,
        // Arrow-function syntax has no capture clause (the form is retired for
        // new code; the grammar reaches it only through legacy paths).
        captures: None,
        // Closure literals carry no annotations (C3-G12 nested-fn carrier).
        annotations: None,
        span,
    })
}

/// Parse regular function expression
pub fn parse_regular_function_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let inner_pairs: Vec<_> = if pair.as_rule() == Rule::function_expr {
        pair.into_inner().collect()
    } else {
        vec![pair]
    };
    let mut params = vec![];
    let mut captures = None;
    let mut return_type: Option<TypeAnnotation> = None;
    let mut body = vec![];

    // Skip "function" keyword

    // Parse parameters
    for part in inner_pairs {
        match part.as_rule() {
            Rule::function_params => {
                // Parse function parameters
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(crate::parser::functions::parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::capture_clause => {
                captures = Some(Box::new(parse_capture_clause(part)?));
            }
            Rule::return_type => {
                return_type = Some(crate::parser::parse_type_annotation(
                    part.into_inner().next().unwrap(),
                )?);
            }
            Rule::function_body => {
                // Parse statements in the function body
                for stmt_pair in part.into_inner() {
                    if stmt_pair.as_rule() == Rule::statement {
                        body.push(crate::parser::statements::parse_statement(stmt_pair)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Expr::FunctionExpr {
        params,
        return_type,
        body,
        // ADR-009 C1 (slice 2): ordinary user source — never a generated node.
        generated_origin: None,
        // ADR-009 C1 (slice 3): producer #1 of the declared-capture carrier.
        captures,
        // Closure literals carry no annotations (C3-G12 nested-fn carrier).
        annotations: None,
        span,
    })
}

/// Parse argument list with support for named arguments
/// Returns (positional_args, named_args)
pub fn parse_arg_list(pair: Pair<Rule>) -> Result<(Vec<Expr>, Vec<(String, Expr)>)> {
    if let Some(arg_list) = pair.into_inner().next() {
        parse_arg_list_pairs(arg_list)
    } else {
        Ok((Vec::new(), Vec::new()))
    }
}

/// Parse a `Rule::arg_list` pair directly (no wrapping call pair).
/// Returns (positional_args, named_args)
pub fn parse_arg_list_pairs(arg_list: Pair<Rule>) -> Result<(Vec<Expr>, Vec<(String, Expr)>)> {
    let mut positional_args = Vec::new();
    let mut named_args = Vec::new();

    for argument in arg_list.into_inner() {
        match argument.as_rule() {
            Rule::argument => {
                // Unwrap the argument to get named_arg or expression
                let inner = argument.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::named_arg => {
                        let mut parts = inner.into_inner();
                        let name = parts.next().unwrap().as_str().to_string();
                        let value = super::parse_expression(parts.next().unwrap())?;
                        named_args.push((name, value));
                    }
                    _ => {
                        // It's an expression (positional argument)
                        positional_args.push(super::parse_expression(inner)?);
                    }
                }
            }
            Rule::named_arg => {
                let mut parts = argument.into_inner();
                let name = parts.next().unwrap().as_str().to_string();
                let value = super::parse_expression(parts.next().unwrap())?;
                named_args.push((name, value));
            }
            _ => {
                // Direct expression (for backward compatibility)
                positional_args.push(super::parse_expression(argument)?);
            }
        }
    }

    Ok((positional_args, named_args))
}
