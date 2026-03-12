//! Primary expression parsing
//!
//! This module handles parsing of primary expressions and postfix operations:
//! - Primary expressions (identifiers, literals, grouped expressions)
//! - Postfix operations (property access, method calls, indexing)
//! - Expression entry point

use crate::ast::{
    DataRef, EnumConstructorPayload, Expr, JoinBranch, JoinExpr, JoinKind, RangeKind, Span, Spanned,
};
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

use super::super::pair_span;

/// Main entry point for parsing expressions
pub fn parse_expression(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::expression => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression content".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_expression(inner)
        }
        Rule::assignment_expr => super::binary_ops::parse_assignment_expr(pair),
        Rule::ternary_expr => super::binary_ops::parse_ternary_expr(pair),
        Rule::null_coalesce_expr => super::binary_ops::parse_null_coalesce_expr(pair),
        Rule::context_expr => super::binary_ops::parse_context_expr(pair),
        Rule::or_expr => super::binary_ops::parse_or_expr(pair),
        Rule::range_expr => super::binary_ops::parse_range_expr(pair),
        _ => Err(ShapeError::ParseError {
            message: format!("expected expression, got {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}

/// Parse postfix expression (property access, method calls, indexing)
pub fn parse_postfix_expr(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let primary_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected primary expression".to_string(),
        location: Some(pair_loc),
    })?;
    let mut expr = parse_primary_expr(primary_pair)?;

    // Collect all postfix operations to handle method calls
    let postfix_ops: Vec<_> = inner.collect();
    let mut i = 0;

    while i < postfix_ops.len() {
        let postfix = &postfix_ops[i];
        match postfix.as_rule() {
            Rule::property_access | Rule::optional_property_access => {
                let is_optional = postfix.as_rule() == Rule::optional_property_access;
                let postfix_loc = pair_location(postfix);
                let property = postfix
                    .clone()
                    .into_inner()
                    .next()
                    .ok_or_else(|| ShapeError::ParseError {
                        message: "expected property name after '.'".to_string(),
                        location: Some(postfix_loc),
                    })?
                    .as_str()
                    .to_string();

                let full_span = Span::new(expr.span().start, pair_span(postfix).end);

                // Check if next operation is a function call (method call)
                if i + 1 < postfix_ops.len() && postfix_ops[i + 1].as_rule() == Rule::function_call
                {
                    let (args, named_args) =
                        super::functions::parse_arg_list(postfix_ops[i + 1].clone())?;
                    expr = Expr::MethodCall {
                        receiver: Box::new(expr),
                        method: property,
                        args,
                        named_args,
                        optional: is_optional,
                        span: full_span,
                    };
                    i += 2; // Skip the function call we just processed
                } else {
                    expr = Expr::PropertyAccess {
                        object: Box::new(expr),
                        property,
                        optional: is_optional,
                        span: full_span,
                    };
                    i += 1;
                }
            }
            Rule::index_access => {
                let postfix_loc = pair_location(postfix);
                let postfix_span = pair_span(postfix);
                // Special case for data indexing (legacy "candle" identifier support)
                if let Expr::Identifier(ref id, _) = expr {
                    if id == "candle" {
                        let index_expr = postfix.clone().into_inner().next().ok_or_else(|| {
                            ShapeError::ParseError {
                                message: "expected index expression in data access".to_string(),
                                location: Some(postfix_loc.clone()),
                            }
                        })?;
                        let (index, timeframe) = super::data_refs::parse_index_expr(index_expr)?;
                        expr = Expr::DataRef(DataRef { index, timeframe }, postfix_span);
                        i += 1;
                        continue;
                    }
                }
                // Special case for data with timeframe: data(5m)[0]
                if let Expr::DataRef(ref mut data_ref, _) = expr {
                    let index_expr = postfix.clone().into_inner().next().ok_or_else(|| {
                        ShapeError::ParseError {
                            message: "expected index expression in data access".to_string(),
                            location: Some(postfix_loc.clone()),
                        }
                    })?;
                    let (index, timeframe_override) =
                        super::data_refs::parse_index_expr(index_expr)?;
                    data_ref.index = index;
                    // If there's a timeframe in the index, it overrides the one from data(5m)
                    if timeframe_override.is_some() {
                        data_ref.timeframe = timeframe_override;
                    }
                    i += 1;
                    continue;
                }
                // General array/object indexing
                let index_expr_pair =
                    postfix
                        .clone()
                        .into_inner()
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected index expression".to_string(),
                            location: Some(postfix_loc),
                        })?;
                let (index_expr, end_expr) =
                    super::data_refs::parse_index_expr_general(index_expr_pair)?;
                expr = Expr::IndexAccess {
                    object: Box::new(expr),
                    index: Box::new(index_expr),
                    end_index: end_expr.map(Box::new),
                    span: postfix_span,
                };
                i += 1;
            }
            Rule::function_call => {
                let (args, named_args) = super::functions::parse_arg_list(postfix.clone())?;
                if let Expr::Identifier(name, _) = expr {
                    expr = Expr::FunctionCall {
                        name,
                        args,
                        named_args,
                        span: pair_span(postfix),
                    };
                } else {
                    // Chained call: expr(args) where expr is not an identifier
                    let full_span = Span::new(expr.span().start, pair_span(postfix).end);
                    expr = Expr::MethodCall {
                        receiver: Box::new(expr),
                        method: "__call__".to_string(),
                        args,
                        named_args,
                        optional: false,
                        span: full_span,
                    };
                }
                i += 1;
            }
            Rule::type_assertion_suffix => {
                let mut inner = postfix.clone().into_inner();

                // Skip the as_keyword pair
                let first = inner.next().ok_or_else(|| ShapeError::ParseError {
                    message: "Type assertion missing type annotation".to_string(),
                    location: None,
                })?;
                let type_pair = if first.as_rule() == Rule::as_keyword {
                    inner.next().ok_or_else(|| ShapeError::ParseError {
                        message: "Type assertion missing type annotation after 'as'".to_string(),
                        location: None,
                    })?
                } else {
                    first
                };
                let type_annotation = crate::parser::parse_type_annotation(type_pair)?;

                // Check for optional comptime field overrides
                let meta_param_overrides = if let Some(overrides_pair) = inner.next() {
                    if overrides_pair.as_rule() == Rule::comptime_field_overrides {
                        Some(crate::parser::types::parse_comptime_field_overrides(
                            overrides_pair,
                        )?)
                    } else {
                        None
                    }
                } else {
                    None
                };

                expr = Expr::TypeAssertion {
                    expr: Box::new(expr),
                    type_annotation,
                    meta_param_overrides,
                    span: pair_span(postfix),
                };
                i += 1;
            }
            Rule::using_impl_suffix => {
                let impl_name = postfix
                    .clone()
                    .into_inner()
                    .next()
                    .ok_or_else(|| ShapeError::ParseError {
                        message: "Missing impl name after 'using'".to_string(),
                        location: Some(pair_location(postfix)),
                    })?
                    .as_str()
                    .to_string();
                let full_span = Span::new(expr.span().start, pair_span(postfix).end);
                expr = Expr::UsingImpl {
                    expr: Box::new(expr),
                    impl_name,
                    span: full_span,
                };
                i += 1;
            }
            Rule::try_operator => {
                // Try operator: expr? - unified fallible propagation (Result/Option)
                expr = Expr::TryOperator(Box::new(expr), pair_span(postfix));
                i += 1;
            }
            _ => {
                return Err(ShapeError::ParseError {
                    message: format!("Unexpected postfix operator: {:?}", postfix.as_rule()),
                    location: None,
                });
            }
        }
    }

    Ok(expr)
}

