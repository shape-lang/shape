//! Type annotation parsing for Shape
//!
//! This module handles parsing of type annotations including:
//! - Basic types (number, string, boolean, etc.)
//! - Complex types (arrays, objects, tuples, functions)
//! - Generic types and union types
//! - Optional types

use crate::ast::{Span, TypeAnnotation};
use crate::error::{Result, ShapeError};
use crate::parser::string_literals::parse_string_literal;
use pest::iterators::Pair;
use std::collections::HashMap;

use super::{Rule, pair_location, pair_span};

/// Parse a type annotation
pub fn parse_type_annotation(pair: Pair<Rule>) -> Result<TypeAnnotation> {
    let pair_loc = pair_location(&pair);

    match pair.as_rule() {
        Rule::type_annotation => {
            let loc = pair_loc.clone();
            let mut inner = pair.into_inner();
            let type_part = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type annotation content".to_string(),
                location: Some(loc.clone()),
            })?;
            parse_type_annotation(type_part)
        }
        Rule::union_type => {
            let mut types = Vec::new();
            for inner in pair.into_inner() {
                types.push(parse_type_annotation(inner)?);
            }
            if types.len() == 1 {
                Ok(types.remove(0))
            } else {
                Ok(TypeAnnotation::Union(types))
            }
        }
        Rule::intersection_type => {
            let mut types = Vec::new();
            for inner in pair.into_inner() {
                types.push(parse_type_annotation(inner)?);
            }
            if types.len() == 1 {
                Ok(types.remove(0))
            } else {
                Ok(TypeAnnotation::Intersection(types))
            }
        }
        Rule::optional_type => {
            let inner = pair
                .clone()
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected type in optional type annotation".to_string(),
                    location: Some(pair_loc),
                })?;
            let mut ty = parse_type_annotation(inner)?;
            if pair.as_str().trim_end().ends_with('?') {
                ty = TypeAnnotation::option(ty);
            }
            Ok(ty)
        }
        Rule::primary_type => {
            let inner = pair
                .clone()
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected type in primary type annotation".to_string(),
                    location: Some(pair_loc),
                })?;
            let mut ty = parse_type_annotation(inner)?;
            let mut remaining = pair.as_str().trim();
            while remaining.ends_with("[]") {
                ty = TypeAnnotation::Array(Box::new(ty));
                remaining = &remaining[..remaining.len() - 2];
            }
            Ok(ty)
        }
        Rule::non_array_type => {
            let mut inner = pair.clone().into_inner();
            let inner_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type in non-array type annotation".to_string(),
                location: Some(pair_loc),
            })?;
            if pair.as_str().trim_start().starts_with("Vec<")
                && inner_pair.as_rule() == Rule::type_annotation
            {
                let inner_ty = parse_type_annotation(inner_pair)?;
                return Ok(TypeAnnotation::Array(Box::new(inner_ty)));
            }
            parse_type_annotation(inner_pair)
        }
        Rule::basic_type => parse_basic_type(pair.as_str()),
        Rule::tuple_type => {
            let mut members = Vec::new();
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::type_annotation {
                    members.push(parse_type_annotation(inner)?);
                }
            }
            Ok(TypeAnnotation::Tuple(members))
        }
        Rule::object_type => parse_object_type(pair),
        Rule::function_type => parse_function_type(pair),
        Rule::dyn_type => {
            let trait_names: Vec<String> = pair
                .into_inner()
                .filter(|p| p.as_rule() == Rule::ident)
                .map(|p| p.as_str().to_string())
                .collect();
            Ok(TypeAnnotation::Dyn(trait_names))
        }
        Rule::unit_type => Ok(TypeAnnotation::Basic("()".to_string())),
        Rule::generic_type => parse_generic_type(pair),
        Rule::type_param => {
            let param = parse_type_param(pair)?;
            Ok(param.type_annotation)
        }
        Rule::ident => Ok(TypeAnnotation::Reference(pair.as_str().to_string())),
        _ => Err(ShapeError::ParseError {
            message: format!("invalid type annotation: {:?}", pair.as_rule()),
            location: Some(pair_loc),
        }),
    }
}

/// Parse basic types (primitives and special types)
pub fn parse_basic_type(name: &str) -> Result<TypeAnnotation> {
    Ok(match name {
        "void" => TypeAnnotation::Void,
        "never" => TypeAnnotation::Never,
        "undefined" => TypeAnnotation::Undefined,
        other => TypeAnnotation::Basic(other.to_string()),
    })
}

