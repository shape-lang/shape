//! AST parser for language extensions

use crate::ast::{
    AnnotationDef, AnnotationHandler, AnnotationHandlerParam, AnnotationHandlerType,
    FunctionParameter,
};
use crate::error::{Result, ShapeError};
use crate::parser::Rule;
use crate::parser::expressions::control_flow::parse_block_expr;
use crate::parser::pair_span;
use crate::parser::types::parse_type_annotation;
use pest::iterators::Pair;

/// Parse an annotation definition with lifecycle handlers
///
/// Grammar:
/// ```pest
/// annotation_def = {
///     "annotation" ~ ident ~ "(" ~ annotation_def_params? ~ ")" ~ "{" ~ annotation_body ~ "}"
/// }
/// annotation_body = { annotation_handler* }
/// annotation_handler = {
///     annotation_handler_name ~ "(" ~ annotation_handler_params? ~ ")" ~ return_type? ~ block_expr
/// }
/// ```
pub fn parse_annotation_def(pair: Pair<Rule>) -> Result<AnnotationDef> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing annotation name".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();
    let name_span = pair_span(&name_pair);

    let mut params: Vec<FunctionParameter> = Vec::new();
    let mut handlers = Vec::new();
    let mut allowed_targets: Option<Vec<crate::ast::AnnotationTargetKind>> = None;

    for part in inner {
        match part.as_rule() {
            Rule::annotation_def_params => {
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::ident {
                        let pattern = crate::ast::DestructurePattern::Identifier(
                            param_pair.as_str().to_string(),
                            pair_span(&param_pair),
                        );
                        params.push(FunctionParameter {
                            pattern,
                            is_const: false,
                            is_reference: false,
                            is_mut_reference: false,
                            type_annotation: None,
                            default_value: None,
                        });
                    }
                }
            }
            Rule::annotation_body => {
                for body_item in part.into_inner() {
                    let item = if body_item.as_rule() == Rule::annotation_body_item {
                        body_item
                            .into_inner()
                            .next()
                            .ok_or_else(|| ShapeError::ParseError {
                                message: "empty annotation body item".to_string(),
                                location: None,
                            })?
                    } else {
                        body_item
                    };

                    match item.as_rule() {
                        Rule::annotation_handler => {
                            handlers.push(parse_annotation_handler(item)?);
                        }
                        Rule::annotation_targets_decl => {
                            let mut targets = Vec::new();
                            for target_pair in item.into_inner() {
                                if target_pair.as_rule() != Rule::annotation_target_kind {
                                    continue;
                                }
                                let kind = match target_pair.as_str() {
                                    "function" => crate::ast::AnnotationTargetKind::Function,
                                    "type" => crate::ast::AnnotationTargetKind::Type,
                                    "module" => crate::ast::AnnotationTargetKind::Module,
                                    "expression" => crate::ast::AnnotationTargetKind::Expression,
                                    "block" => crate::ast::AnnotationTargetKind::Block,
                                    "await_expr" => crate::ast::AnnotationTargetKind::AwaitExpr,
                                    "binding" => crate::ast::AnnotationTargetKind::Binding,
                                    other => {
                                        return Err(ShapeError::ParseError {
                                            message: format!(
                                                "unknown annotation target kind '{}'",
                                                other
                                            ),
                                            location: None,
                                        });
                                    }
                                };
                                targets.push(kind);
                            }
                            allowed_targets = Some(targets);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(AnnotationDef {
        name,
        name_span,
        doc_comment: None,
        params,
        allowed_targets,
        handlers,
        span,
    })
}

/// Parse a single annotation lifecycle handler
fn parse_annotation_handler(pair: Pair<Rule>) -> Result<AnnotationHandler> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    // Parse handler type (on_define, before, after, metadata, comptime pre/post)
    let handler_name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing annotation handler name".to_string(),
        location: None,
    })?;

    let handler_kind = handler_name_pair
        .as_str()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let handler_type = match handler_kind.as_str() {
        "on_define" => AnnotationHandlerType::OnDefine,
        "before" => AnnotationHandlerType::Before,
        "after" => AnnotationHandlerType::After,
        "metadata" => AnnotationHandlerType::Metadata,
        "comptime pre" => AnnotationHandlerType::ComptimePre,
        "comptime post" => AnnotationHandlerType::ComptimePost,
        other => {
            return Err(ShapeError::ParseError {
                message: format!(
                    "unknown annotation handler type '{}'. Expected `on_define`, `before`, `after`, `metadata`, `comptime pre`, or `comptime post`",
                    other
                ),
                location: None,
            });
        }
    };

    // Parse handler parameters
    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = None;

    for part in inner {
        match part.as_rule() {
            Rule::annotation_handler_params => {
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::annotation_handler_param {
                        let raw = param_pair.as_str().trim();
                        let (name, is_variadic) = if let Some(rest) = raw.strip_prefix("...") {
                            (rest.trim().to_string(), true)
                        } else {
                            (raw.to_string(), false)
                        };
                        if !name.is_empty() {
                            params.push(AnnotationHandlerParam { name, is_variadic });
                        }
                    }
                }
            }
            Rule::return_type => {
                // return_type = "->" ~ type_annotation
                if let Some(type_pair) = part.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            Rule::block_expr => {
                body = Some(parse_block_expr(part)?);
            }
            _ => {}
        }
    }

    let body = body.ok_or_else(|| ShapeError::ParseError {
        message: "Missing annotation handler body".to_string(),
        location: None,
    })?;

    Ok(AnnotationHandler {
        handler_type,
        params,
        return_type,
        body,
        span,
    })
}

pub fn parse_extend_statement(pair: Pair<Rule>) -> Result<crate::ast::ExtendStatement> {
    use crate::ast::types::TypeName;

    let mut inner = pair.into_inner();

    // Parse type_name: ident ~ ("<" ~ type_annotation ~ ">")?
    let type_name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing type name in extend statement".to_string(),
        location: None,
    })?;

    let type_name = {
        let mut tn_inner = type_name_pair.into_inner();
        let name_pair = tn_inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "Missing type name identifier".to_string(),
            location: None,
        })?;
        let name = name_pair.as_str().to_string();
        let type_args: Vec<_> = tn_inner
            .map(|p| parse_type_annotation(p))
            .collect::<Result<Vec<_>>>()?;
        if type_args.is_empty() {
            TypeName::Simple(name)
        } else {
            TypeName::Generic { name, type_args }
        }
    };

    // Parse method_def*
    let mut methods = Vec::new();
    for method_pair in inner {
        if method_pair.as_rule() == Rule::method_def {
            methods.push(super::types::parse_method_def_shared(method_pair)?);
        }
    }

    Ok(crate::ast::ExtendStatement { type_name, methods })
}