/// Parse primary expression
pub fn parse_primary_expr(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    // Handle case where we might have nested primary_expr rules
    match pair.as_rule() {
        Rule::primary_expr => {
            // Check if this is an empty array literal case
            let inner_pairs: Vec<_> = pair.clone().into_inner().collect();
            if inner_pairs.is_empty() {
                // For cases that shouldn't happen
                return Err(ShapeError::ParseError {
                    message: "empty primary expression".to_string(),
                    location: Some(pair_loc),
                });
            }
            let first = inner_pairs
                .into_iter()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected primary expression content".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_primary_expr_inner(first)
        }
        _ => parse_primary_expr_inner(pair),
    }
}

/// Parse the inner primary expression
fn parse_primary_expr_inner(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);

    match pair.as_rule() {
        Rule::unit_literal => Ok(Expr::Literal(crate::ast::Literal::Unit, span)),
        Rule::literal => super::literals::parse_literal(pair),
        Rule::array_literal => super::literals::parse_array_literal(pair),
        Rule::data_ref => super::data_refs::parse_data_ref(pair),
        Rule::time_ref => Ok(Expr::TimeRef(super::temporal::parse_time_ref(pair)?, span)),
        Rule::datetime_expr => Ok(Expr::DateTime(
            super::temporal::parse_datetime_expr(pair)?,
            span,
        )),
        Rule::pattern_name => {
            let name_pair = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected pattern name".to_string(),
                    location: Some(pair_loc),
                })?;
            Ok(Expr::PatternRef(name_pair.as_str().to_string(), span))
        }
        Rule::enum_constructor_expr => parse_enum_constructor_expr(pair),
        Rule::ident => Ok(Expr::Identifier(pair.as_str().to_string(), span)),
        Rule::expression => parse_expression(pair),
        Rule::temporal_nav => super::temporal::parse_temporal_nav(pair),
        Rule::timeframe_expr => super::temporal::parse_timeframe_expr(pair),
        Rule::async_let_expr => super::control_flow::parse_async_let_expr(pair),
        Rule::async_scope_expr => super::control_flow::parse_async_scope_expr(pair),
        Rule::if_expr => super::control_flow::parse_if_expr(pair),
        Rule::while_expr => super::control_flow::parse_while_expr(pair),
        Rule::for_expr => super::control_flow::parse_for_expr(pair),
        Rule::loop_expr => super::control_flow::parse_loop_expr(pair),
        Rule::let_expr => super::control_flow::parse_let_expr(pair),
        Rule::match_expr => super::control_flow::parse_match_expr(pair),
        Rule::break_expr => super::control_flow::parse_break_expr(pair),
        Rule::continue_expr => Ok(Expr::Continue(span)),
        Rule::return_expr => super::control_flow::parse_return_expr(pair),
        Rule::block_expr => super::control_flow::parse_block_expr(pair),
        Rule::object_literal => super::literals::parse_object_literal(pair),
        Rule::function_expr => super::functions::parse_function_expr(pair),
        Rule::some_expr => parse_some_expr(pair),
        Rule::duration => super::temporal::parse_duration(pair),
        // Handle nested primary_expr (this should recursively call parse_primary_expr)
        Rule::primary_expr => parse_primary_expr(pair),
        // Handle postfix_expr (function calls, property access, etc.)
        Rule::postfix_expr => parse_postfix_expr(pair),
        Rule::list_comprehension => super::comprehensions::parse_list_comprehension(pair),
        Rule::await_expr => parse_await_expr(pair),
        Rule::struct_literal => parse_struct_literal(pair),
        Rule::from_query_expr => super::control_flow::parse_from_query_expr(pair),
        Rule::comptime_for_expr => parse_comptime_for_expr(pair),
        Rule::comptime_block => parse_comptime_block(pair),
        Rule::annotated_expr => parse_annotated_expr(pair),
        Rule::datetime_range => {
            // Parse datetime range - return Range expression if end is present
            let (start, end) = super::temporal::parse_datetime_range(pair)?;
            match end {
                Some(end_expr) => Ok(Expr::Range {
                    start: Some(Box::new(start)),
                    end: Some(Box::new(end_expr)),
                    kind: RangeKind::Inclusive, // DateTime ranges are inclusive
                    span,
                }),
                None => Ok(start),
            }
        }
        _ => Err(ShapeError::ParseError {
            message: format!("Unexpected primary expression: {:?}", pair.as_rule()),
            location: None,
        }),
    }
}

