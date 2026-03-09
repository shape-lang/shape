//! Function and annotation parsing for Shape
//!
//! This module handles parsing of:
//! - Function definitions with parameters and return types
//! - Function parameters with default values
//! - Annotations (@warmup, @strategy, etc.)

use crate::ast::{
    Annotation, BuiltinFunctionDecl, ForeignFunctionDef, FunctionDef, FunctionParameter,
    NativeAbiBinding,
};
use crate::error::Result;
use pest::iterators::Pair;

use super::expressions;
use super::statements;
use super::string_literals::parse_string_literal;
use super::types;
use super::types::parse_type_annotation;
use super::{Rule, pair_span};

/// Parse annotations
pub fn parse_annotations(pair: Pair<Rule>) -> Result<Vec<Annotation>> {
    let mut annotations = vec![];

    for annotation_pair in pair.into_inner() {
        if annotation_pair.as_rule() == Rule::annotation {
            annotations.push(parse_annotation(annotation_pair)?);
        }
    }

    Ok(annotations)
}

/// Parse a single annotation
pub fn parse_annotation(pair: Pair<Rule>) -> Result<Annotation> {
    let span = pair_span(&pair);
    let mut name = String::new();
    let mut args = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::annotation_name | Rule::ident => {
                name = inner_pair.as_str().to_string();
            }
            Rule::annotation_args => {
                for arg_pair in inner_pair.into_inner() {
                    if arg_pair.as_rule() == Rule::expression {
                        args.push(expressions::parse_expression(arg_pair)?);
                    }
                }
            }
            Rule::expression => {
                args.push(expressions::parse_expression(inner_pair)?);
            }
            _ => {}
        }
    }

    Ok(Annotation { name, args, span })
}

/// Parse a function parameter
pub fn parse_function_param(pair: Pair<Rule>) -> Result<FunctionParameter> {
    let mut pattern = None;
    let mut is_const = false;
    let mut is_reference = false;
    let mut is_mut_reference = false;
    let mut is_out = false;
    let mut type_annotation = None;
    let mut default_value = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::param_const_keyword => {
                is_const = true;
            }
            Rule::param_ref_keyword => {
                is_reference = true;
                // Check for &mut: param_ref_keyword contains optional param_mut_keyword
                for child in inner_pair.into_inner() {
                    if child.as_rule() == Rule::param_mut_keyword {
                        is_mut_reference = true;
                    }
                }
            }
            Rule::param_out_keyword => {
                is_out = true;
            }
            Rule::destructure_pattern => {
                pattern = Some(super::items::parse_pattern(inner_pair)?);
            }
            Rule::type_annotation => {
                type_annotation = Some(parse_type_annotation(inner_pair)?);
            }
            Rule::expression => {
                default_value = Some(expressions::parse_expression(inner_pair)?);
            }
            _ => {}
        }
    }

    let pattern = pattern.ok_or_else(|| crate::error::ShapeError::ParseError {
        message: "expected pattern in function parameter".to_string(),
        location: None,
    })?;

    Ok(FunctionParameter {
        pattern,
        is_const,
        is_reference,
        is_mut_reference,
        is_out,
        type_annotation,
        default_value,
    })
}

/// Parse a function definition
pub fn parse_function_def(pair: Pair<Rule>) -> Result<FunctionDef> {
    let mut name = String::new();
    let mut name_span = crate::ast::Span::DUMMY;
    let mut type_params = None;
    let mut params = vec![];
    let mut return_type = None;
    let mut where_clause = None;
    let mut body = vec![];
    let mut annotations = vec![];
    let mut is_async = false;
    let mut is_comptime = false;

    // Parse all parts sequentially (can't use find() as it consumes the iterator)
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::annotations => {
                annotations = parse_annotations(inner_pair)?;
            }
            Rule::async_keyword => {
                is_async = true;
            }
            Rule::comptime_keyword => {
                is_comptime = true;
            }
            Rule::ident => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                    name_span = pair_span(&inner_pair);
                }
            }
            Rule::type_params => {
                type_params = Some(types::parse_type_params(inner_pair)?);
            }
            Rule::function_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::return_type => {
                // Skip the "->" and get the type annotation
                if let Some(type_pair) = inner_pair.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            Rule::where_clause => {
                where_clause = Some(parse_where_clause(inner_pair)?);
            }
            Rule::function_body => {
                // Parse all statements in the function body
                body = statements::parse_statements(inner_pair.into_inner())?;
            }
            _ => {}
        }
    }

    Ok(FunctionDef {
        name,
        name_span,
        doc_comment: None,
        type_params,
        params,
        return_type,
        where_clause,
        body,
        annotations,
        is_async,
        is_comptime,
    })
}

