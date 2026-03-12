//! Loop and flow control expression parsing
//!
//! This module handles parsing of loop constructs and flow control:
//! - While loops
//! - For loops
//! - Infinite loops
//! - Let expressions
//! - Break expressions
//! - Return expressions
//! - Block expressions

use crate::ast::{
    Assignment, AsyncLetExpr, BlockExpr, BlockItem, Expr, ForExpr, IfStatement, LetExpr, LoopExpr,
    Span, Statement, WhileExpr,
};
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use pest::iterators::Pair;

use super::super::super::pair_span;
use super::pattern_matching::parse_pattern;
use crate::parser::pair_location;

/// Parse while expression
pub fn parse_while_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let condition_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected condition in while expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let condition = super::super::parse_expression(condition_pair)?;
    let body_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected body in while expression".to_string(),
        location: Some(pair_loc),
    })?;
    let body = parse_block_expr(body_pair)?;

    Ok(Expr::While(
        Box::new(WhileExpr {
            condition: Box::new(condition),
            body: Box::new(body),
        }),
        span,
    ))
}

/// Parse for expression (including `for await x in stream { }`)
pub fn parse_for_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    // Detect `for await` by checking the raw text
    let is_async = pair.as_str().trim_start().starts_with("for") && pair.as_str().contains("await");
    let mut inner = pair.into_inner();

    let for_clause = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected for clause".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let clause_loc = pair_location(&for_clause);
    let mut clause_inner = for_clause.into_inner();
    let pattern_pair = clause_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected pattern in for loop".to_string(),
        location: Some(clause_loc.clone()),
    })?;
    let pattern = parse_pattern(pattern_pair)?;
    let iterable_pair = clause_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected iterable expression in for loop".to_string(),
        location: Some(clause_loc),
    })?;
    let iterable = super::super::parse_expression(iterable_pair)?;

    let body_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected body in for expression".to_string(),
        location: Some(pair_loc),
    })?;
    let body = parse_block_expr(body_pair)?;

    Ok(Expr::For(
        Box::new(ForExpr {
            pattern,
            iterable: Box::new(iterable),
            body: Box::new(body),
            is_async,
        }),
        span,
    ))
}

/// Parse loop expression
pub fn parse_loop_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let body_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected body in loop expression".to_string(),
        location: Some(pair_loc),
    })?;
    let body = parse_block_expr(body_pair)?;

    Ok(Expr::Loop(
        Box::new(LoopExpr {
            body: Box::new(body),
        }),
        span,
    ))
}

/// Parse let expression
pub fn parse_let_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let pattern_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected pattern in let expression".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let pattern = parse_pattern(pattern_pair)?;

    // Check if there's an initializer
    let mut value = None;
    let mut body_expr = None;

    for next_pair in inner {
        if next_pair.as_rule() == Rule::expression {
            if value.is_none() {
                value = Some(Box::new(super::super::parse_expression(next_pair)?));
            } else {
                body_expr = Some(super::super::parse_expression(next_pair)?);
            }
        }
    }

    let body = body_expr.ok_or_else(|| ShapeError::ParseError {
        message: "let expression missing body".to_string(),
        location: Some(pair_loc),
    })?;

    Ok(Expr::Let(
        Box::new(LetExpr {
            pattern,
            type_annotation: None, // Add missing field
            value,
            body: Box::new(body),
        }),
        span,
    ))
}

/// Parse break expression
pub fn parse_break_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    // Skip the break_keyword child pair — only look for an optional expression.
    let mut inner = pair
        .into_inner()
        .filter(|p| p.as_rule() != Rule::break_keyword);
    let value = if let Some(expr) = inner.next() {
        Some(Box::new(super::super::parse_expression(expr)?))
    } else {
        None
    };
    Ok(Expr::Break(value, span))
}

/// Parse return expression
pub fn parse_return_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    // The "return" keyword starts at the beginning of this pair.
    let keyword_line = pair.as_span().start_pos().line_col().0;
    // Skip the return_keyword child pair — only look for an optional expression.
    let mut inner = pair
        .into_inner()
        .filter(|p| p.as_rule() != Rule::return_keyword);
    let value = if let Some(expr) = inner.next() {
        // Only treat as `return <expr>` if the expression starts on the same
        // line as `return`. The grammar greedily consumes the next expression
        // even across newlines; bare `return` on its own line should be a
        // void return, not `return <next-line-expr>`.
        let expr_line = expr.as_span().start_pos().line_col().0;
        if expr_line > keyword_line {
            None
        } else {
            Some(Box::new(super::super::parse_expression(expr)?))
        }
    } else {
        None
    };
    Ok(Expr::Return(value, span))
}

