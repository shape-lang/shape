//! Pattern matching expression parsing
//!
//! This module handles parsing of pattern matching expressions:
//! - Match expressions
//! - Pattern parsing (wildcard, identifier, literal, array, object, constructor)

use crate::ast::{Expr, MatchArm, MatchExpr, Pattern};
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use pest::iterators::Pair;

use super::super::super::pair_span;
use crate::parser::pair_location;

/// Parse match expression
pub fn parse_match_expr(pair: Pair<Rule>) -> Result<Expr> {
    let span = pair_span(&pair);
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let scrutinee_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression to match against".to_string(),
        location: Some(pair_loc),
    })?;
    let scrutinee = parse_match_scrutinee(scrutinee_pair)?;
    let mut arms = Vec::new();

    for arm_pair in inner {
        if arm_pair.as_rule() == Rule::match_arm {
            let arm_inner_pairs: Vec<_> = arm_pair.into_inner().collect();
            let pattern_span = Some(pair_span(&arm_inner_pairs[0]));
            let pattern = parse_pattern(arm_inner_pairs[0].clone())?;

            let mut guard = None;
            let mut body = None;

            // Process remaining pairs
            for i in 1..arm_inner_pairs.len() {
                let next = &arm_inner_pairs[i];
                if next.as_rule() == Rule::expression {
                    if body.is_none() && guard.is_some() {
                        body = Some(super::super::parse_expression(next.clone())?);
                    } else if guard.is_none() {
                        // This might be a guard or the body
                        if i < arm_inner_pairs.len() - 1 {
                            guard = Some(Box::new(super::super::parse_expression(next.clone())?));
                        } else {
                            body = Some(super::super::parse_expression(next.clone())?);
                        }
                    }
                }
            }

            let body = body.ok_or_else(|| ShapeError::ParseError {
                message: "Match arm missing body".to_string(),
                location: None,
            })?;

            arms.push(MatchArm {
                pattern,
                guard,
                body: Box::new(body),
                pattern_span,
            });
        }
    }

    Ok(Expr::Match(
        Box::new(MatchExpr {
            scrutinee: Box::new(scrutinee),
            arms,
        }),
        span,
    ))
}

fn parse_match_scrutinee(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::match_scrutinee => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected match scrutinee".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_match_scrutinee(inner)
        }
        Rule::match_scrutinee_ident => {
            let ident_pair = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected identifier in match scrutinee".to_string(),
                    location: Some(pair_loc),
                })?;
            Ok(Expr::Identifier(
                ident_pair.as_str().to_string(),
                pair_span(&ident_pair),
            ))
        }
        Rule::ident => Ok(Expr::Identifier(
            pair.as_str().to_string(),
            pair_span(&pair),
        )),
        Rule::expression => super::super::parse_expression(pair),
        _ => super::super::parse_expression(pair),
    }
}