/// Parse object type with fields
pub fn parse_object_type(pair: Pair<Rule>) -> Result<TypeAnnotation> {
    let mut fields = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::object_type_member_list {
            for member in inner.into_inner() {
                if member.as_rule() == Rule::object_type_member {
                    fields.push(parse_object_type_member(member)?);
                }
            }
        }
    }
    Ok(TypeAnnotation::Object(fields))
}

/// Parse a single object type member (field)
pub fn parse_object_type_member(pair: Pair<Rule>) -> Result<crate::ast::ObjectTypeField> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.clone().into_inner();
    let mut annotations = Vec::new();

    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected field name in object type member".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    let name_pair = if first.as_rule() == Rule::annotations {
        annotations = super::functions::parse_annotations(first)?;
        inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected field name after annotations in object type member".to_string(),
            location: Some(pair_loc.clone()),
        })?
    } else {
        first
    };
    let name = name_pair.as_str().to_string();

    let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: format!("expected type annotation for field '{}'", name),
        location: Some(pair_loc),
    })?;
    let type_annotation = parse_type_annotation(type_pair)?;

    let before_colon = pair.as_str().split(':').next().unwrap_or("");
    let optional = before_colon.contains('?');

    Ok(crate::ast::ObjectTypeField {
        name,
        optional,
        type_annotation,
        annotations,
    })
}

/// Parse function type signature
pub fn parse_function_type(pair: Pair<Rule>) -> Result<TypeAnnotation> {
    let mut params = Vec::new();
    let mut return_type = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::type_param_list => {
                params = parse_type_param_list(inner)?;
            }
            Rule::type_annotation => {
                return_type = Some(parse_type_annotation(inner)?);
            }
            _ => {}
        }
    }

    let returns = return_type.ok_or_else(|| ShapeError::ParseError {
        message: "Function type missing return type".to_string(),
        location: None,
    })?;

    Ok(TypeAnnotation::Function {
        params,
        returns: Box::new(returns),
    })
}

/// Parse list of type parameters
pub fn parse_type_param_list(pair: Pair<Rule>) -> Result<Vec<crate::ast::FunctionParam>> {
    let mut params = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::type_param {
            params.push(parse_type_param(inner)?);
        }
    }
    Ok(params)
}

/// Parse a single type parameter
pub fn parse_type_param(pair: Pair<Rule>) -> Result<crate::ast::FunctionParam> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.clone().into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected type parameter content".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    if first.as_rule() == Rule::ident {
        let name = first.as_str().to_string();
        let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: format!("type parameter '{}' missing type annotation", name),
            location: Some(pair_loc),
        })?;
        let type_annotation = parse_type_annotation(type_pair)?;
        let before_colon = pair.as_str().split(':').next().unwrap_or("");
        let optional = before_colon.contains('?');
        Ok(crate::ast::FunctionParam {
            name: Some(name),
            optional,
            type_annotation,
        })
    } else {
        let type_annotation = parse_type_annotation(first)?;
        Ok(crate::ast::FunctionParam {
            name: None,
            optional: false,
            type_annotation,
        })
    }
}

fn unwrap_documented_pair<'a>(
    pair: Pair<'a, Rule>,
    documented_rule: Rule,
    inner_rule: Rule,
    context: &'static str,
) -> Result<(Option<crate::ast::DocComment>, Pair<'a, Rule>)> {
    if pair.as_rule() == inner_rule {
        return Ok((None, pair));
    }

    if pair.as_rule() != documented_rule {
        return Err(ShapeError::ParseError {
            message: format!("expected {}", context),
            location: Some(pair_location(&pair)),
        });
    }

    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let mut doc_comment = None;
    let mut item = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: format!("expected {}", context),
        location: Some(pair_loc.clone()),
    })?;

    if item.as_rule() == Rule::doc_comment {
        doc_comment = Some(super::docs::parse_doc_comment(item));
        item = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: format!("expected {} after doc comment", context),
            location: Some(pair_loc.clone()),
        })?;
    }

    if item.as_rule() != inner_rule {
        return Err(ShapeError::ParseError {
            message: format!("expected {}", context),
            location: Some(pair_location(&item)),
        });
    }

    Ok((doc_comment, item))
}

