//! Statement parsing for Shape

use crate::error::{Result, ShapeError};
use crate::parser::pair_location;
use pest::iterators::Pair;

use crate::ast::{Assignment, ForInit, ForLoop, IfStatement, Span, Statement, WhileLoop};
use crate::parser::extensions::parse_extend_statement;
use crate::parser::{Rule, expressions, pair_span, parse_variable_decl};

/// Parse a statement
pub fn parse_statement(pair: Pair<Rule>) -> Result<Statement> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected statement content".to_string(),
            location: Some(pair_loc.clone()),
        })?;

    match inner.as_rule() {
        Rule::return_stmt => parse_return_stmt(inner),
        Rule::break_stmt => Ok(Statement::Break(pair_span(&inner))),
        Rule::continue_stmt => Ok(Statement::Continue(pair_span(&inner))),
        Rule::variable_decl => {
            let decl = parse_variable_decl(inner)?;
            Ok(Statement::VariableDecl(decl, span))
        }
        Rule::assignment => {
            let inner_loc = pair_location(&inner);
            let inner_span = pair_span(&inner);
            let mut inner = inner.into_inner();
            let pattern_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected pattern in assignment".to_string(),
                location: Some(inner_loc.clone()),
            })?;
            let pattern = crate::parser::parse_pattern(pattern_pair)?;
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value in assignment".to_string(),
                location: Some(inner_loc),
            })?;
            let value = expressions::parse_expression(value_pair)?;
            Ok(Statement::Assignment(
                Assignment { pattern, value },
                inner_span,
            ))
        }
        Rule::expression_stmt => {
            let inner_loc = pair_location(&inner);
            let inner_span = pair_span(&inner);
            let expr_pair = inner
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression in statement".to_string(),
                    location: Some(inner_loc),
                })?;
            let expr = expressions::parse_expression(expr_pair)?;
            Ok(Statement::Expression(expr, inner_span))
        }
        Rule::for_loop => parse_for_loop(inner),
        Rule::while_loop => parse_while_loop(inner),
        Rule::if_stmt => parse_if_stmt(inner),
        Rule::extend_statement => {
            let ext = parse_extend_statement(inner)?;
            Ok(Statement::Extend(ext, span))
        }
        Rule::function_def => {
            // Desugar nested `fn name(params) { body }` to
            // `let name = fn(params) { body }` (a VariableDecl statement).
            let func_def = crate::parser::functions::parse_function_def(inner)?;
            let func_expr = crate::ast::Expr::FunctionExpr {
                params: func_def.params,
                return_type: func_def.return_type,
                body: func_def.body,
                span,
            };
            Ok(Statement::VariableDecl(
                crate::ast::VariableDecl {
                    kind: crate::ast::VarKind::Let,
                    is_mut: false,
                    pattern: crate::ast::DestructurePattern::Identifier(func_def.name, span),
                    type_annotation: None,
                    value: Some(func_expr),
                    ownership: Default::default(),
                },
                span,
            ))
        }
        Rule::remove_target_stmt => Ok(Statement::RemoveTarget(pair_span(&inner))),
        Rule::set_param_value_stmt => {
            let inner_span = pair_span(&inner);
            let mut inner_parts = inner.into_inner();
            let param_pair = inner_parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected parameter name in `set param` value directive".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let expr_pair = inner_parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected expression in `set param` value directive".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let expression = crate::parser::expressions::parse_expression(expr_pair)?;
            Ok(Statement::SetParamValue {
                param_name: param_pair.as_str().to_string(),
                expression,
                span: inner_span,
            })
        }
        Rule::set_param_type_stmt => {
            let inner_span = pair_span(&inner);
            let mut inner_parts = inner.into_inner();
            let param_pair = inner_parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected parameter name in `set param` directive".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let type_pair = inner_parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation in `set param` directive".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let type_annotation = crate::parser::types::parse_type_annotation(type_pair)?;
            Ok(Statement::SetParamType {
                param_name: param_pair.as_str().to_string(),
                type_annotation,
                span: inner_span,
            })
        }
        Rule::set_return_stmt => {
            let inner_span = pair_span(&inner);
            let mut inner_parts = inner.into_inner();
            let payload_pair = inner_parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation or expression in `set return` directive"
                    .to_string(),
                location: Some(pair_loc.clone()),
            })?;
            match payload_pair.as_rule() {
                Rule::type_annotation => {
                    let type_annotation =
                        crate::parser::types::parse_type_annotation(payload_pair)?;
                    Ok(Statement::SetReturnType {
                        type_annotation,
                        span: inner_span,
                    })
                }
                Rule::set_return_expr_payload => {
                    let expr_pair =
                        payload_pair
                            .into_inner()
                            .next()
                            .ok_or_else(|| ShapeError::ParseError {
                                message:
                                    "expected expression in parenthesized `set return` directive"
                                        .to_string(),
                                location: Some(pair_loc.clone()),
                            })?;
                    let expression = expressions::parse_expression(expr_pair)?;
                    Ok(Statement::SetReturnExpr {
                        expression,
                        span: inner_span,
                    })
                }
                _ => Err(ShapeError::ParseError {
                    message: "expected type annotation or expression in `set return` directive"
                        .to_string(),
                    location: Some(pair_loc),
                }),
            }
        }
        Rule::replace_body_stmt => {
            let inner_span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let Some(payload) = parts.next() else {
                return Ok(Statement::ReplaceBody {
                    body: Vec::new(),
                    span: inner_span,
                });
            };
            match payload.as_rule() {
                Rule::replace_body_expr_payload => {
                    let expr_pair =
                        payload
                            .into_inner()
                            .next()
                            .ok_or_else(|| ShapeError::ParseError {
                                message:
                                    "expected expression in parenthesized `replace body` directive"
                                        .to_string(),
                                location: Some(pair_loc.clone()),
                            })?;
                    let expression = expressions::parse_expression(expr_pair)?;
                    Ok(Statement::ReplaceBodyExpr {
                        expression,
                        span: inner_span,
                    })
                }
                Rule::statement => {
                    let mut body = Vec::new();
                    body.push(parse_statement(payload)?);
                    body.extend(parse_statements(parts)?);
                    Ok(Statement::ReplaceBody {
                        body,
                        span: inner_span,
                    })
                }
                _ => Err(ShapeError::ParseError {
                    message: "expected body block or expression in `replace body` directive"
                        .to_string(),
                    location: Some(pair_loc),
                }),
            }
        }
        Rule::replace_module_stmt => {
            let inner_span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let payload = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected expression payload in `replace module` directive".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            if payload.as_rule() != Rule::replace_module_expr_payload {
                return Err(ShapeError::ParseError {
                    message: "expected parenthesized expression in `replace module` directive"
                        .to_string(),
                    location: Some(pair_loc),
                });
            }
            let expr_pair = payload
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression in parenthesized `replace module` directive"
                        .to_string(),
                    location: Some(pair_loc),
                })?;
            let expression = expressions::parse_expression(expr_pair)?;
            Ok(Statement::ReplaceModuleExpr {
                expression,
                span: inner_span,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected statement type: {:?}", inner.as_rule()),
            location: None,
        }),
    }
}