/// Parse pattern
pub fn parse_pattern(pair: Pair<Rule>) -> Result<Pattern> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::pattern => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected pattern content".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_pattern(inner)
        }
        Rule::pattern_wildcard => Ok(Pattern::Wildcard),
        Rule::pattern_typed => {
            let mut inner = pair.into_inner();
            let name = inner
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected identifier in typed pattern".to_string(),
                    location: Some(pair_loc.clone()),
                })?
                .as_str()
                .to_string();
            let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation in typed pattern".to_string(),
                location: Some(pair_loc),
            })?;
            let type_annotation = crate::parser::parse_type_annotation(type_pair)?;
            Ok(Pattern::Typed {
                name,
                type_annotation,
            })
        }
        Rule::pattern_identifier => Ok(Pattern::Identifier(pair.as_str().to_string())),
        Rule::pattern_literal => {
            let literal_pair = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected literal in pattern".to_string(),
                    location: Some(pair_loc.clone()),
                })?;
            let literal = super::super::literals::parse_literal(literal_pair)?;
            match literal {
                Expr::Literal(lit, _) => Ok(Pattern::Literal(lit)),
                _ => Err(ShapeError::ParseError {
                    message: "expected literal in pattern".to_string(),
                    location: Some(pair_loc),
                }),
            }
        }
        Rule::pattern_array => {
            let mut patterns = Vec::new();
            for inner in pair.into_inner() {
                patterns.push(parse_pattern(inner)?);
            }
            Ok(Pattern::Array(patterns))
        }
        Rule::pattern_object => {
            let mut fields = Vec::new();
            for field in pair.into_inner() {
                if field.as_rule() == Rule::pattern_field {
                    let mut field_inner = field.into_inner();
                    let name = field_inner
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected field name in object pattern".to_string(),
                            location: Some(pair_loc.clone()),
                        })?
                        .as_str()
                        .to_string();
                    // Shorthand: `{x, y}` is equivalent to `{x: x, y: y}`
                    let pattern = if let Some(pattern_pair) = field_inner.next() {
                        parse_pattern(pattern_pair)?
                    } else {
                        Pattern::Identifier(name.clone())
                    };
                    fields.push((name, pattern));
                }
            }
            Ok(Pattern::Object(fields))
        }
        Rule::pattern_constructor => {
            let mut inner = pair.into_inner();
            let ctor_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected constructor pattern".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            parse_constructor_pattern(ctor_pair)
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected pattern rule: {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}

fn parse_constructor_pattern(pair: Pair<Rule>) -> Result<Pattern> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::pattern_qualified_constructor => {
            let inner = pair.into_inner();
            let mut ident_segments = Vec::new();
            let mut payload_pair = None;
            for child in inner {
                match child.as_rule() {
                    Rule::ident | Rule::variant_ident => {
                        ident_segments.push(child.as_str().to_string())
                    }
                    Rule::pattern_constructor_payload => payload_pair = Some(child),
                    _ => {}
                }
            }
            if ident_segments.len() < 2 {
                return Err(ShapeError::ParseError {
                    message: "expected Enum::Variant in constructor pattern".to_string(),
                    location: Some(pair_loc),
                });
            }
            let variant = ident_segments.pop().unwrap();
            let enum_path = if ident_segments.len() == 1 {
                crate::ast::TypePath::simple(ident_segments.remove(0))
            } else {
                crate::ast::TypePath::from_segments(ident_segments)
            };
            let fields = if let Some(payload) = payload_pair {
                parse_constructor_payload(payload)?
            } else {
                crate::ast::PatternConstructorFields::Unit
            };
            Ok(Pattern::Constructor {
                enum_name: Some(enum_path),
                variant,
                fields,
            })
        }
        Rule::pattern_unqualified_constructor => {
            let mut inner = pair.into_inner();
            let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected constructor name in pattern".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let variant = match name_pair.as_rule() {
                Rule::pattern_constructor_name => name_pair
                    .clone()
                    .into_inner()
                    .next()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_else(|| name_pair.as_str().to_string()),
                Rule::pattern_constructor_keyword => name_pair.as_str().to_string(),
                _ => name_pair.as_str().to_string(),
            };
            let fields = if let Some(payload_pair) = inner.next() {
                parse_constructor_payload(payload_pair)?
            } else {
                crate::ast::PatternConstructorFields::Unit
            };
            Ok(Pattern::Constructor {
                enum_name: None,
                variant,
                fields,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected constructor pattern rule: {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}

fn parse_constructor_payload(pair: Pair<Rule>) -> Result<crate::ast::PatternConstructorFields> {
    let pair_loc = pair_location(&pair);
    match pair.as_rule() {
        Rule::pattern_constructor_payload => {
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected constructor payload".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_constructor_payload(inner)
        }
        Rule::pattern_constructor_tuple => {
            let mut patterns = Vec::new();
            for inner in pair.into_inner() {
                patterns.push(parse_pattern(inner)?);
            }
            Ok(crate::ast::PatternConstructorFields::Tuple(patterns))
        }
        Rule::pattern_constructor_struct => {
            let mut fields = Vec::new();
            for field in pair.into_inner() {
                if field.as_rule() == Rule::pattern_field {
                    let field_loc = pair_location(&field);
                    let mut field_inner = field.into_inner();
                    let name = field_inner
                        .next()
                        .ok_or_else(|| ShapeError::ParseError {
                            message: "expected field name in constructor pattern".to_string(),
                            location: Some(field_loc.clone()),
                        })?
                        .as_str()
                        .to_string();
                    let pattern = if let Some(pattern_pair) = field_inner.next() {
                        parse_pattern(pattern_pair)?
                    } else {
                        // Shorthand: { radius } == { radius: radius }
                        crate::ast::Pattern::Identifier(name.clone())
                    };
                    fields.push((name, pattern));
                }
            }
            Ok(crate::ast::PatternConstructorFields::Struct(fields))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected constructor payload rule: {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}