/// Parse Some expression: Some(value) constructor for Option type
fn parse_some_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected expression in Some()".to_string(),
            location: Some(pair_loc),
        })?;
    let inner_expr = parse_expression(inner)?;
    Ok(Expr::FunctionCall {
        name: "Some".to_string(),
        args: vec![inner_expr],
        named_args: vec![],
        span,
    })
}

/// Parse await expression: await expr | await @annotation expr | await join kind { ... }
fn parse_await_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    // Filter out the await_keyword atomic token
    let inner_pairs: Vec<_> = pair
        .into_inner()
        .filter(|p| p.as_rule() != Rule::await_keyword)
        .collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "expected expression after 'await'".to_string(),
            location: Some(pair_loc),
        });
    }

    // Check what the first inner element is
    let first = &inner_pairs[0];
    match first.as_rule() {
        Rule::join_expr => {
            // await join all|race|any|settle { branches }
            let join = parse_join_expr(first.clone())?;
            let join_span = pair_span(first);
            Ok(Expr::Await(
                Box::new(Expr::Join(Box::new(join), join_span)),
                span,
            ))
        }
        Rule::annotation => {
            // await @annotation ... expr — collect annotations and wrap target
            let mut annotations = Vec::new();
            let mut target_pair = None;

            for p in &inner_pairs {
                match p.as_rule() {
                    Rule::annotation => {
                        annotations.push(crate::parser::functions::parse_annotation(p.clone())?);
                    }
                    Rule::postfix_expr => {
                        target_pair = Some(p.clone());
                    }
                    _ => {}
                }
            }

            let target = target_pair.ok_or_else(|| ShapeError::ParseError {
                message: "expected expression after annotations in await".to_string(),
                location: Some(pair_loc),
            })?;

            let mut expr = parse_postfix_expr(target)?;

            // Wrap in Annotated nodes right-to-left so the first annotation is outermost
            for annotation in annotations.into_iter().rev() {
                let anno_span = Span::new(annotation.span.start, expr.span().end);
                expr = Expr::Annotated {
                    annotation,
                    target: Box::new(expr),
                    span: anno_span,
                };
            }

            Ok(Expr::Await(Box::new(expr), span))
        }
        Rule::postfix_expr => {
            // await expr (simple case)
            let expr = parse_postfix_expr(first.clone())?;
            Ok(Expr::Await(Box::new(expr), span))
        }
        _ => Err(ShapeError::ParseError {
            message: format!(
                "unexpected token in await expression: {:?}",
                first.as_rule()
            ),
            location: Some(pair_loc),
        }),
    }
}