/// Parse a declaration-only builtin function definition.
///
/// Grammar:
/// `builtin fn name<T>(params...) -> ReturnType;`
pub fn parse_builtin_function_decl(pair: Pair<Rule>) -> Result<BuiltinFunctionDecl> {
    let mut name = String::new();
    let mut name_span = crate::ast::Span::DUMMY;
    let mut type_params = None;
    let mut params = vec![];
    let mut return_type = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::ident => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                    name_span = pair_span(&inner_pair);
                }
            }
            Rule::type_params => {
                type_params = Some(types::parse_type_params(inner_pair)?);
            }
            Rule::function_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::return_type => {
                if let Some(type_pair) = inner_pair.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            _ => {}
        }
    }

    let return_type = return_type.ok_or_else(|| crate::error::ShapeError::ParseError {
        message: "builtin function declaration requires an explicit return type".to_string(),
        location: None,
    })?;

    Ok(BuiltinFunctionDecl {
        name,
        name_span,
        doc_comment: None,
        type_params,
        params,
        return_type,
    })
}

/// Parse a foreign function definition: `fn python analyze(data: DataTable) -> number { ... }`
pub fn parse_foreign_function_def(pair: Pair<Rule>) -> Result<ForeignFunctionDef> {
    let mut language = String::new();
    let mut language_span = crate::ast::Span::DUMMY;
    let mut name = String::new();
    let mut name_span = crate::ast::Span::DUMMY;
    let mut type_params = None;
    let mut params = vec![];
    let mut return_type = None;
    let mut body_text = String::new();
    let mut body_span = crate::ast::Span::DUMMY;
    let mut annotations = vec![];
    let mut is_async = false;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::annotations => {
                annotations = parse_annotations(inner_pair)?;
            }
            Rule::async_keyword => {
                is_async = true;
            }
            Rule::function_keyword => {}
            Rule::foreign_language_id => {
                language = inner_pair.as_str().to_string();
                language_span = pair_span(&inner_pair);
            }
            Rule::ident => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                    name_span = pair_span(&inner_pair);
                }
            }
            Rule::type_params => {
                type_params = Some(types::parse_type_params(inner_pair)?);
            }
            Rule::function_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::return_type => {
                if let Some(type_pair) = inner_pair.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            Rule::foreign_body => {
                body_span = pair_span(&inner_pair);
                body_text = dedent_foreign_body(inner_pair.as_str());
            }
            _ => {}
        }
    }

    Ok(ForeignFunctionDef {
        language,
        language_span,
        name,
        name_span,
        doc_comment: None,
        type_params,
        params,
        return_type,
        body_text,
        body_span,
        annotations,
        is_async,
        native_abi: None,
    })
}

