//! Item parsing for Shape
//!
//! This module handles parsing of:
//! - Variable declarations (let, var, const)
//! - Destructuring patterns (identifiers, arrays, objects, rest)

use crate::ast::{
    DecompositionBinding, DestructurePattern, ObjectPatternField, ObjectTypeField,
    OwnershipModifier, TypeAnnotation, VarKind, VariableDecl,
};
use crate::error::{Result, ShapeError};
use pest::iterators::Pair;

use super::expressions;
use super::types::parse_type_annotation;
use super::{Rule, pair_location, pair_span};

/// Parse a variable declaration
pub fn parse_variable_decl(pair: Pair<Rule>) -> Result<VariableDecl> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse variable kind (let, var, const)
    let keyword_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected variable declaration keyword".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("use 'let', 'var', or 'const' to declare a variable"),
        ),
    })?;
    let kind_str = keyword_pair.as_str();
    let kind = match kind_str {
        "let" => VarKind::Let,
        "var" => VarKind::Var,
        "const" => VarKind::Const,
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("invalid variable declaration kind: '{}'", kind_str),
                location: Some(
                    pair_location(&keyword_pair).with_hint("use 'let', 'var', or 'const'"),
                ),
            });
        }
    };

    // Parse optional 'mut' modifier
    let next_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected variable name or pattern after keyword".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("provide a variable name, e.g., 'let x = 5'"),
        ),
    })?;
    let (is_mut, pattern_pair) = if next_pair.as_rule() == Rule::var_mut_modifier {
        let p = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected variable name or pattern after 'mut'".to_string(),
            location: Some(pair_loc.with_hint("provide a variable name, e.g., 'let mut x = 5'")),
        })?;
        (true, p)
    } else {
        (false, next_pair)
    };
    let pattern = parse_pattern(pattern_pair)?;

    // Parse optional type annotation, ownership modifier, and value
    let mut type_annotation = None;
    let mut value = None;
    let mut ownership = OwnershipModifier::Inferred;

    for pair in inner {
        match pair.as_rule() {
            Rule::type_annotation => {
                type_annotation = Some(parse_type_annotation(pair)?);
            }
            Rule::ownership_modifier => {
                ownership = match pair.as_str() {
                    "move" => OwnershipModifier::Move,
                    "clone" => OwnershipModifier::Clone,
                    _ => OwnershipModifier::Inferred,
                };
            }
            Rule::expression => {
                value = Some(expressions::parse_expression(pair)?);
            }
            _ => {}
        }
    }

    Ok(VariableDecl {
        kind,
        is_mut,
        pattern,
        type_annotation,
        value,
        ownership,
    })
}

/// Parse a pattern for destructuring
pub fn parse_pattern(pair: Pair<Rule>) -> Result<DestructurePattern> {
    let pair_loc = pair_location(&pair);

    match pair.as_rule() {
        Rule::destructure_pattern => {
            // Pattern is a wrapper, get the inner pattern
            let inner = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected pattern content".to_string(),
                    location: Some(pair_loc),
                })?;
            parse_pattern(inner)
        }
        Rule::destructure_ident_pattern => {
            let ident = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected identifier in pattern".to_string(),
                    location: Some(pair_loc),
                })?;
            let ident_span = pair_span(&ident);
            Ok(DestructurePattern::Identifier(
                ident.as_str().to_string(),
                ident_span,
            ))
        }
        Rule::destructure_array_pattern => {
            let mut patterns = Vec::new();
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::destructure_pattern {
                    patterns.push(parse_pattern(inner)?);
                }
            }
            Ok(DestructurePattern::Array(patterns))
        }
        Rule::destructure_object_pattern => {
            let mut fields = Vec::new();
            for field in pair.into_inner() {
                if field.as_rule() == Rule::destructure_object_pattern_field {
                    let field_loc = pair_location(&field);
                    let field_str = field.as_str();
                    let mut field_inner = field.into_inner();
                    if field_str.trim_start().starts_with("...") {
                        // Rest pattern
                        let ident_pair =
                            field_inner.next().ok_or_else(|| ShapeError::ParseError {
                                message: "expected identifier after '...' in object pattern"
                                    .to_string(),
                                location: Some(field_loc),
                            })?;
                        let ident_span = pair_span(&ident_pair);
                        let ident = ident_pair.as_str().to_string();
                        fields.push(ObjectPatternField {
                            key: "...".to_string(),
                            pattern: DestructurePattern::Rest(Box::new(
                                DestructurePattern::Identifier(ident, ident_span),
                            )),
                        });
                    } else {
                        // Regular field
                        let key_pair =
                            field_inner.next().ok_or_else(|| ShapeError::ParseError {
                                message: "expected field name in object pattern".to_string(),
                                location: Some(field_loc),
                            })?;
                        let key_span = pair_span(&key_pair);
                        let key = key_pair.as_str().to_string();
                        let pattern = if let Some(pattern_pair) = field_inner.next() {
                            parse_pattern(pattern_pair)?
                        } else {
                            DestructurePattern::Identifier(key.clone(), key_span)
                        };
                        fields.push(ObjectPatternField { key, pattern });
                    }
                }
            }
            Ok(DestructurePattern::Object(fields))
        }
        Rule::destructure_rest_pattern => {
            let ident_pair = pair
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected identifier after '...' in rest pattern".to_string(),
                    location: Some(pair_loc),
                })?;
            let ident_span = pair_span(&ident_pair);
            let ident = ident_pair.as_str().to_string();
            Ok(DestructurePattern::Rest(Box::new(
                DestructurePattern::Identifier(ident, ident_span),
            )))
        }
        Rule::destructure_decomposition_pattern => {
            // Decomposition pattern: (name: Type, name: Type, ...)
            // Used for extracting components from intersection types
            let mut bindings = Vec::new();
            for binding_pair in pair.into_inner() {
                if binding_pair.as_rule() == Rule::decomposition_binding {
                    let binding_span = pair_span(&binding_pair);
                    let mut binding_inner = binding_pair.into_inner();

                    let name_pair = binding_inner.next().ok_or_else(|| ShapeError::ParseError {
                        message: "expected identifier in decomposition binding".to_string(),
                        location: Some(pair_loc.clone()),
                    })?;
                    let name = name_pair.as_str().to_string();

                    let type_pair = binding_inner.next().ok_or_else(|| ShapeError::ParseError {
                        message: "expected type annotation in decomposition binding".to_string(),
                        location: Some(pair_loc.clone()),
                    })?;
                    let type_annotation = if type_pair.as_rule() == Rule::decomposition_field_set {
                        // Shorthand: {x, y, z} — field names only, types are placeholders
                        let fields = type_pair
                            .into_inner()
                            .filter(|p| p.as_rule() == Rule::ident)
                            .map(|p| ObjectTypeField {
                                name: p.as_str().to_string(),
                                optional: false,
                                type_annotation: TypeAnnotation::Basic("_".into()),
                                annotations: vec![],
                            })
                            .collect();
                        TypeAnnotation::Object(fields)
                    } else {
                        parse_type_annotation(type_pair)?
                    };

                    bindings.push(DecompositionBinding {
                        name,
                        type_annotation,
                        span: binding_span,
                    });
                }
            }

            if bindings.len() < 2 {
                return Err(ShapeError::ParseError {
                    message: "decomposition pattern requires at least 2 bindings".to_string(),
                    location: Some(pair_loc),
                });
            }

            Ok(DestructurePattern::Decomposition(bindings))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("invalid pattern rule: {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}