/// Parse a return statement
fn parse_return_stmt(pair: Pair<Rule>) -> Result<Statement> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Skip the return_keyword atomic token
    let first = inner.next();
    if let Some(ref p) = first {
        if p.as_rule() == Rule::return_keyword {
            let keyword_end_line = p.as_span().end_pos().line_col().0;
            // Keyword consumed, check for expression
            if let Some(expr_pair) = inner.next() {
                // Only treat as `return <expr>` if the expression starts on the
                // same line as `return`. Otherwise it's a bare `return` followed
                // by dead code on the next line (the grammar greedily consumes it).
                let expr_start_line = expr_pair.as_span().start_pos().line_col().0;
                if expr_start_line > keyword_end_line {
                    return Ok(Statement::Return(None, span));
                }
                let expr = expressions::parse_expression(expr_pair)?;
                return Ok(Statement::Return(Some(expr), span));
            } else {
                return Ok(Statement::Return(None, span));
            }
        }
    }

    if let Some(expr_pair) = first {
        // Return with expression
        let expr = expressions::parse_expression(expr_pair)?;
        Ok(Statement::Return(Some(expr), span))
    } else {
        // Return without expression
        Ok(Statement::Return(None, span))
    }
}

/// Parse a for loop
pub fn parse_for_loop(pair: Pair<Rule>) -> Result<Statement> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Parse for clause
    let for_clause = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected for clause".to_string(),
        location: Some(pair_loc),
    })?;
    let init = parse_for_clause(for_clause)?;

    // Parse body
    let mut body = vec![];
    for stmt_pair in inner {
        if stmt_pair.as_rule() == Rule::statement {
            body.push(parse_statement(stmt_pair)?);
        }
    }

    Ok(Statement::For(
        ForLoop {
            init,
            body,
            is_async: false,
        },
        span,
    ))
}