pub fn parse_type_params(pair: Pair<Rule>) -> Result<Vec<crate::ast::TypeParam>> {
    let pair_loc = pair_location(&pair);
    let mut params = Vec::new();
    for param_pair in pair.into_inner() {
        if matches!(
            param_pair.as_rule(),
            Rule::documented_type_param_name | Rule::type_param_name
        ) {
            let (doc_comment, param_pair) = unwrap_documented_pair(
                param_pair,
                Rule::documented_type_param_name,
                Rule::type_param_name,
                "type parameter",
            )?;
            let param_span = pair_span(&param_pair);
            let mut param_inner = param_pair.into_inner();
            let name_pair = param_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected type parameter name".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let name = name_pair.as_str().to_string();
            let mut default_type = None;
            let mut trait_bounds = Vec::new();
            for remaining in param_inner {
                match remaining.as_rule() {
                    Rule::type_annotation => {
                        default_type = Some(parse_type_annotation(remaining)?);
                    }
                    Rule::trait_bound_list => {
                        for bound_ident in remaining.into_inner() {
                            if bound_ident.as_rule() == Rule::ident {
                                trait_bounds.push(bound_ident.as_str().to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            params.push(crate::ast::TypeParam {
                name,
                span: param_span,
                doc_comment,
                default_type,
                trait_bounds,
            });
        }
    }
    Ok(params)
}

/// Parse a declaration-only builtin type definition.
///
/// Grammar:
/// `builtin type Name<T>;`
pub fn parse_builtin_type_decl(pair: Pair<Rule>) -> Result<crate::ast::BuiltinTypeDecl> {
    let pair_loc = pair_location(&pair);
    let mut name = String::new();
    let mut name_span = crate::ast::Span::DUMMY;
    let mut type_params = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                if name.is_empty() {
                    name = inner.as_str().to_string();
                    name_span = super::pair_span(&inner);
                }
            }
            Rule::type_params => {
                type_params = Some(parse_type_params(inner)?);
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err(ShapeError::ParseError {
            message: "expected builtin type name".to_string(),
            location: Some(pair_loc),
        });
    }

    Ok(crate::ast::BuiltinTypeDecl {
        name,
        name_span,
        doc_comment: None,
        type_params,
    })
}

/// Parse generic type with arguments
pub fn parse_generic_type(pair: Pair<Rule>) -> Result<TypeAnnotation> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected generic type name".to_string(),
            location: Some(pair_loc.clone()),
        })?
        .as_str()
        .to_string();
    let mut args = Vec::new();
    for arg in inner {
        if arg.as_rule() == Rule::type_annotation {
            args.push(parse_type_annotation(arg)?);
        }
    }
    if name == "Matrix" {
        return Err(ShapeError::ParseError {
            message: "Matrix<T> has been removed; use Mat<T> instead".to_string(),
            location: Some(pair_loc),
        });
    }
    if (name == "Vec" || name == "Array") && args.len() == 1 {
        Ok(TypeAnnotation::Array(Box::new(args.remove(0))))
    } else {
        Ok(TypeAnnotation::Generic { name, args })
    }
}

/// Parse comptime field overrides for type aliases
///
/// Grammar: `"{" ~ comptime_field_override ~ ("," ~ comptime_field_override)* ~ ","? ~ "}"`
/// Where: `comptime_field_override = { ident ~ ":" ~ expression }`
pub fn parse_comptime_field_overrides(
    pair: Pair<Rule>,
) -> Result<HashMap<String, crate::ast::Expr>> {
    let mut overrides = HashMap::new();

    for override_pair in pair.into_inner() {
        if override_pair.as_rule() == Rule::comptime_field_override {
            let mut inner = override_pair.into_inner();

            let key_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "Missing parameter name in override".to_string(),
                location: None,
            })?;
            let key = key_pair.as_str().to_string();

            let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "Missing expression in parameter override".to_string(),
                location: None,
            })?;
            let expr = super::expressions::parse_expression(expr_pair)?;

            overrides.insert(key, expr);
        }
    }

    Ok(overrides)
}