/// Parse join expression: join all|race|any|settle { branch, ... }
fn parse_join_expr(pair: Pair<Rule>) -> Result<JoinExpr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse join kind
    let kind_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected join strategy (all, race, any, settle)".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let kind = match kind_pair.as_str() {
        "all" => JoinKind::All,
        "race" => JoinKind::Race,
        "any" => JoinKind::Any,
        "settle" => JoinKind::Settle,
        other => {
            return Err(ShapeError::ParseError {
                message: format!(
                    "unknown join strategy: '{}'. Expected all, race, any, or settle",
                    other
                ),
                location: Some(pair_location(&kind_pair)),
            });
        }
    };

    // Parse branch list
    let branch_list_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected join branches".to_string(),
        location: Some(pair_loc),
    })?;

    let mut branches = Vec::new();
    for branch_pair in branch_list_pair.into_inner() {
        if branch_pair.as_rule() == Rule::join_branch {
            branches.push(parse_join_branch(branch_pair)?);
        }
    }

    Ok(JoinExpr {
        kind,
        branches,
        span,
    })
}

/// Parse a single join branch: [annotations] [label:] expression
fn parse_join_branch(pair: Pair<Rule>) -> Result<JoinBranch> {
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    let mut annotations = Vec::new();
    let mut label = None;
    let mut expr = None;

    // The grammar alternatives for join_branch produce different inner sequences:
    // 1. annotation+ ident ":" expression -> annotations, label, expr
    // 2. annotation+ expression -> annotations, expr
    // 3. ident ":" expression -> label, expr
    // 4. expression -> expr
    //
    // We can distinguish by checking types in order.
    let mut i = 0;

    // Collect leading annotations
    while i < inner_pairs.len() && inner_pairs[i].as_rule() == Rule::annotation {
        annotations.push(crate::parser::functions::parse_annotation(
            inner_pairs[i].clone(),
        )?);
        i += 1;
    }

    // Check if next is ident followed by expression (label case)
    if i < inner_pairs.len() && inner_pairs[i].as_rule() == Rule::ident {
        // Could be label:expr or just an expression that starts with ident
        // In the grammar, the ident alternative in join_branch is specifically
        // `ident ~ ":" ~ expression`, so if we get Rule::ident here followed
        // by Rule::expression, it's a labeled branch.
        if i + 1 < inner_pairs.len() && inner_pairs[i + 1].as_rule() == Rule::expression {
            label = Some(inner_pairs[i].as_str().to_string());
            i += 1;
            expr = Some(parse_expression(inner_pairs[i].clone())?);
        } else {
            // Shouldn't happen with correct grammar, but handle gracefully
            expr = Some(parse_expression(inner_pairs[i].clone())?);
        }
    } else if i < inner_pairs.len() {
        expr = Some(parse_expression(inner_pairs[i].clone())?);
    }

    let expr = expr.ok_or_else(|| ShapeError::ParseError {
        message: "expected expression in join branch".to_string(),
        location: None,
    })?;

    Ok(JoinBranch {
        label,
        expr,
        annotations,
    })
}