/// Parse a native ABI declaration:
/// `extern "C" fn name(args...) -> Ret from "library" [as "symbol"];`
pub fn parse_extern_native_function_def(pair: Pair<Rule>) -> Result<ForeignFunctionDef> {
    let mut abi = String::new();
    let mut abi_span = crate::ast::Span::DUMMY;
    let mut name = String::new();
    let mut name_span = crate::ast::Span::DUMMY;
    let mut type_params = None;
    let mut params = Vec::new();
    let mut return_type = None;
    let mut library: Option<String> = None;
    let mut symbol: Option<String> = None;
    let mut annotations = Vec::new();
    let mut is_async = false;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::annotations => {
                annotations = parse_annotations(inner_pair)?;
            }
            Rule::async_keyword => {
                is_async = true;
            }
            Rule::extern_abi => {
                abi_span = pair_span(&inner_pair);
                abi = parse_extern_abi(inner_pair)?;
            }
            Rule::function_keyword => {}
            Rule::ident => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                    name_span = pair_span(&inner_pair);
                }
            }
            Rule::type_params => {
                type_params = Some(types::parse_type_params(inner_pair)?);
            }
            Rule::function_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::return_type => {
                if let Some(type_pair) = inner_pair.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            Rule::extern_native_link => {
                for link_part in inner_pair.into_inner() {
                    match link_part.as_rule() {
                        Rule::extern_native_library => {
                            library = Some(parse_string_literal(link_part.as_str())?);
                        }
                        Rule::extern_native_symbol => {
                            symbol = Some(parse_string_literal(link_part.as_str())?);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let library = library.ok_or_else(|| crate::error::ShapeError::ParseError {
        message: "extern native declaration requires `from \"library\"`".to_string(),
        location: None,
    })?;

    if abi.trim() != "C" {
        return Err(crate::error::ShapeError::ParseError {
            message: format!(
                "unsupported extern ABI '{}': only \"C\" is currently supported",
                abi
            ),
            location: None,
        });
    }

    let symbol = symbol.unwrap_or_else(|| name.clone());

    Ok(ForeignFunctionDef {
        // Keep foreign-language compatibility for downstream compilation/runtime
        // while carrying explicit native ABI metadata.
        language: "native".to_string(),
        language_span: abi_span,
        name,
        name_span,
        doc_comment: None,
        type_params,
        params,
        return_type,
        body_text: String::new(),
        body_span: crate::ast::Span::DUMMY,
        annotations,
        is_async,
        native_abi: Some(NativeAbiBinding {
            abi,
            library,
            symbol,
            package_key: None,
        }),
    })
}

pub(crate) fn parse_extern_abi(pair: Pair<Rule>) -> Result<String> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| crate::error::ShapeError::ParseError {
            message: "extern declaration is missing ABI name".to_string(),
            location: None,
        })?;

    match inner.as_rule() {
        Rule::string => parse_string_literal(inner.as_str()),
        Rule::ident => Ok(inner.as_str().to_string()),
        _ => Err(crate::error::ShapeError::ParseError {
            message: format!("unsupported extern ABI token: {:?}", inner.as_rule()),
            location: None,
        }),
    }
}

/// Strip common leading whitespace from foreign body text.
///
/// Similar to Python's `textwrap.dedent`. This is critical for Python blocks
/// since the body is indented inside Shape code but needs to be dedented
/// for the foreign language runtime.
///
/// Note: The Pest parser's implicit WHITESPACE rule consumes the newline and
/// leading whitespace between `{` and the first token of `foreign_body`. This
/// means the first line has its leading whitespace eaten by the parser, while
/// subsequent lines retain their original indentation. We compute `min_indent`
/// from lines after the first, then strip that amount only from those lines.
/// The first line is kept as-is.
fn dedent_foreign_body(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    if lines.len() == 1 {
        return lines[0].trim_start().to_string();
    }

    // Compute min_indent from lines after the first, since the parser already
    // consumed the first line's leading whitespace.
    let min_indent = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // First line: keep as-is (parser already stripped its whitespace).
    // Subsequent lines: strip min_indent characters.
    let mut result = Vec::with_capacity(lines.len());
    result.push(lines[0]);
    for line in &lines[1..] {
        if line.len() >= min_indent {
            result.push(&line[min_indent..]);
        } else {
            result.push(line.trim());
        }
    }
    result.join("\n")
}

/// Parse a where clause: `where T: Bound1 + Bound2, U: Bound3`
pub fn parse_where_clause(pair: Pair<Rule>) -> Result<Vec<crate::ast::types::WherePredicate>> {
    let mut predicates = Vec::new();
    for child in pair.into_inner() {
        if child.as_rule() == Rule::where_predicate {
            predicates.push(parse_where_predicate(child)?);
        }
    }
    Ok(predicates)
}

fn parse_where_predicate(pair: Pair<Rule>) -> Result<crate::ast::types::WherePredicate> {
    let mut inner = pair.into_inner();

    let name_pair = inner
        .next()
        .ok_or_else(|| crate::error::ShapeError::ParseError {
            message: "expected type parameter name in where predicate".to_string(),
            location: None,
        })?;
    let type_name = name_pair.as_str().to_string();

    let mut bounds = Vec::new();
    for remaining in inner {
        if remaining.as_rule() == Rule::trait_bound_list {
            for bound_ident in remaining.into_inner() {
                if bound_ident.as_rule() == Rule::ident {
                    bounds.push(bound_ident.as_str().to_string());
                }
            }
        }
    }

    Ok(crate::ast::types::WherePredicate { type_name, bounds })
}