/// Parse struct type definition
///
/// Grammar: `"type" ~ ident ~ type_params? ~ "{" ~ struct_field_list? ~ "}"`
pub fn parse_struct_type_def(pair: Pair<Rule>) -> Result<crate::ast::StructTypeDef> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let mut annotations = Vec::new();
    let mut type_params = None;
    let mut fields = Vec::new();

    // First child may be annotations or the struct name ident
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing struct type name".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    let name = if first.as_rule() == Rule::annotations {
        annotations = super::functions::parse_annotations(first)?;
        let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "Missing struct type name after annotations".to_string(),
            location: Some(pair_loc.clone()),
        })?;
        name_pair.as_str().to_string()
    } else {
        first.as_str().to_string()
    };

    for part in inner {
        match part.as_rule() {
            Rule::type_params => {
                type_params = Some(parse_type_params(part)?);
            }
            Rule::struct_field_list => {
                for field_pair in part.into_inner() {
                    if matches!(
                        field_pair.as_rule(),
                        Rule::documented_struct_field | Rule::struct_field
                    ) {
                        fields.push(parse_struct_field(field_pair)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(crate::ast::StructTypeDef {
        name,
        doc_comment: None,
        type_params,
        fields,
        methods: Vec::new(),
        annotations,
        native_layout: None,
    })
}

/// Parse native-layout type definition.
///
/// Grammar: `annotations? ~ "type" ~ extern_abi ~ ident ~ type_params? ~ "{" ~ struct_field_list? ~ "}"`
pub fn parse_native_struct_type_def(pair: Pair<Rule>) -> Result<crate::ast::StructTypeDef> {
    let pair_loc = pair_location(&pair);
    let mut annotations = Vec::new();
    let mut abi: Option<String> = None;
    let mut name: Option<String> = None;
    let mut type_params = None;
    let mut fields = Vec::new();

    for part in pair.into_inner() {
        match part.as_rule() {
            Rule::annotations => {
                annotations = super::functions::parse_annotations(part)?;
            }
            Rule::extern_abi => {
                abi = Some(super::functions::parse_extern_abi(part)?);
            }
            Rule::ident => {
                if name.is_none() {
                    name = Some(part.as_str().to_string());
                }
            }
            Rule::type_params => {
                type_params = Some(parse_type_params(part)?);
            }
            Rule::struct_field_list => {
                for field_pair in part.into_inner() {
                    if matches!(
                        field_pair.as_rule(),
                        Rule::documented_struct_field | Rule::struct_field
                    ) {
                        fields.push(parse_struct_field(field_pair)?);
                    }
                }
            }
            _ => {}
        }
    }

    let abi = abi.ok_or_else(|| ShapeError::ParseError {
        message: "native layout type declaration requires an ABI name".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    if abi.trim() != "C" {
        return Err(ShapeError::ParseError {
            message: format!(
                "unsupported native ABI '{}': only C is currently supported for type layouts",
                abi
            ),
            location: Some(pair_loc.clone()),
        });
    }

    let name = name.ok_or_else(|| ShapeError::ParseError {
        message: "Missing native layout type name".to_string(),
        location: Some(pair_loc),
    })?;

    Ok(crate::ast::StructTypeDef {
        name,
        doc_comment: None,
        type_params,
        fields,
        methods: Vec::new(),
        annotations,
        native_layout: Some(crate::ast::NativeLayoutBinding { abi }),
    })
}

/// Parse a single struct field
///
/// Grammar: `annotations? ~ comptime_keyword? ~ ident ~ ":" ~ type_annotation ~ ("=" ~ expression)?`
fn parse_struct_field(pair: Pair<Rule>) -> Result<crate::ast::StructField> {
    let (doc_comment, pair) = unwrap_documented_pair(
        pair,
        Rule::documented_struct_field,
        Rule::struct_field,
        "struct field",
    )?;
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    let mut annotations = vec![];
    let mut is_comptime = false;

    // Peek at the first child — may be annotations, comptime_keyword, or ident
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing struct field name".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    let mut current = first;

    if current.as_rule() == Rule::annotations {
        annotations = super::functions::parse_annotations(current)?;
        current = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "Missing struct field name after annotations".to_string(),
            location: Some(pair_loc.clone()),
        })?;
    }

    if current.as_rule() == Rule::comptime_keyword {
        is_comptime = true;
        current = inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "Missing struct field name after 'comptime'".to_string(),
            location: Some(pair_loc.clone()),
        })?;
    }

    let name = current.as_str().to_string();

    let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: format!("Missing type annotation for struct field '{}'", name),
        location: Some(pair_loc),
    })?;
    let type_annotation = parse_type_annotation(type_pair)?;

    // Parse optional default value expression (for comptime fields)
    let default_value = if let Some(expr_pair) = inner.next() {
        Some(super::expressions::parse_expression(expr_pair)?)
    } else {
        None
    };

    Ok(crate::ast::StructField {
        annotations,
        is_comptime,
        name,
        span,
        doc_comment,
        type_annotation,
        default_value,
    })
}

/// Parse type alias definition
///
/// Grammar: `"type" ~ ident ~ type_params? ~ "=" ~ type_annotation ~ comptime_field_overrides? ~ ";"?`
pub fn parse_type_alias_def(pair: Pair<Rule>) -> Result<crate::ast::TypeAliasDef> {
    let mut inner = pair.into_inner();

    // Parse alias name
    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing type alias name".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();

    // Check for type parameters (generics), type annotation, and comptime field overrides
    let mut type_params = None;
    let mut type_annotation = None;
    let mut meta_param_overrides = None;

    for part in inner {
        match part.as_rule() {
            Rule::type_params => {
                // Parse generic type parameters like <T, U>
                type_params = Some(parse_type_params(part)?);
            }
            Rule::type_annotation => {
                type_annotation = Some(parse_type_annotation(part)?);
            }
            Rule::comptime_field_overrides => {
                meta_param_overrides = Some(parse_comptime_field_overrides(part)?);
            }
            _ => {}
        }
    }

    let type_annotation = type_annotation.ok_or_else(|| ShapeError::ParseError {
        message: "Type alias missing type annotation".to_string(),
        location: None,
    })?;

    Ok(crate::ast::TypeAliasDef {
        name,
        doc_comment: None,
        type_params,
        type_annotation,
        meta_param_overrides,
    })
}

/// Parse enum definition
///
/// Grammar: `"enum" ~ ident ~ type_params? ~ "{" ~ enum_member_list? ~ "}"`
pub fn parse_enum_def(pair: Pair<Rule>) -> Result<crate::ast::EnumDef> {
    let pair_loc = pair_location(&pair);
    let inner = pair.into_inner();

    let mut annotations = Vec::new();
    let mut name = String::new();
    let mut type_params = None;
    let mut members = Vec::new();

    for part in inner {
        match part.as_rule() {
            Rule::annotations => {
                annotations = crate::parser::functions::parse_annotations(part)?;
            }
            Rule::ident => {
                if name.is_empty() {
                    name = part.as_str().to_string();
                }
            }
            Rule::type_params => {
                type_params = Some(parse_type_params(part)?);
            }
            Rule::enum_member_list => {
                for member_pair in part.into_inner() {
                    if matches!(
                        member_pair.as_rule(),
                        Rule::documented_enum_member | Rule::enum_member
                    ) {
                        members.push(parse_enum_member(member_pair)?);
                    }
                }
            }
            Rule::documented_enum_member | Rule::enum_member => {
                members.push(parse_enum_member(part)?);
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Missing enum name".to_string(),
            location: Some(pair_loc),
        });
    }

    Ok(crate::ast::EnumDef {
        name,
        doc_comment: None,
        type_params,
        members,
        annotations,
    })
}

fn parse_enum_member(pair: Pair<Rule>) -> Result<crate::ast::EnumMember> {
    let (doc_comment, pair) = unwrap_documented_pair(
        pair,
        Rule::documented_enum_member,
        Rule::enum_member,
        "enum member",
    )?;
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected enum member content".to_string(),
            location: Some(pair_loc.clone()),
        })?;

    match inner.as_rule() {
        Rule::enum_variant_unit => {
            let mut unit_inner = inner.into_inner();
            let name_pair = unit_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected enum variant name".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let name = name_pair.as_str().to_string();
            let value = if let Some(val_pair) = unit_inner.next() {
                match val_pair.as_rule() {
                    Rule::string => Some(crate::ast::EnumValue::String(parse_string_literal(
                        val_pair.as_str(),
                    )?)),
                    Rule::number => {
                        let n: f64 =
                            val_pair
                                .as_str()
                                .parse()
                                .map_err(|e| ShapeError::ParseError {
                                    message: format!("Invalid enum value number: {}", e),
                                    location: Some(pair_loc.clone()),
                                })?;
                        Some(crate::ast::EnumValue::Number(n))
                    }
                    _ => {
                        return Err(ShapeError::ParseError {
                            message: "invalid enum unit value".to_string(),
                            location: Some(pair_loc),
                        });
                    }
                }
            } else {
                None
            };

            Ok(crate::ast::EnumMember {
                name,
                kind: crate::ast::EnumMemberKind::Unit { value },
                span,
                doc_comment,
            })
        }
        Rule::enum_variant_tuple => {
            let mut tuple_inner = inner.into_inner();
            let name_pair = tuple_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected enum variant name".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let name = name_pair.as_str().to_string();
            let mut fields = Vec::new();
            for type_pair in tuple_inner {
                if type_pair.as_rule() == Rule::type_annotation {
                    fields.push(parse_type_annotation(type_pair)?);
                }
            }
            Ok(crate::ast::EnumMember {
                name,
                kind: crate::ast::EnumMemberKind::Tuple(fields),
                span,
                doc_comment,
            })
        }
        Rule::enum_variant_struct => {
            let mut struct_inner = inner.into_inner();
            let name_pair = struct_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected enum variant name".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            let name = name_pair.as_str().to_string();
            let mut fields = Vec::new();
            for part in struct_inner {
                if part.as_rule() == Rule::object_type_member_list {
                    for field_pair in part.into_inner() {
                        if field_pair.as_rule() == Rule::object_type_member {
                            fields.push(parse_object_type_member(field_pair)?);
                        }
                    }
                }
            }
            Ok(crate::ast::EnumMember {
                name,
                kind: crate::ast::EnumMemberKind::Struct(fields),
                span,
                doc_comment,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected enum member rule: {:?}", inner.as_rule()),
            location: Some(pair_loc),
        }),
    }
}