/// Parse struct literal: TypeName { field: value, ... }
///
/// Grammar: `ident ~ "{" ~ object_fields? ~ "}"`
/// Reuses object_fields parsing (same as object literals and enum struct payloads)
fn parse_struct_literal(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected struct type name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let type_name = name_pair.as_str().to_string();

    let mut fields = Vec::new();

    // Parse optional object_fields
    if let Some(fields_pair) = inner.next() {
        if fields_pair.as_rule() == Rule::object_fields {
            for field_item_pair in fields_pair.into_inner() {
                if field_item_pair.as_rule() != Rule::object_field_item {
                    continue;
                }
                let field_item_loc = pair_location(&field_item_pair);
                let field_item_inner =
                    field_item_pair
                        .into_inner()
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected struct field content".to_string(),
                            location: Some(field_item_loc.clone()),
                        })?;
                match field_item_inner.as_rule() {
                    Rule::object_field => {
                        let mut field_inner = field_item_inner.into_inner();
                        let field_kind =
                            field_inner.next().ok_or_else(|| ShapeError::ParseError {
                                message: "expected struct field content".to_string(),
                                location: Some(field_item_loc.clone()),
                            })?;
                        match field_kind.as_rule() {
                            Rule::object_value_field => {
                                let mut value_inner = field_kind.into_inner();
                                let key_pair =
                                    value_inner.next().ok_or_else(|| ShapeError::ParseError {
                                        message: "expected struct field key".to_string(),
                                        location: Some(field_item_loc.clone()),
                                    })?;
                                let key_pair = if key_pair.as_rule() == Rule::object_field_name {
                                    key_pair.into_inner().next().ok_or_else(|| {
                                        ShapeError::ParseError {
                                            message: "expected struct field key".to_string(),
                                            location: Some(field_item_loc.clone()),
                                        }
                                    })?
                                } else {
                                    key_pair
                                };
                                let key = key_pair.as_str().to_string();
                                let value_pair =
                                    value_inner.next().ok_or_else(|| ShapeError::ParseError {
                                        message: format!(
                                            "expected value for struct field '{}'",
                                            key
                                        ),
                                        location: Some(field_item_loc),
                                    })?;
                                let value = parse_expression(value_pair)?;
                                fields.push((key, value));
                            }
                            _ => {
                                return Err(ShapeError::ParseError {
                                    message: "typed fields and spreads are not supported in struct literals".to_string(),
                                    location: Some(field_item_loc),
                                });
                            }
                        }
                    }
                    Rule::object_spread => {
                        return Err(ShapeError::ParseError {
                            message: "spread is not supported in struct literals".to_string(),
                            location: Some(field_item_loc),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(Expr::StructLiteral {
        type_name,
        fields,
        span,
    })
}

fn parse_enum_constructor_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let path_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected enum variant path".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let (enum_name, variant) = parse_enum_variant_path(path_pair)?;

    let payload = if let Some(payload_pair) = inner.next() {
        match payload_pair.as_rule() {
            Rule::enum_tuple_payload => {
                let (args, named_args) = super::functions::parse_arg_list(payload_pair)?;
                if !named_args.is_empty() {
                    return Err(ShapeError::ParseError {
                        message: "named arguments are not allowed in enum tuple constructors"
                            .to_string(),
                        location: Some(pair_loc),
                    });
                }
                EnumConstructorPayload::Tuple(args)
            }
            Rule::enum_struct_payload => {
                let fields = parse_enum_struct_payload(payload_pair)?;
                EnumConstructorPayload::Struct(fields)
            }
            other => {
                return Err(ShapeError::ParseError {
                    message: format!("unexpected enum constructor payload: {:?}", other),
                    location: Some(pair_loc),
                });
            }
        }
    } else {
        EnumConstructorPayload::Unit
    };

    Ok(Expr::EnumConstructor {
        enum_name,
        variant,
        payload,
        span,
    })
}

fn parse_enum_variant_path(pair: Pair<Rule>) -> Result<(String, String)> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let enum_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected enum name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let variant_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected enum variant name".to_string(),
        location: Some(pair_loc),
    })?;
    Ok((
        enum_pair.as_str().to_string(),
        variant_pair.as_str().to_string(),
    ))
}

fn parse_enum_struct_payload(pair: Pair<Rule>) -> Result<Vec<(String, Expr)>> {
    let mut fields = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() != Rule::object_fields {
            continue;
        }
        for field_item_pair in inner.into_inner() {
            if field_item_pair.as_rule() != Rule::object_field_item {
                continue;
            }
            let field_item_loc = pair_location(&field_item_pair);
            let field_item_inner =
                field_item_pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ShapeError::ParseError {
                        message: "expected enum struct field content".to_string(),
                        location: Some(field_item_loc.clone()),
                    })?;
            match field_item_inner.as_rule() {
                Rule::object_field => {
                    let mut field_inner = field_item_inner.into_inner();
                    let field_kind = field_inner.next().ok_or_else(|| ShapeError::ParseError {
                        message: "expected enum struct field content".to_string(),
                        location: Some(field_item_loc.clone()),
                    })?;
                    match field_kind.as_rule() {
                        Rule::object_value_field => {
                            let mut value_inner = field_kind.into_inner();
                            let key_pair =
                                value_inner.next().ok_or_else(|| ShapeError::ParseError {
                                    message: "expected enum struct field key".to_string(),
                                    location: Some(field_item_loc.clone()),
                                })?;
                            let key_pair = if key_pair.as_rule() == Rule::object_field_name {
                                key_pair.into_inner().next().ok_or_else(|| {
                                    ShapeError::ParseError {
                                        message: "expected enum struct field key".to_string(),
                                        location: Some(field_item_loc.clone()),
                                    }
                                })?
                            } else {
                                key_pair
                            };
                            let key = match key_pair.as_rule() {
                                Rule::ident | Rule::keyword => key_pair.as_str().to_string(),
                                _ => {
                                    return Err(ShapeError::ParseError {
                                        message: "unexpected enum struct field key".to_string(),
                                        location: Some(pair_location(&key_pair)),
                                    });
                                }
                            };
                            let value_pair =
                                value_inner.next().ok_or_else(|| ShapeError::ParseError {
                                    message: format!("expected value for enum field '{}'", key),
                                    location: Some(field_item_loc),
                                })?;
                            let value = parse_expression(value_pair)?;
                            fields.push((key, value));
                        }
                        Rule::object_typed_field => {
                            return Err(ShapeError::ParseError {
                                message: "typed fields are not allowed in enum constructors"
                                    .to_string(),
                                location: Some(field_item_loc),
                            });
                        }
                        other => {
                            return Err(ShapeError::ParseError {
                                message: format!("unexpected enum field kind: {:?}", other),
                                location: Some(field_item_loc),
                            });
                        }
                    }
                }
                Rule::object_spread => {
                    return Err(ShapeError::ParseError {
                        message: "spread fields are not allowed in enum constructors".to_string(),
                        location: Some(field_item_loc),
                    });
                }
                other => {
                    return Err(ShapeError::ParseError {
                        message: format!("unexpected enum struct field: {:?}", other),
                        location: Some(field_item_loc),
                    });
                }
            }
        }
    }

    Ok(fields)
}