/// Parse block expression
///
/// The PEG grammar uses `(block_statement ~ ";"?)* ~ block_item?` where `";"?`
/// is silent/optional. To implement semicolon-suppresses-return semantics
/// (`{ 1; }` yields `()` while `{ 1 }` yields `1`), we inspect the raw source
/// text after each `block_statement` span to detect whether a semicolon was
/// actually present.
pub fn parse_block_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let mut items = Vec::new();
    let mut had_semi = Vec::new();

    if let Some(block_items) = pair.into_inner().next() {
        let source = block_items.as_str();
        let block_start = block_items.as_span().start();
        // Collect all inner pairs to analyze them
        let inner_pairs: Vec<_> = block_items.into_inner().collect();

        // Process each item, tracking whether it's a statement (with semicolon) or final expression
        for item_pair in inner_pairs {
            match item_pair.as_rule() {
                Rule::block_statement => {
                    // Detect if a semicolon follows this block_statement in the source
                    let stmt_end = item_pair.as_span().end();
                    let offset = stmt_end - block_start;
                    let has_semicolon = source[offset..].starts_with(';')
                        || source[offset..].trim_start().starts_with(';');

                    let inner = item_pair.into_inner().next().unwrap();
                    let inner_span = pair_span(&inner);
                    let block_item = parse_block_entry(inner)?;
                    // If a semicolon follows, ensure expressions become statements
                    // so they don't produce a value on the stack
                    let block_item = if has_semicolon {
                        expr_to_statement(block_item, inner_span)
                    } else {
                        block_item
                    };
                    had_semi.push(has_semicolon);
                    items.push(block_item);
                }
                Rule::block_item => {
                    // This is the final expression without semicolon - the block's value
                    let inner = item_pair.into_inner().next().unwrap();
                    let block_item = parse_block_entry(inner)?;
                    // Convert tail-position if-statement to a conditional expression
                    // so the block evaluates to the if's value.
                    let block_item = if_stmt_to_tail_expr(block_item);
                    had_semi.push(false);
                    items.push(block_item);
                }
                _ => {} // Skip other tokens
            }
        }
    }

    // Empty blocks evaluate to Unit
    if items.is_empty() {
        return Ok(Expr::Unit(span));
    }

    // Only promote the last item to a tail expression if it did NOT have a
    // trailing semicolon. When it did, the expression was already wrapped as
    // a Statement by expr_to_statement above, and the compiler's
    // compile_expr_block will emit unit for the missing tail value.
    if let Some(&last_had_semi) = had_semi.last() {
        if !last_had_semi {
            if let Some(last) = items.pop() {
                items.push(if_stmt_to_tail_expr(last));
            }
        }
    }

    Ok(Expr::Block(BlockExpr { items }, span))
}

/// Convert a `BlockItem::Expression` to a `BlockItem::Statement` so the
/// compiler treats it as a side-effect (pops the value) rather than keeping
/// it as the block's return value.
fn expr_to_statement(item: BlockItem, span: Span) -> BlockItem {
    match item {
        BlockItem::Expression(expr) => BlockItem::Statement(Statement::Expression(expr, span)),
        other => other,
    }
}

/// Convert a tail-position `if` statement into a conditional expression so the
/// block evaluates to the value of the `if` rather than discarding it.
///
/// Only `BlockItem::Statement(Statement::If(..))` is converted; every other
/// variant is returned unchanged.
fn if_stmt_to_tail_expr(item: BlockItem) -> BlockItem {
    match item {
        BlockItem::Statement(Statement::If(if_stmt, span)) => {
            BlockItem::Expression(if_stmt_to_conditional(if_stmt, span))
        }
        other => other,
    }
}

/// Recursively convert an `IfStatement` (statement form) into an
/// `Expr::Conditional` (expression form) by wrapping each branch's
/// `Vec<Statement>` in a `Block` expression.
fn if_stmt_to_conditional(if_stmt: IfStatement, span: Span) -> Expr {
    let then_expr = stmts_to_block_expr(if_stmt.then_body, span);

    let else_expr = if_stmt.else_body.map(|stmts| {
        // An `else if` is represented as a single Statement::If inside the vec.
        if stmts.len() == 1 && matches!(stmts.first(), Some(Statement::If(..))) {
            let mut iter = stmts.into_iter();
            if let Some(Statement::If(nested_if, nested_span)) = iter.next() {
                return Box::new(if_stmt_to_conditional(nested_if, nested_span));
            }
            // The matches! guard above ensures we have Statement::If, so this
            // path is not reachable. Fall through to stmts_to_block_expr with
            // an empty vec if it ever were.
            return Box::new(stmts_to_block_expr(Vec::new(), span));
        }
        Box::new(stmts_to_block_expr(stmts, span))
    });

    Expr::Conditional {
        condition: Box::new(if_stmt.condition),
        then_expr: Box::new(then_expr),
        else_expr,
        span,
    }
}