/// Parse interface definition
///
/// Grammar: `"interface" ~ ident ~ type_params? ~ "{" ~ interface_body ~ "}"`
pub fn parse_interface_def(pair: Pair<Rule>) -> Result<crate::ast::InterfaceDef> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing interface name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let name = name_pair.as_str().to_string();

    let mut type_params = None;
    let mut members = Vec::new();

    for part in inner {
        match part.as_rule() {
            Rule::type_params => {
                type_params = Some(parse_type_params(part)?);
            }
            Rule::interface_body => {
                members = parse_interface_body(part)?;
            }
            _ => {}
        }
    }

    Ok(crate::ast::InterfaceDef {
        name,
        doc_comment: None,
        type_params,
        members,
    })
}

/// Parse trait definition
///
/// Grammar: `annotations? ~ "trait" ~ ident ~ type_params? ~ (":" ~ type_annotation ~ ("+" ~ type_annotation)*)? ~ "{" ~ trait_body ~ "}"`
///
/// Traits reuse the same body syntax as interfaces (method/property signatures).
/// Supertrait bounds use `:` syntax: `trait Foo: Bar + Baz { ... }`
pub fn parse_trait_def(pair: Pair<Rule>) -> Result<crate::ast::TraitDef> {
    let pair_loc = pair_location(&pair);
    let inner = pair.into_inner();

    let mut annotations = Vec::new();
    let mut type_params = None;
    let mut super_traits = Vec::new();
    let mut members = Vec::new();
    let mut name = String::new();

    // First child may be annotations or ident
    for part in inner {
        match part.as_rule() {
            Rule::annotations => {
                annotations = crate::parser::functions::parse_annotations(part)?;
            }
            Rule::ident => {
                if name.is_empty() {
                    name = part.as_str().to_string();
                }
            }
            Rule::type_params => {
                type_params = Some(parse_type_params(part)?);
            }
            Rule::supertrait_list => {
                for inner_part in part.into_inner() {
                    if inner_part.as_rule() == Rule::optional_type {
                        super_traits.push(parse_type_annotation(inner_part)?);
                    }
                }
            }
            Rule::trait_body => {
                members = parse_trait_body(part)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err(ShapeError::ParseError {
            message: "Missing trait name".to_string(),
            location: Some(pair_loc),
        });
    }

    Ok(crate::ast::TraitDef {
        name,
        doc_comment: None,
        type_params,
        super_traits,
        members,
        annotations,
    })
}

fn parse_trait_body(pair: Pair<Rule>) -> Result<Vec<crate::ast::TraitMember>> {
    let mut members = Vec::new();

    for trait_member in pair.into_inner() {
        if trait_member.as_rule() != Rule::trait_member {
            continue;
        }

        let mut member_inner = trait_member.into_inner();
        let mut doc_comment = None;
        let mut inner = member_inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected trait member".to_string(),
            location: None,
        })?;

        if inner.as_rule() == Rule::doc_comment {
            doc_comment = Some(super::docs::parse_doc_comment(inner));
            inner = member_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected trait member after doc comment".to_string(),
                location: None,
            })?;
        }

        if inner.as_rule() == Rule::trait_member_core {
            inner = inner
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected trait member".to_string(),
                    location: None,
                })?;
        }

        match inner.as_rule() {
            Rule::associated_type_decl => {
                let (name, bounds, span) = parse_associated_type_decl(inner)?;
                members.push(crate::ast::TraitMember::AssociatedType {
                    name,
                    bounds,
                    span,
                    doc_comment,
                });
            }
            Rule::method_def => {
                let mut method = parse_method_def_shared(inner)?;
                method.doc_comment = doc_comment;
                members.push(crate::ast::TraitMember::Default(method));
            }
            Rule::interface_member | Rule::documented_interface_member => {
                let mut im = parse_interface_member(inner)?;
                if let Some(doc_comment) = doc_comment {
                    attach_interface_member_doc_comment(&mut im, doc_comment);
                }
                members.push(crate::ast::TraitMember::Required(im));
            }
            _ => {}
        }
    }

    Ok(members)
}