/// Parse comptime block: `comptime { stmts }`
///
/// Grammar: `comptime_block = { "comptime" ~ block_expr }`
/// Produces `Expr::Comptime(Vec<Statement>, Span)`.
fn parse_comptime_block(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let block_pair = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected block after 'comptime'".to_string(),
            location: Some(pair_loc),
        })?;

    // Parse the block_expr into an Expr::Block, then extract statements
    let block_expr = super::control_flow::parse_block_expr(block_pair)?;
    let stmts = block_items_to_statements(block_expr, span);
    Ok(Expr::Comptime(stmts, span))
}

/// Parse comptime for expression: `comptime for field in target.fields { stmts }`
///
/// Grammar: `comptime_for_expr = { "comptime" ~ "for" ~ ident ~ "in" ~ postfix_expr ~ "{" ~ statement* ~ "}" }`
/// Produces `Expr::ComptimeFor(ComptimeForExpr, Span)`.
fn parse_comptime_for_expr(pair: Pair<Rule>) -> Result<Expr> {
    use crate::ast::expr_helpers::ComptimeForExpr;
    use crate::parser::statements::parse_statement;

    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse loop variable name
    let var_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected loop variable in comptime for".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let variable = var_pair.as_str().to_string();

    // Parse iterable expression (postfix_expr in grammar)
    let iter_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected iterable expression in comptime for".to_string(),
        location: Some(pair_loc),
    })?;
    let iterable = parse_postfix_expr(iter_pair)?;

    // Parse body statements
    let mut body = Vec::new();
    for stmt_pair in inner {
        if stmt_pair.as_rule() == Rule::statement {
            body.push(parse_statement(stmt_pair)?);
        }
    }

    Ok(Expr::ComptimeFor(
        Box::new(ComptimeForExpr {
            variable,
            iterable: Box::new(iterable),
            body,
        }),
        span,
    ))
}