/// Wrap a `Vec<Statement>` as a block expression whose last entry
/// is promoted to `BlockItem::Expression` when possible, so it
/// produces a value on the stack.
fn stmts_to_block_expr(stmts: Vec<Statement>, span: Span) -> Expr {
    if stmts.is_empty() {
        return Expr::Unit(span);
    }
    let len = stmts.len();
    let mut items: Vec<BlockItem> = Vec::with_capacity(len);
    for (i, s) in stmts.into_iter().enumerate() {
        let is_last = i == len - 1;
        if is_last {
            match s {
                // Promote trailing expression-statement to a block expression item
                Statement::Expression(expr, _) => {
                    items.push(BlockItem::Expression(expr));
                }
                // Promote trailing nested if to a conditional expression (recursively)
                Statement::If(nested_if, nested_span) => {
                    items.push(BlockItem::Expression(if_stmt_to_conditional(
                        nested_if,
                        nested_span,
                    )));
                }
                other => {
                    items.push(BlockItem::Statement(other));
                }
            }
        } else {
            items.push(BlockItem::Statement(s));
        }
    }
    Expr::Block(BlockExpr { items }, span)
}

fn parse_block_entry(inner: Pair<Rule>) -> Result<BlockItem> {
    match inner.as_rule() {
        Rule::return_stmt => {
            let return_span = pair_span(&inner);
            let value = inner
                .into_inner()
                .filter(|p| p.as_rule() != Rule::return_keyword)
                .next()
                .map(|expr_pair| super::super::parse_expression(expr_pair))
                .transpose()?
                .map(Box::new);
            Ok(BlockItem::Expression(Expr::Return(value, return_span)))
        }
        Rule::variable_decl => {
            let decl = crate::parser::parse_variable_decl(inner)?;
            Ok(BlockItem::VariableDecl(decl))
        }
        Rule::assignment => {
            let mut inner = inner.into_inner();
            let pattern = crate::parser::parse_pattern(inner.next().unwrap())?;
            let value = super::super::parse_expression(inner.next().unwrap())?;
            Ok(BlockItem::Assignment(Assignment { pattern, value }))
        }
        Rule::expression => {
            let expr = super::super::parse_expression(inner)?;
            Ok(BlockItem::Expression(expr))
        }
        Rule::if_stmt => {
            let stmt = crate::parser::statements::parse_if_stmt(inner)?;
            Ok(BlockItem::Statement(stmt))
        }
        Rule::for_loop => {
            let stmt = crate::parser::statements::parse_for_loop(inner)?;
            Ok(BlockItem::Statement(stmt))
        }
        Rule::while_loop => {
            let stmt = crate::parser::statements::parse_while_loop(inner)?;
            Ok(BlockItem::Statement(stmt))
        }
        Rule::extend_statement => {
            let span = pair_span(&inner);
            let ext = crate::parser::extensions::parse_extend_statement(inner)?;
            Ok(BlockItem::Statement(crate::ast::Statement::Extend(
                ext, span,
            )))
        }
        Rule::remove_target_stmt => Ok(BlockItem::Statement(crate::ast::Statement::RemoveTarget(
            pair_span(&inner),
        ))),
        Rule::set_param_value_stmt => {
            let span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let param_pair = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected parameter name in `set param` value directive".to_string(),
                location: None,
            })?;
            let expr_pair = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected expression in `set param` value directive".to_string(),
                location: None,
            })?;
            let expression = super::super::parse_expression(expr_pair)?;
            Ok(BlockItem::Statement(crate::ast::Statement::SetParamValue {
                param_name: param_pair.as_str().to_string(),
                expression,
                span,
            }))
        }
        Rule::set_param_type_stmt => {
            let span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let param_pair = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected parameter name in `set param` directive".to_string(),
                location: None,
            })?;
            let type_pair = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation in `set param` directive".to_string(),
                location: None,
            })?;
            let type_annotation = crate::parser::types::parse_type_annotation(type_pair)?;
            Ok(BlockItem::Statement(crate::ast::Statement::SetParamType {
                param_name: param_pair.as_str().to_string(),
                type_annotation,
                span,
            }))
        }
        Rule::set_return_stmt => {
            let span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let payload_pair = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation or expression in `set return` directive"
                    .to_string(),
                location: None,
            })?;
            match payload_pair.as_rule() {
                Rule::type_annotation => {
                    let type_annotation =
                        crate::parser::types::parse_type_annotation(payload_pair)?;
                    Ok(BlockItem::Statement(crate::ast::Statement::SetReturnType {
                        type_annotation,
                        span,
                    }))
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
                                location: None,
                            })?;
                    let expression = super::super::parse_expression(expr_pair)?;
                    Ok(BlockItem::Statement(crate::ast::Statement::SetReturnExpr {
                        expression,
                        span,
                    }))
                }
                _ => Err(ShapeError::ParseError {
                    message: "expected type annotation or expression in `set return` directive"
                        .to_string(),
                    location: None,
                }),
            }
        }
        Rule::replace_body_stmt => {
            let span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let Some(payload) = parts.next() else {
                return Ok(BlockItem::Statement(crate::ast::Statement::ReplaceBody {
                    body: Vec::new(),
                    span,
                }));
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
                                location: None,
                            })?;
                    let expression = super::super::parse_expression(expr_pair)?;
                    Ok(BlockItem::Statement(
                        crate::ast::Statement::ReplaceBodyExpr { expression, span },
                    ))
                }
                Rule::statement => {
                    let mut body = Vec::new();
                    body.push(crate::parser::statements::parse_statement(payload)?);
                    body.extend(crate::parser::statements::parse_statements(parts)?);
                    Ok(BlockItem::Statement(crate::ast::Statement::ReplaceBody {
                        body,
                        span,
                    }))
                }
                _ => Err(ShapeError::ParseError {
                    message: "expected body block or expression in `replace body` directive"
                        .to_string(),
                    location: None,
                }),
            }
        }
        Rule::replace_module_stmt => {
            let span = pair_span(&inner);
            let mut parts = inner.into_inner();
            let payload = parts.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected expression payload in `replace module` directive".to_string(),
                location: None,
            })?;
            if payload.as_rule() != Rule::replace_module_expr_payload {
                return Err(ShapeError::ParseError {
                    message: "expected parenthesized expression in `replace module` directive"
                        .to_string(),
                    location: None,
                });
            }
            let expr_pair = payload
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression in parenthesized `replace module` directive"
                        .to_string(),
                    location: None,
                })?;
            let expression = super::super::parse_expression(expr_pair)?;
            Ok(BlockItem::Statement(
                crate::ast::Statement::ReplaceModuleExpr { expression, span },
            ))
        }
        // Nested function definition: desugar `fn name(params) { body }` inside a block
        // to `let name = fn(params) { body }` (a VariableDecl with a FunctionExpr value).
        Rule::function_def => {
            let span = pair_span(&inner);
            let func_def = crate::parser::functions::parse_function_def(inner)?;
            let func_expr = Expr::FunctionExpr {
                params: func_def.params,
                return_type: func_def.return_type,
                body: func_def.body,
                span,
            };
            Ok(BlockItem::VariableDecl(crate::ast::VariableDecl {
                kind: crate::ast::VarKind::Let,
                is_mut: false,
                pattern: crate::ast::DestructurePattern::Identifier(func_def.name, span),
                type_annotation: None,
                value: Some(func_expr),
                ownership: Default::default(),
            }))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected block entry: {:?}", inner.as_rule()),
            location: None,
        }),
    }
}

/// Parse async let expression: `async let name = expr`
/// Spawns a task and binds a future handle to a local variable.
pub fn parse_async_let_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected variable name in async let".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let name = name_pair.as_str().to_string();

    let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in async let".to_string(),
        location: Some(pair_loc),
    })?;
    let expr = super::super::parse_expression(expr_pair)?;

    Ok(Expr::AsyncLet(
        Box::new(AsyncLetExpr {
            name,
            expr: Box::new(expr),
            span,
        }),
        span,
    ))
}

/// Parse async scope expression: `async scope { ... }`
/// Cancellation boundary -- on scope exit, all pending tasks are cancelled in reverse order.
pub fn parse_async_scope_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let body_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected body in async scope".to_string(),
        location: Some(pair_loc),
    })?;
    let body = parse_block_expr(body_pair)?;

    Ok(Expr::AsyncScope(Box::new(body), span))
}