/// Parse a for clause (for x in expr or for init; cond; update)
fn parse_for_clause(pair: Pair<Rule>) -> Result<ForInit> {
    let pair_loc = pair_location(&pair);
    let inner_str = pair.as_str();
    let mut inner = pair.into_inner();

    // Check if it's a for-in loop by looking for "in" keyword
    if inner_str.contains(" in ") {
        // for x in expr (or for {x, y} in expr with destructuring)
        let pattern_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected pattern in for-in loop".to_string(),
            location: Some(pair_loc.clone()),
        })?;
        let pattern = super::items::parse_pattern(pattern_pair)?;
        let iter_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected iterable expression in for-in loop".to_string(),
            location: Some(pair_loc),
        })?;
        let iter_expr = expressions::parse_expression(iter_pair)?;
        Ok(ForInit::ForIn {
            pattern,
            iter: iter_expr,
        })
    } else {
        // for (init; condition; update)
        let init_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected initialization in for loop".to_string(),
            location: Some(pair_loc.clone()),
        })?;
        let init_decl = parse_variable_decl(init_pair)?;
        let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected condition in for loop".to_string(),
            location: Some(pair_loc.clone()),
        })?;
        let condition = expressions::parse_expression(condition_pair)?;
        let update_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected update expression in for loop".to_string(),
            location: Some(pair_loc),
        })?;
        let update = expressions::parse_expression(update_pair)?;

        Ok(ForInit::ForC {
            init: Box::new(Statement::VariableDecl(init_decl, Span::DUMMY)),
            condition,
            update,
        })
    }
}

/// Parse a while loop
pub fn parse_while_loop(pair: Pair<Rule>) -> Result<Statement> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Parse condition
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition in while loop".to_string(),
        location: Some(pair_loc),
    })?;
    let condition = expressions::parse_expression(condition_pair)?;

    // Parse body
    let mut body = vec![];
    for stmt_pair in inner {
        if stmt_pair.as_rule() == Rule::statement {
            body.push(parse_statement(stmt_pair)?);
        }
    }

    Ok(Statement::While(WhileLoop { condition, body }, span))
}

/// Parse an if statement
pub fn parse_if_stmt(pair: Pair<Rule>) -> Result<Statement> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Parse condition
    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition in if statement".to_string(),
        location: Some(pair_loc),
    })?;
    let condition = expressions::parse_expression(condition_pair)?;

    // Parse then body
    let mut then_body = vec![];
    let mut else_body = None;

    for part in inner {
        match part.as_rule() {
            Rule::statement => {
                then_body.push(parse_statement(part)?);
            }
            Rule::else_clause => {
                else_body = Some(parse_else_clause(part)?);
            }
            _ => {}
        }
    }

    Ok(Statement::If(
        IfStatement {
            condition,
            then_body,
            else_body,
        },
        span,
    ))
}

/// Parse an else clause (can be else {...} or else if (...) {...})
fn parse_else_clause(pair: Pair<Rule>) -> Result<Vec<Statement>> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();
    let mut statements = vec![];

    // Check if this is an else if
    let first = inner.next();
    if let Some(first_pair) = first {
        match first_pair.as_rule() {
            Rule::expression => {
                // This is an else if - parse condition
                let condition = expressions::parse_expression(first_pair)?;
                let mut then_body = vec![];
                let mut else_body = None;

                // Parse the body and potential else clause
                for part in inner {
                    match part.as_rule() {
                        Rule::statement => {
                            then_body.push(parse_statement(part)?);
                        }
                        Rule::else_clause => {
                            else_body = Some(parse_else_clause(part)?);
                        }
                        _ => {}
                    }
                }

                // Create an if statement for the else if
                statements.push(Statement::If(
                    IfStatement {
                        condition,
                        then_body,
                        else_body,
                    },
                    span,
                ));
            }
            Rule::statement => {
                // This is a regular else block - just parse statements
                statements.push(parse_statement(first_pair)?);
                for stmt_pair in inner {
                    if stmt_pair.as_rule() == Rule::statement {
                        statements.push(parse_statement(stmt_pair)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(statements)
}

/// Parse multiple statements (for function bodies)
pub fn parse_statements(pairs: pest::iterators::Pairs<Rule>) -> Result<Vec<Statement>> {
    let mut statements = vec![];

    for pair in pairs {
        if pair.as_rule() == Rule::statement {
            statements.push(parse_statement(pair)?);
        } else if pair.as_rule() == Rule::stmt_recovery {
            let span = pair.as_span();
            let text = pair.as_str().trim();
            let preview = if text.len() > 40 {
                format!("{}...", &text[..40])
            } else {
                text.to_string()
            };
            return Err(ShapeError::ParseError {
                message: format!("Syntax error near: {}", preview),
                location: Some(pair_location(&pair).with_length(span.end() - span.start())),
            });
        }
    }

    Ok(statements)
}