/// Parse `type Item;` or `type Item: Bound1 + Bound2;`
fn parse_associated_type_decl(pair: Pair<Rule>) -> Result<(String, Vec<TypeAnnotation>, Span)> {
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected associated type name".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();

    let mut bounds = Vec::new();
    for remaining in inner {
        if remaining.as_rule() == Rule::trait_bound_list {
            for bound_ident in remaining.into_inner() {
                if bound_ident.as_rule() == Rule::ident {
                    bounds.push(TypeAnnotation::Basic(bound_ident.as_str().to_string()));
                }
            }
        }
    }

    Ok((name, bounds, span))
}

/// Shared parser for method_def rule, used by extend, impl, and trait bodies
pub(crate) fn parse_method_def_shared(pair: Pair<Rule>) -> Result<crate::ast::types::MethodDef> {
    use crate::ast::types::MethodDef;

    // Detect optional "async" prefix from the raw text
    let is_async = pair.as_str().trim_start().starts_with("async");
    let span = pair_span(&pair);

    let mut md_inner = pair.into_inner();

    let name_pair = md_inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "Missing method name".to_string(),
        location: None,
    })?;
    let name = name_pair.as_str().to_string();

    let mut params = Vec::new();
    let mut when_clause = None;
    let mut return_type = None;
    let mut body = Vec::new();

    for part in md_inner {
        match part.as_rule() {
            Rule::function_params => {
                for param_pair in part.into_inner() {
                    if param_pair.as_rule() == Rule::function_param {
                        params.push(super::functions::parse_function_param(param_pair)?);
                    }
                }
            }
            Rule::when_clause => {
                if let Some(expr_pair) = part.into_inner().next() {
                    when_clause = Some(Box::new(crate::parser::expressions::parse_expression(
                        expr_pair,
                    )?));
                }
            }
            Rule::return_type => {
                if let Some(type_pair) = part.into_inner().next() {
                    return_type = Some(parse_type_annotation(type_pair)?);
                }
            }
            Rule::function_body => {
                body = super::statements::parse_statements(part.into_inner())?;
            }
            _ => {}
        }
    }

    Ok(MethodDef {
        name,
        span,
        declaring_module_path: None,
        doc_comment: None,
        annotations: Vec::new(),
        params,
        when_clause,
        return_type,
        body,
        is_async,
    })
}