/// Parse an impl block: `impl TraitName for TypeName [as ImplName] { method_def* }`
///
/// Reuses the same method_def parsing as extend blocks.
pub fn parse_impl_block(pair: Pair<Rule>) -> Result<crate::ast::ImplBlock> {
    let mut inner = pair.into_inner();

    // First type_name is the trait being implemented
    let trait_name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing trait name in impl block".to_string(),
        location: None,
    })?;
    let trait_name = parse_type_name(trait_name_pair)?;

    // Second type_name is the target type
    let target_type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing target type in impl block".to_string(),
        location: None,
    })?;
    let target_type = parse_type_name(target_type_pair)?;

    // Parse optional impl_name, where_clause and impl_member*
    let mut impl_name = None;
    let mut methods = Vec::new();
    let mut associated_type_bindings = Vec::new();
    let mut where_clause = None;
    for member_pair in inner {
        if member_pair.as_rule() == Rule::impl_name {
            let name_pair =
                member_pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| ShapeError::ParseError {
                        message: "Missing impl name after 'as'".to_string(),
                        location: None,
                    })?;
            impl_name = Some(name_pair.as_str().to_string());
            continue;
        }
        if member_pair.as_rule() == Rule::where_clause {
            where_clause = Some(super::functions::parse_where_clause(member_pair)?);
            continue;
        }
        if member_pair.as_rule() != Rule::impl_member {
            continue;
        }
        for child in member_pair.into_inner() {
            match child.as_rule() {
                Rule::associated_type_binding => {
                    let binding = parse_associated_type_binding(child)?;
                    associated_type_bindings.push(binding);
                }
                Rule::method_def => {
                    methods.push(super::types::parse_method_def_shared(child)?);
                }
                _ => {}
            }
        }
    }

    Ok(crate::ast::ImplBlock {
        trait_name,
        target_type,
        impl_name,
        methods,
        associated_type_bindings,
        where_clause,
    })
}

/// Parse `type Item = number;`
fn parse_associated_type_binding(
    pair: Pair<Rule>,
) -> Result<crate::ast::types::AssociatedTypeBinding> {
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected associated type name in binding".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();

    let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: format!("expected type annotation for associated type '{}'", name),
        location: None,
    })?;
    let concrete_type = super::types::parse_type_annotation(type_pair)?;

    Ok(crate::ast::types::AssociatedTypeBinding {
        name,
        concrete_type,
    })
}

/// Parse a type_name from the grammar (ident with optional generic args)
fn parse_type_name(pair: Pair<Rule>) -> Result<crate::ast::types::TypeName> {
    use crate::ast::types::TypeName;

    let mut tn_inner = pair.into_inner();
    let name_pair = tn_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing type name identifier".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();
    let type_args: Vec<_> = tn_inner
        .map(|p| super::types::parse_type_annotation(p))
        .collect::<Result<Vec<_>>>()?;
    if type_args.is_empty() {
        Ok(TypeName::Simple(name))
    } else {
        Ok(TypeName::Generic { name, type_args })
    }
}

pub fn parse_optimize_statement(_pair: Pair<Rule>) -> Result<crate::ast::OptimizeStatement> {
    Err(ShapeError::ParseError {
        message: "parse_optimize_statement not yet implemented".to_string(),
        location: None,
    })
}