/// Convert a Block expression (from block_expr parsing) into a Vec<Statement>.
/// The last expression in a block becomes a return statement so the comptime
/// mini-VM can capture the result.
pub(crate) fn block_items_to_statements(
    block_expr: Expr,
    span: Span,
) -> Vec<crate::ast::Statement> {
    use crate::ast::{BlockItem, Statement};

    let items = match block_expr {
        Expr::Block(block, _) => block.items,
        // If it's a single expression (empty block rules can produce this), wrap it
        other => return vec![Statement::Return(Some(other), span)],
    };

    let mut stmts = Vec::new();
    let len = items.len();
    for (i, item) in items.into_iter().enumerate() {
        let is_last = i == len - 1;
        match item {
            BlockItem::VariableDecl(decl) => {
                stmts.push(Statement::VariableDecl(decl, span));
            }
            BlockItem::Assignment(assign) => {
                stmts.push(Statement::Assignment(assign, span));
            }
            BlockItem::Statement(stmt) => {
                stmts.push(stmt);
            }
            BlockItem::Expression(expr) => {
                if is_last {
                    // Last expression becomes the return value
                    stmts.push(Statement::Return(Some(expr), span));
                } else {
                    stmts.push(Statement::Expression(expr, span));
                }
            }
        }
    }
    stmts
}

/// Parse annotated expression: `@annotation expr`
///
/// Grammar: `annotated_expr = { annotation+ ~ postfix_expr }`
/// Produces nested `Expr::Annotated { annotation, target, span }`.
/// Multiple annotations nest left-to-right:
/// `@retry(3) @timeout(5s) fetch()` becomes
/// `Annotated { @retry(3), target: Annotated { @timeout(5s), target: fetch() } }`
fn parse_annotated_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    if inner_pairs.is_empty() {
        return Err(ShapeError::ParseError {
            message: "expected annotations and expression".to_string(),
            location: Some(pair_loc),
        });
    }

    // Collect annotations (all but the last pair which is the target expression)
    let mut annotations = Vec::new();
    let mut target_pair = None;
    for p in &inner_pairs {
        match p.as_rule() {
            Rule::annotation => {
                annotations.push(crate::parser::functions::parse_annotation(p.clone())?);
            }
            _ => {
                target_pair = Some(p.clone());
            }
        }
    }

    let target_pair = target_pair.ok_or_else(|| ShapeError::ParseError {
        message: "expected expression after annotations".to_string(),
        location: Some(pair_loc),
    })?;
    let target = parse_postfix_expr(target_pair)?;

    // Wrap the target in nested Annotated expressions (right-to-left)
    // @a @b expr -> Annotated(@a, Annotated(@b, expr))
    let mut result = target;
    for annotation in annotations.into_iter().rev() {
        result = Expr::Annotated {
            annotation,
            target: Box::new(result),
            span,
        };
    }

    Ok(result)
}

// Note: backtest { } block syntax has been removed.
// Use backtest(strategy, { config }) function instead.