pub(crate) fn parse_documented_method_def_shared(
    pair: Pair<Rule>,
) -> Result<crate::ast::types::MethodDef> {
    if pair.as_rule() == Rule::method_def {
        return parse_method_def_shared(pair);
    }
    assert_eq!(pair.as_rule(), Rule::documented_method_def);
    let inner = pair.into_inner();
    let mut doc_comment = None;
    let mut annotations = Vec::new();
    let mut method_pair = None;

    for child in inner {
        match child.as_rule() {
            Rule::doc_comment => {
                doc_comment = Some(super::docs::parse_doc_comment(child));
            }
            Rule::annotations => {
                annotations = super::functions::parse_annotations(child)?;
            }
            Rule::method_def => {
                method_pair = Some(child);
            }
            _ => {}
        }
    }

    let method_pair = method_pair.ok_or_else(|| ShapeError::ParseError {
        message: "expected method definition".to_string(),
        location: None,
    })?;
    let mut method = parse_method_def_shared(method_pair)?;
    method.doc_comment = doc_comment;
    method.annotations = annotations;
    Ok(method)
}

fn parse_interface_body(pair: Pair<Rule>) -> Result<Vec<crate::ast::InterfaceMember>> {
    let mut members = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::interface_member_list {
            for member in inner.into_inner() {
                if matches!(
                    member.as_rule(),
                    Rule::documented_interface_member | Rule::interface_member
                ) {
                    members.push(parse_interface_member(member)?);
                }
            }
        }
    }
    Ok(members)
}

fn parse_interface_member(pair: Pair<Rule>) -> Result<crate::ast::InterfaceMember> {
    let (doc_comment, pair) = unwrap_documented_pair(
        pair,
        Rule::documented_interface_member,
        Rule::interface_member,
        "interface member",
    )?;
    let pair_loc = pair_location(&pair);
    let raw = pair.as_str();
    let trimmed = raw.trim_start();
    let span = pair_span(&pair);

    if trimmed.starts_with('[') {
        return parse_interface_index_signature(pair, trimmed, doc_comment);
    }

    let mut inner = pair.into_inner();
    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected interface member name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let name = name_pair.as_str().to_string();

    let (optional, is_method) = parse_interface_member_kind(trimmed, &name);

    let mut params = Vec::new();
    let mut type_annotation = None;
    for part in inner {
        match part.as_rule() {
            Rule::type_param_list => {
                params = parse_type_param_list(part)?;
            }
            Rule::type_annotation => {
                type_annotation = Some(parse_type_annotation(part)?);
            }
            _ => {}
        }
    }

    let type_annotation = type_annotation.ok_or_else(|| ShapeError::ParseError {
        message: format!("interface member '{}' missing type annotation", name),
        location: Some(pair_loc),
    })?;

    if is_method {
        Ok(crate::ast::InterfaceMember::Method {
            name,
            optional,
            params,
            return_type: type_annotation,
            is_async: false,
            span,
            doc_comment,
        })
    } else {
        Ok(crate::ast::InterfaceMember::Property {
            name,
            optional,
            type_annotation,
            span,
            doc_comment,
        })
    }
}

fn parse_interface_member_kind(raw: &str, name: &str) -> (bool, bool) {
    let trimmed = raw.trim_start();
    let Some(mut rest) = trimmed.strip_prefix(name) else {
        return (false, false);
    };
    rest = rest.trim_start();
    let mut optional = false;
    if rest.starts_with('?') {
        optional = true;
        rest = rest[1..].trim_start();
    }
    let is_method = rest.starts_with('(');
    (optional, is_method)
}

fn parse_interface_index_signature(
    pair: Pair<Rule>,
    raw: &str,
    doc_comment: Option<crate::ast::DocComment>,
) -> Result<crate::ast::InterfaceMember> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let mut inner = pair.into_inner();
    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected index signature parameter name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let param_name = name_pair.as_str().to_string();

    let mut return_type = None;
    for part in inner {
        if part.as_rule() == Rule::type_annotation {
            return_type = Some(parse_type_annotation(part)?);
        }
    }

    let return_type = return_type.ok_or_else(|| ShapeError::ParseError {
        message: "index signature missing return type".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    let param_type =
        parse_index_signature_param_type(raw).ok_or_else(|| ShapeError::ParseError {
            message: "index signature missing parameter type".to_string(),
            location: Some(pair_loc),
        })?;

    Ok(crate::ast::InterfaceMember::IndexSignature {
        param_name,
        param_type,
        return_type,
        span,
        doc_comment,
    })
}

fn attach_interface_member_doc_comment(
    member: &mut crate::ast::InterfaceMember,
    doc_comment: crate::ast::DocComment,
) {
    match member {
        crate::ast::InterfaceMember::Property {
            doc_comment: slot, ..
        }
        | crate::ast::InterfaceMember::Method {
            doc_comment: slot, ..
        }
        | crate::ast::InterfaceMember::IndexSignature {
            doc_comment: slot, ..
        } => *slot = Some(doc_comment),
    }
}

fn parse_index_signature_param_type(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let open = trimmed.find('[')?;
    let close = trimmed[open + 1..].find(']')? + open + 1;
    let inside = trimmed[open + 1..close].trim();
    let mut parts = inside.splitn(2, ':');
    parts.next()?;
    let param_type = parts.next()?.trim();
    if param_type == "string" || param_type == "number" {
        Some(param_type.to_string())
    } else {
        None
    }
}
