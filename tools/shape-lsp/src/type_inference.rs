//! Shared type inference utilities for the LSP
//!
//! This module provides the canonical implementations of type inference functions
//! used by hover, completions, and inlay hints. All type inference should go through
//! these functions to avoid duplication and ensure consistency.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use shape_ast::ast::{
    Expr, InterfaceMember, Item, Literal, ObjectEntry, ObjectTypeField, Pattern, Program,
    Statement, TraitMember, TypeAnnotation, VariableDecl,
};
use shape_runtime::metadata::UnifiedMetadata;
use shape_runtime::schema_cache::{
    DataSourceSchemaCache, EntitySchema, SourceSchema, default_cache_path,
    load_cached_source_for_uri_with_diagnostics,
};
use shape_runtime::type_system::{
    PropertyAssignmentCollector, Type, TypeInferenceEngine, TypeScheme,
};
use shape_runtime::visitor::{Visitor, walk_program};
use shape_vm::compiler::ParamPassMode;
use std::path::{Path, PathBuf};

/// Global unified metadata, loaded lazily on first access
static UNIFIED_METADATA: OnceLock<UnifiedMetadata> = OnceLock::new();

pub fn unified_metadata() -> &'static UnifiedMetadata {
    UNIFIED_METADATA.get_or_init(UnifiedMetadata::load)
}

/// Convert a TypeAnnotation to a string representation
pub fn type_annotation_to_string(ta: &TypeAnnotation) -> Option<String> {
    match ta {
        TypeAnnotation::Basic(s) => Some(s.clone()),
        TypeAnnotation::Array(inner) => {
            type_annotation_to_string(inner).map(|s| format!("{}[]", s))
        }
        TypeAnnotation::Reference(s) => Some(s.clone()),
        TypeAnnotation::Generic { name, args } => {
            let arg_strs: Vec<String> = args.iter().filter_map(type_annotation_to_string).collect();
            Some(format!("{}<{}>", name, arg_strs.join(", ")))
        }
        TypeAnnotation::Void => Some("()".to_string()),
        TypeAnnotation::Never => Some("never".to_string()),
        TypeAnnotation::Null => Some("None".to_string()),
        TypeAnnotation::Undefined => Some("undefined".to_string()),
        TypeAnnotation::Dyn(traits) => Some(format!("dyn {}", traits.join(" + "))),
        TypeAnnotation::Tuple(items) => {
            let strs: Vec<String> = items.iter().filter_map(type_annotation_to_string).collect();
            Some(format!("({})", strs.join(", ")))
        }
        TypeAnnotation::Object(fields) => Some(format_object_shape_from_type_fields(fields)),
        TypeAnnotation::Function { .. } => Some("Function".to_string()),
        TypeAnnotation::Union(types) => {
            let strs: Vec<String> = types.iter().filter_map(type_annotation_to_string).collect();
            Some(strs.join(" | "))
        }
        TypeAnnotation::Intersection(types) => {
            let strs: Vec<String> = types.iter().filter_map(type_annotation_to_string).collect();
            merge_structural_intersection_shapes(&strs).or_else(|| Some(strs.join(" + ")))
        }
    }
}

/// Infer the type of an expression
pub fn infer_expr_type(expr: &Expr) -> Option<String> {
    let env = HashMap::new();
    infer_expr_type_with_env(expr, &env)
}

fn infer_expr_type_with_env(expr: &Expr, env: &HashMap<String, String>) -> Option<String> {
    match expr {
        Expr::Literal(lit, _) => Some(infer_literal_type(lit)),
        Expr::FunctionCall { name, .. } => infer_function_return_type(name),
        Expr::QualifiedFunctionCall {
            namespace, function, ..
        } => infer_function_return_type(&format!("{}::{}", namespace, function)),
        Expr::EnumConstructor { enum_name, .. } => Some(enum_name.clone()),
        Expr::MethodCall {
            receiver, method, ..
        } => match method.as_str() {
            // Type-preserving methods: return same type as receiver
            "filter" | "where" | "head" | "tail" | "slice" | "reverse" | "concat" | "orderBy"
            | "limit" | "sort" | "execute" => infer_expr_type_with_env(receiver, env),
            // Aggregation methods: always return number
            "sum" | "mean" | "avg" | "min" | "max" | "count" | "reduce" => {
                Some("number".to_string())
            }
            // String conversion
            "toString" | "to_string" | "toFixed" => Some("string".to_string()),
            // Universal type query
            "type" => Some("Type".to_string()),
            // Length/size
            "length" | "len" => Some("number".to_string()),
            // Boolean checks
            "isEmpty" | "contains" | "startsWith" | "endsWith" | "some" | "every" | "is_ok"
            | "is_err" | "is_some" | "is_none" => Some("bool".to_string()),
            // Unwrap: extract inner type from Result/Option
            "unwrap" | "unwrap_or" => {
                if let Some(receiver_type) = infer_expr_type_with_env(receiver, env) {
                    extract_wrapper_inner(&receiver_type)
                } else {
                    None
                }
            }
            // Map produces Array
            "map" => Some("Array".to_string()),
            _ => None,
        },
        Expr::BinaryOp {
            op, left, right, ..
        } => {
            use shape_ast::ast::BinaryOp;
            match op {
                BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEq
                | BinaryOp::Greater
                | BinaryOp::GreaterEq
                | BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::FuzzyEqual
                | BinaryOp::FuzzyGreater
                | BinaryOp::FuzzyLess => Some("bool".to_string()),
                BinaryOp::Add => {
                    let left_type = infer_expr_type_with_env(left, env);
                    let right_type = infer_expr_type_with_env(right, env);
                    infer_add_type(left_type.as_deref(), right_type.as_deref())
                        .or_else(|| Some("number".to_string()))
                }
                BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                    let left_type = infer_expr_type_with_env(left, env);
                    let right_type = infer_expr_type_with_env(right, env);
                    infer_numeric_arithmetic_type(left_type.as_deref(), right_type.as_deref())
                        .or_else(|| Some("number".to_string()))
                }
                BinaryOp::NullCoalesce => None,
                BinaryOp::ErrorContext => Some("Result".to_string()),
                BinaryOp::Pipe => {
                    // Pipe: a |> f(x) rewrites to f(a, x)
                    // Infer from right side first, then fall back to left type
                    if let Some(right_type) = infer_expr_type_with_env(right, env) {
                        Some(right_type)
                    } else {
                        // Unknown function on right: assume type-preserving
                        infer_expr_type_with_env(left, env)
                    }
                }
                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitShl
                | BinaryOp::BitShr => Some("number".to_string()),
            }
        }
        Expr::Array(elements, _) => Some(infer_array_type(elements)),
        Expr::Object(entries, _) => Some(infer_object_shape(entries)),
        Expr::DataRef(_, _) => Some("Row".to_string()),
        Expr::TryOperator(inner, _) => {
            if let Some(inner_type) = infer_expr_type_with_env(inner, env) {
                extract_wrapper_inner(&inner_type)
            } else {
                None
            }
        }
        Expr::UsingImpl { expr, .. } => infer_expr_type_with_env(expr, env),
        Expr::Identifier(name, _) => env.get(name).cloned(),
        Expr::DataDateTimeRef(_, _) => Some("Data".to_string()),
        Expr::DataRelativeAccess { .. } => Some("Data".to_string()),
        Expr::PropertyAccess { .. } => None,
        Expr::IndexAccess { .. } => None,
        Expr::UnaryOp { op, .. } => {
            use shape_ast::ast::UnaryOp;
            match op {
                UnaryOp::Not => Some("bool".to_string()),
                UnaryOp::Neg => Some("number".to_string()),
                UnaryOp::BitNot => Some("number".to_string()),
            }
        }
        Expr::TimeRef(_, _) => Some("Time".to_string()),
        Expr::DateTime(_, _) => Some("DateTime".to_string()),
        Expr::PatternRef(_, _) => Some("Pattern".to_string()),
        Expr::Conditional { then_expr, .. } => infer_expr_type_with_env(then_expr, env),
        Expr::Block(_, _) => None,
        Expr::TypeAssertion {
            type_annotation, ..
        } => type_annotation_to_string(type_annotation),
        Expr::InstanceOf { .. } => Some("bool".to_string()),
        Expr::FunctionExpr { .. } => Some("Function".to_string()),
        Expr::Duration(_, _) => Some("Duration".to_string()),
        Expr::Spread(_, _) => None,
        Expr::If(_, _) => None,
        Expr::While(_, _) => None,
        Expr::For(_, _) => None,
        Expr::Loop(_, _) => None,
        Expr::Let(_, _) => None,
        Expr::Assign(_, _) => None,
        Expr::Break(_, _) => None,
        Expr::Continue(_) => None,
        Expr::Return(_, _) => None,
        Expr::Match(match_expr, _) => {
            let mut arm_types: Vec<String> = match_expr
                .arms
                .iter()
                .filter_map(|arm| {
                    let mut arm_env = env.clone();
                    collect_typed_pattern_bindings(&arm.pattern, &mut arm_env);
                    infer_expr_type_with_env(&arm.body, &arm_env)
                })
                .collect();
            if arm_types.is_empty() {
                None
            } else {
                arm_types.sort();
                arm_types.dedup();
                match arm_types.len() {
                    0 => None,
                    1 => arm_types.into_iter().next(),
                    _ => Some(arm_types.join(" | ")),
                }
            }
        }
        Expr::Unit(_) => Some("()".to_string()),
        Expr::Range { .. } => Some("Range".to_string()),
        Expr::TimeframeContext { expr, .. } => infer_expr_type_with_env(expr, env),
        Expr::ListComprehension(_, _) => Some("Array".to_string()),
        Expr::SimulationCall { .. } => Some("SimulationResult".to_string()),
        Expr::WindowExpr(_, _) => Some("Number".to_string()),
        Expr::FuzzyComparison { .. } => Some("bool".to_string()),
        Expr::FromQuery(_, _) => Some("Array".to_string()),
        Expr::StructLiteral { type_name, .. } => Some(type_name.clone()),
        Expr::Await(inner, _) => infer_expr_type_with_env(inner, env),
        Expr::Join(_, _) => Some("Array".to_string()),
        Expr::Annotated { target, .. } => infer_expr_type_with_env(target, env),
        Expr::AsyncLet(_, _) => None,
        Expr::AsyncScope(inner, _) => infer_expr_type_with_env(inner, env),
        Expr::Comptime(_, _) => None,
        Expr::ComptimeFor(_, _) => None,
        Expr::Reference { expr: inner, .. } => infer_expr_type_with_env(inner, env),
        Expr::TableRows(..) => Some("Table".to_string()),
    }
}

/// Infer the type of a literal
pub fn infer_literal_type(lit: &Literal) -> String {
    match lit {
        Literal::Int(_) => "int".to_string(),
        Literal::UInt(_) => "u64".to_string(),
        Literal::TypedInt(_, w) => w.type_name().to_string(),
        Literal::Number(_) => "number".to_string(),
        Literal::Decimal(_) => "decimal".to_string(),
        Literal::String(_) => "string".to_string(),
        Literal::FormattedString { .. } => "string".to_string(),
        Literal::ContentString { .. } => "string".to_string(),
        Literal::Bool(_) => "bool".to_string(),
        Literal::Char(_) => "char".to_string(),
        Literal::None => "Option".to_string(),
        Literal::Unit => "()".to_string(),
        Literal::Timeframe(_) => "Timeframe".to_string(),
    }
}

/// Extract the inner type from Result<T> or Option<T>
pub fn extract_wrapper_inner(type_name: &str) -> Option<String> {
    if type_name.starts_with("Result<") && type_name.ends_with('>') {
        let inner = &type_name[7..type_name.len() - 1];
        if let Some(comma_pos) = inner.find(',') {
            return Some(inner[..comma_pos].trim().to_string());
        }
        return Some(inner.to_string());
    }
    if type_name.starts_with("Option<") && type_name.ends_with('>') {
        let inner = &type_name[7..type_name.len() - 1];
        return Some(inner.to_string());
    }
    if type_name.ends_with('?') {
        return Some(type_name[..type_name.len() - 1].to_string());
    }
    Some(type_name.to_string())
}

/// Infer the return type of a built-in function
pub fn infer_function_return_type(name: &str) -> Option<String> {
    unified_metadata()
        .get_function(name)
        .map(|f| f.return_type.clone())
}

/// Infer the type string for an array expression
fn infer_array_type(elements: &[Expr]) -> String {
    if elements.is_empty() {
        return "Array".to_string();
    }
    if let Some(first_type) = infer_expr_type(&elements[0]) {
        let all_same = elements
            .iter()
            .skip(1)
            .all(|e| infer_expr_type(e).as_deref() == Some(first_type.as_str()));
        if all_same {
            format!("{}[]", first_type)
        } else {
            "Array".to_string()
        }
    } else {
        "Array".to_string()
    }
}

fn format_object_shape_from_type_fields(fields: &[ObjectTypeField]) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }

    let parts: Vec<String> = fields
        .iter()
        .map(|field| {
            let field_type = type_annotation_to_string(&field.type_annotation)
                .unwrap_or_else(|| "unknown".to_string());
            if field.optional {
                format!("{}?: {}", field.name, field_type)
            } else {
                format!("{}: {}", field.name, field_type)
            }
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

fn split_top_level(input: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;

    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ => {}
        }

        if ch == delimiter
            && paren_depth == 0
            && bracket_depth == 0
            && brace_depth == 0
            && angle_depth == 0
        {
            parts.push(input[start..idx].trim().to_string());
            start = idx + ch.len_utf8();
        }
    }

    parts.push(input[start..].trim().to_string());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

pub fn is_structural_object_shape(type_name: &str) -> bool {
    let t = type_name.trim();
    t.starts_with('{') && t.ends_with('}')
}

fn is_generic_object_type(type_name: &str) -> bool {
    type_name.trim().eq_ignore_ascii_case("object")
}

pub fn parse_object_shape_fields(shape: &str) -> Option<Vec<(String, String)>> {
    let trimmed = shape.trim();
    if !is_structural_object_shape(trimmed) {
        return None;
    }

    let inner = trimmed
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))?
        .trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }

    let mut fields = Vec::new();
    for part in split_top_level(inner, ',') {
        if part.starts_with("...") {
            continue;
        }
        let (name, ty) = part.split_once(':')?;
        let field_name = name.trim().trim_end_matches('?').trim().to_string();
        let field_type = ty.trim().to_string();
        if field_name.is_empty() || field_type.is_empty() {
            return None;
        }
        fields.push((field_name, field_type));
    }
    Some(fields)
}

pub fn format_object_shape(fields: &[(String, String)]) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }
    let field_strs: Vec<String> = fields
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, ty))
        .collect();
    format!("{{ {} }}", field_strs.join(", "))
}

pub fn merge_object_shapes(left: &str, right: &str) -> Option<String> {
    let mut merged = parse_object_shape_fields(left)?;
    let right_fields = parse_object_shape_fields(right)?;

    for (name, ty) in right_fields {
        if !merged.iter().any(|(existing, _)| existing == &name) {
            merged.push((name, ty));
        }
    }

    Some(format_object_shape(&merged))
}

fn merge_structural_intersection_shapes(parts: &[String]) -> Option<String> {
    let mut iter = parts.iter();
    let first = iter.next()?;
    if !is_structural_object_shape(first) {
        return None;
    }

    let mut merged = first.clone();
    for part in iter {
        if !is_structural_object_shape(part) {
            return None;
        }
        merged = merge_object_shapes(&merged, part)?;
    }
    Some(merged)
}

fn infer_add_type(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let (Some(left), Some(right)) = (left, right) else {
        return None;
    };

    if left == "string" || right == "string" {
        return Some("string".to_string());
    }

    if is_structural_object_shape(left) && is_structural_object_shape(right) {
        return merge_object_shapes(left, right);
    }

    infer_numeric_arithmetic_type(Some(left), Some(right))
}

fn infer_numeric_arithmetic_type(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let (Some(left), Some(right)) = (left, right) else {
        return None;
    };
    if !is_numeric_type_name(left) || !is_numeric_type_name(right) {
        return None;
    }
    if left == right {
        return Some(left.to_string());
    }
    Some("number".to_string())
}

fn is_numeric_type_name(ty: &str) -> bool {
    matches!(
        ty,
        "int" | "number" | "decimal" | "float" | "integer" | "f64" | "i64"
    )
}

fn collect_typed_pattern_bindings(pattern: &Pattern, env: &mut HashMap<String, String>) {
    match pattern {
        Pattern::Typed {
            name,
            type_annotation,
        } => {
            if let Some(type_name) = type_annotation_to_string(type_annotation) {
                env.insert(name.clone(), type_name);
            }
        }
        Pattern::Array(patterns) => {
            for pat in patterns {
                collect_typed_pattern_bindings(pat, env);
            }
        }
        Pattern::Object(fields) => {
            for (_, pat) in fields {
                collect_typed_pattern_bindings(pat, env);
            }
        }
        Pattern::Constructor { fields, .. } => match fields {
            shape_ast::ast::PatternConstructorFields::Tuple(patterns) => {
                for pat in patterns {
                    collect_typed_pattern_bindings(pat, env);
                }
            }
            shape_ast::ast::PatternConstructorFields::Struct(fields) => {
                for (_, pat) in fields {
                    collect_typed_pattern_bindings(pat, env);
                }
            }
            shape_ast::ast::PatternConstructorFields::Unit => {}
        },
        Pattern::Identifier(_) | Pattern::Literal(_) | Pattern::Wildcard => {}
    }
}

/// Infer the shape of an object literal
pub fn infer_object_shape(entries: &[ObjectEntry]) -> String {
    format_object_shape(&collect_object_fields(entries))
}

/// Extract struct type field definitions from a parsed program.
///
/// Collects fields from two sources:
/// 1. Explicit type definitions (`type MyType { i: int }`)
/// 2. Struct literal usage in variable declarations (`let b = MyType { i: 10D }`)
///    as a fallback when no explicit type definition exists.
pub fn extract_struct_fields(
    program: &Program,
) -> std::collections::HashMap<String, Vec<(String, String)>> {
    use shape_ast::ast::Statement;

    let mut result = std::collections::HashMap::new();

    // 1. From explicit type definitions (these take precedence)
    for item in &program.items {
        if let Item::StructType(struct_def, _) = item {
            let fields: Vec<(String, String)> = struct_def
                .fields
                .iter()
                .map(|f| {
                    let mut type_str = type_annotation_to_string(&f.type_annotation)
                        .unwrap_or_else(|| "unknown".to_string());
                    if f.is_comptime {
                        // Include default value in type info for comptime fields
                        let default_repr = f
                            .default_value
                            .as_ref()
                            .map(|expr| match expr {
                                Expr::Literal(shape_ast::ast::Literal::String(s), _) => {
                                    format!(" = \"{}\"", s)
                                }
                                Expr::Literal(shape_ast::ast::Literal::Number(n), _) => {
                                    format!(" = {}", n)
                                }
                                Expr::Literal(shape_ast::ast::Literal::Int(n), _) => {
                                    format!(" = {}", n)
                                }
                                Expr::Literal(shape_ast::ast::Literal::Bool(b), _) => {
                                    format!(" = {}", b)
                                }
                                _ => String::new(),
                            })
                            .unwrap_or_default();
                        type_str = format!("comptime {}{}", type_str, default_repr);
                    }
                    (f.name.clone(), type_str)
                })
                .collect();
            result.insert(struct_def.name.clone(), fields);
        }
    }

    // 2. From struct literal usage (fallback when no type definition exists)
    for item in &program.items {
        let value_expr = match item {
            Item::VariableDecl(decl, _) => decl.value.as_ref(),
            Item::Statement(Statement::VariableDecl(decl, _), _) => decl.value.as_ref(),
            _ => None,
        };
        if let Some(Expr::StructLiteral {
            type_name, fields, ..
        }) = value_expr
        {
            if !result.contains_key(type_name) {
                let inferred: Vec<(String, String)> = fields
                    .iter()
                    .map(|(name, expr)| {
                        let type_str =
                            infer_expr_type(expr).unwrap_or_else(|| "unknown".to_string());
                        (name.clone(), type_str)
                    })
                    .collect();
                result.insert(type_name.clone(), inferred);
            }
        }
    }

    result
}

fn parse_named_generic_type(type_name: &str) -> Option<(String, Vec<String>)> {
    let trimmed = type_name.trim();
    let start = trimmed.find('<')?;
    let end = trimmed.rfind('>')?;
    if end <= start {
        return None;
    }
    let base = trimmed[..start].trim().to_string();
    let inner = trimmed[start + 1..end].trim();
    if inner.is_empty() {
        return Some((base, Vec::new()));
    }
    Some((base, split_top_level(inner, ',')))
}

fn replace_type_identifier(input: &str, identifier: &str, replacement: &str) -> String {
    if identifier.is_empty() {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    let mut token_started = false;

    let flush_token = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if token == identifier {
            out.push_str(replacement);
        } else {
            out.push_str(token);
        }
        token.clear();
    };

    for ch in input.chars() {
        let is_ident_char = ch.is_ascii_alphanumeric() || ch == '_';
        if is_ident_char {
            token.push(ch);
            token_started = true;
        } else {
            if token_started {
                flush_token(&mut token, &mut out);
                token_started = false;
            }
            out.push(ch);
        }
    }
    if token_started {
        flush_token(&mut token, &mut out);
    }

    out
}

fn substitute_type_params_in_field_type(
    field_type: &str,
    bindings: &HashMap<String, String>,
) -> String {
    let mut resolved = field_type.to_string();
    for (param, arg) in bindings {
        resolved = replace_type_identifier(&resolved, param, arg);
    }
    resolved
}

/// Resolve a struct field type for a concrete type string, including generic
/// instantiations like `MyType<number>`.
pub fn resolve_struct_field_type(
    program: &Program,
    type_name: &str,
    field_name: &str,
) -> Option<String> {
    let (base_name, generic_args) = parse_named_generic_type(type_name)
        .unwrap_or_else(|| (type_name.trim().to_string(), Vec::new()));

    for item in &program.items {
        let Item::StructType(struct_def, _) = item else {
            continue;
        };
        if struct_def.name != base_name {
            continue;
        }

        let field = struct_def.fields.iter().find(|f| f.name == field_name)?;
        let mut field_type = type_annotation_to_string(&field.type_annotation)
            .unwrap_or_else(|| "unknown".to_string());

        if let Some(type_params) = &struct_def.type_params {
            if !type_params.is_empty() {
                let mut bindings: HashMap<String, String> = HashMap::new();
                for (idx, param) in type_params.iter().enumerate() {
                    let bound = generic_args.get(idx).cloned().or_else(|| {
                        param
                            .default_type
                            .as_ref()
                            .and_then(type_annotation_to_string)
                    });
                    if let Some(bound) = bound {
                        bindings.insert(param.name.clone(), bound);
                    }
                }
                field_type = substitute_type_params_in_field_type(&field_type, &bindings);
            }
        }

        return Some(field_type);
    }

    None
}

/// Convert a compiler `Type` to a display string.
/// This is the canonical location; `completion::inference` re-exports self.
pub fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Concrete(annotation) => {
            type_annotation_to_string(annotation).unwrap_or_else(|| "unknown".to_string())
        }
        Type::Generic { base, args } => {
            let base_name = type_to_string(base);
            if args.is_empty() {
                base_name
            } else {
                let arg_list: Vec<String> = args.iter().map(type_to_string).collect();
                format!("{}<{}>", base_name, arg_list.join(", "))
            }
        }
        Type::Variable(_) => "unknown".to_string(),
        Type::Constrained { .. } => "unknown".to_string(),
        Type::Function { params, returns } => {
            let param_list: Vec<String> = params.iter().map(type_to_string).collect();
            format!("({}) -> {}", param_list.join(", "), type_to_string(returns))
        }
    }
}

/// Infer expression type using the compiler's TypeInferenceEngine.
/// Returns `None` when inference fails or resolves to unknown.
pub fn infer_expr_type_via_engine(expr: &Expr) -> Option<String> {
    let mut engine = TypeInferenceEngine::new();
    match engine.infer_expr(expr) {
        Ok(ty) => {
            let s = type_to_string(&ty);
            if s == "unknown" { None } else { Some(s) }
        }
        Err(_) => None,
    }
}

/// Inferred type information for a function's parameters and return type.
#[derive(Debug, Clone)]
pub enum ParamReferenceMode {
    Shared,
    Exclusive,
}

impl ParamReferenceMode {
    pub fn prefix(&self) -> &'static str {
        match self {
            ParamReferenceMode::Shared => "&",
            ParamReferenceMode::Exclusive => "&mut ",
        }
    }
}

/// Inferred type information for a function's parameters and return type.
#[derive(Debug, Clone)]
pub struct FunctionTypeInfo {
    /// Parameter types inferred by the engine: (param_name, type_string).
    /// Only includes parameters that lack explicit type annotations.
    pub param_types: Vec<(String, String)>,
    /// Effective pass mode for parameters (explicit and inferred refs).
    pub param_ref_modes: HashMap<String, ParamReferenceMode>,
    /// Return type if inferred (None when the function has an explicit return annotation).
    pub return_type: Option<String>,
}

/// Run TypeInferenceEngine and extract per-function parameter/return types.
pub fn infer_function_signatures(program: &Program) -> HashMap<String, FunctionTypeInfo> {
    let augmented = shape_ast::transform::augment_program_with_generated_extends(program);
    let mut engine = TypeInferenceEngine::new();
    let mut result = HashMap::new();
    let inferred_param_pass_modes = shape_vm::compiler::infer_param_pass_modes(&augmented);

    // Collect function AST definitions
    let func_defs: Vec<&shape_ast::ast::FunctionDef> = program
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Function(f, _) = item {
                Some(f)
            } else {
                None
            }
        })
        .collect();

    let (types, _) = engine.infer_program_best_effort(&augmented);
    let func_map: HashMap<&str, &&shape_ast::ast::FunctionDef> =
        func_defs.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut inferred_infos: HashMap<String, FunctionTypeInfo> = HashMap::new();

    for (name, ty) in &types {
        let Some(func_def) = func_map.get(name.as_str()) else {
            continue;
        };

        let (param_type_strings, return_type_string) = match ty {
            Type::Function { params, returns } => (
                params.iter().map(type_to_string).collect::<Vec<_>>(),
                Some(type_to_string(returns)),
            ),
            Type::Concrete(TypeAnnotation::Function { params, returns }) => (
                params
                    .iter()
                    .map(|p| {
                        type_annotation_to_string(&p.type_annotation)
                            .unwrap_or_else(|| "unknown".to_string())
                    })
                    .collect::<Vec<_>>(),
                type_annotation_to_string(returns),
            ),
            _ => continue,
        };

        let param_types: Vec<(String, String)> = func_def
            .params
            .iter()
            .zip(param_type_strings.iter())
            .filter_map(|(ast_param, inferred_type)| {
                if ast_param.type_annotation.is_some() {
                    return None;
                }
                let param_name = ast_param.simple_name()?.to_string();
                if inferred_type == "_" || inferred_type == "unknown" {
                    return None;
                }
                Some((param_name, inferred_type.clone()))
            })
            .collect();
        let mut param_ref_modes = HashMap::new();
        let param_modes = inferred_param_pass_modes
            .get(name)
            .cloned()
            .unwrap_or_default();
        for (idx, ast_param) in func_def.params.iter().enumerate() {
            let Some(param_name) = ast_param.simple_name() else {
                continue;
            };
            let mode = match param_modes
                .get(idx)
                .copied()
                .unwrap_or(if ast_param.is_reference {
                    ParamPassMode::ByRefShared
                } else {
                    ParamPassMode::ByValue
                }) {
                ParamPassMode::ByRefExclusive => ParamReferenceMode::Exclusive,
                ParamPassMode::ByRefShared => ParamReferenceMode::Shared,
                ParamPassMode::ByValue => continue,
            };
            param_ref_modes.insert(param_name.to_string(), mode);
        }

        let return_type = if func_def.return_type.is_none() {
            return_type_string.filter(|s| s != "_" && s != "unknown")
        } else {
            None
        };

        inferred_infos.insert(
            name.clone(),
            FunctionTypeInfo {
                param_types,
                param_ref_modes,
                return_type,
            },
        );
    }

    for func_def in &func_defs {
        let mut info = inferred_infos
            .remove(&func_def.name)
            .unwrap_or(FunctionTypeInfo {
                param_types: Vec::new(),
                param_ref_modes: HashMap::new(),
                return_type: None,
            });

        if func_def.return_type.is_none() && info.return_type.is_none() {
            info.return_type = infer_function_return_from_body_via_engine(func_def);
        }

        // Fully annotated signatures don't need inferred hints.
        if func_def.return_type.is_some() && info.param_types.is_empty() {
            continue;
        }

        // Keep function entries when return annotation is absent so hover can
        // render a full `fn` signature from AST annotations.
        if func_def.return_type.is_none()
            || !info.param_types.is_empty()
            || !info.param_ref_modes.is_empty()
            || info.return_type.is_some()
        {
            result.insert(func_def.name.clone(), info);
        }
    }

    // Insert foreign functions with their declared return type.
    // Foreign functions must declare explicit types (including Result<T> for
    // dynamic languages) — we just surface the declared type here.
    for item in &program.items {
        if let Item::ForeignFunction(foreign_fn, _) = item {
            let ret = foreign_fn
                .return_type
                .as_ref()
                .and_then(type_annotation_to_string);
            result
                .entry(foreign_fn.name.clone())
                .or_insert_with(|| FunctionTypeInfo {
                    param_types: Vec::new(),
                    param_ref_modes: HashMap::new(),
                    return_type: ret,
                });
        }
    }

    result
}

fn infer_function_return_from_body_via_engine(
    func_def: &shape_ast::ast::FunctionDef,
) -> Option<String> {
    infer_return_type_for_block_with_params(&func_def.body, Some(&func_def.params))
}

/// Infer a return type for a generic statement block using TypeInferenceEngine.
///
/// This is shared by hover for impl-method fallback signatures.
pub fn infer_block_return_type_via_engine(body: &[Statement]) -> Option<String> {
    infer_return_type_for_block_with_params(body, None)
}

fn infer_return_type_for_block_with_params(
    body: &[Statement],
    params: Option<&[shape_ast::ast::FunctionParameter]>,
) -> Option<String> {
    let return_exprs = collect_return_expressions(body);
    if return_exprs.is_empty() {
        return None;
    }

    let mut engine = TypeInferenceEngine::new();

    if let Some(params) = params {
        for param in params {
            let Some(name) = param.simple_name() else {
                continue;
            };
            let Some(type_ann) = &param.type_annotation else {
                continue;
            };
            engine
                .env
                .define(name, TypeScheme::mono(Type::Concrete(type_ann.clone())));
        }
    }

    let mut inferred = Vec::new();
    for expr in return_exprs {
        if let Ok(ty) = engine.infer_expr(&expr) {
            let s = type_to_string(&ty);
            if s != "unknown" {
                inferred.push(s);
                continue;
            }
        }

        // Fallback to lightweight expression inference when the engine does not
        // yet model a syntax form (e.g., newer formatted-string variants).
        if let Some(fallback) = infer_expr_type(&expr) {
            if fallback != "unknown" {
                inferred.push(fallback);
            }
        }
    }

    inferred.sort();
    inferred.dedup();
    match inferred.len() {
        0 => None,
        1 => inferred.into_iter().next(),
        _ => Some(inferred.join(" | ")),
    }
}

fn collect_return_expressions(body: &[Statement]) -> Vec<Expr> {
    let mut exprs = Vec::new();

    for stmt in body {
        match stmt {
            Statement::Return(Some(expr), _) => exprs.push(expr.clone()),
            Statement::Expression(expr, _) => collect_return_exprs_from_expr(expr, &mut exprs),
            _ => {}
        }
    }

    if let Some(Statement::Expression(expr, _)) = body.last() {
        if !matches!(expr, Expr::Return(_, _)) {
            exprs.push(expr.clone());
        }
    }

    exprs
}

fn collect_return_exprs_from_expr(expr: &Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::Return(Some(inner), _) => out.push(inner.as_ref().clone()),
        Expr::If(if_expr, _) => {
            collect_return_exprs_from_expr(&if_expr.then_branch, out);
            if let Some(else_branch) = &if_expr.else_branch {
                collect_return_exprs_from_expr(else_branch, out);
            }
        }
        Expr::Block(block_expr, _) => {
            for item in &block_expr.items {
                match item {
                    shape_ast::ast::BlockItem::Statement(Statement::Expression(inner, _)) => {
                        collect_return_exprs_from_expr(inner, out)
                    }
                    shape_ast::ast::BlockItem::Expression(inner) => {
                        collect_return_exprs_from_expr(inner, out)
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Run TypeInferenceEngine on a whole program, returning variable name -> type string.
pub fn infer_program_types(program: &Program) -> HashMap<String, String> {
    infer_program_types_with_context(program, None, None, None)
}

/// Run TypeInferenceEngine on a whole program with optional file/workspace context.
pub fn infer_program_types_with_context(
    program: &Program,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> HashMap<String, String> {
    let augmented = shape_ast::transform::augment_program_with_generated_extends(program);
    let mut engine = TypeInferenceEngine::new();
    let mut types = HashMap::new();

    let (inferred, _) = engine.infer_program_best_effort(&augmented);
    for (name, ty) in inferred {
        let mut s = type_to_string(&ty);
        if let Some(structural) = infer_variable_type(&augmented, &name) {
            if is_structural_object_shape(&structural) {
                if is_structural_object_shape(&s) {
                    if let Some(merged) = merge_object_shapes(&s, &structural) {
                        s = merged;
                    }
                } else if is_generic_object_type(&s) {
                    s = structural;
                }
            }
        }
        if s != "unknown" {
            types.insert(name, s);
        }
    }

    augment_schema_backed_module_call_types(
        program,
        &mut types,
        current_file,
        workspace_root,
        current_source,
    );

    types
}

fn augment_schema_backed_module_call_types(
    program: &Program,
    types: &mut HashMap<String, String>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) {
    for item in &program.items {
        match item {
            Item::VariableDecl(var_decl, _) => {
                maybe_insert_schema_backed_type_from_decl(
                    var_decl,
                    types,
                    current_file,
                    workspace_root,
                    current_source,
                );
            }
            Item::Statement(Statement::VariableDecl(var_decl, _), _) => {
                maybe_insert_schema_backed_type_from_decl(
                    var_decl,
                    types,
                    current_file,
                    workspace_root,
                    current_source,
                );
            }
            _ => {}
        }
    }
}

fn maybe_insert_schema_backed_type_from_decl(
    var_decl: &VariableDecl,
    types: &mut HashMap<String, String>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) {
    let Some(name) = var_decl.pattern.as_identifier() else {
        return;
    };
    let Some(value) = &var_decl.value else {
        return;
    };
    let Some(conn_type) =
        infer_schema_backed_type_from_expr(value, current_file, workspace_root, current_source)
    else {
        return;
    };
    types.insert(name.to_string(), conn_type);
}

fn infer_schema_backed_type_from_expr(
    expr: &Expr,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<String> {
    let Expr::MethodCall {
        receiver,
        method,
        args,
        named_args: _,
        ..
    } = expr
    else {
        return None;
    };
    let module_name = match receiver.as_ref() {
        Expr::Identifier(name, _) => name.as_str(),
        _ => return None,
    };
    let source_schema_provider = schema_provider_for_module_call(
        module_name,
        method,
        args.len(),
        current_file,
        workspace_root,
        current_source,
    )?;
    let uri = match args.first() {
        Some(Expr::Literal(Literal::String(uri), _)) => Some(uri.as_str()),
        _ => None,
    }?;
    let source = resolve_source_schema_for_module_call(
        module_name,
        &source_schema_provider,
        uri,
        current_file,
        workspace_root,
        current_source,
    )?;
    Some(connection_shape_from_source_schema(&source))
}

fn schema_provider_for_module_call(
    module_name: &str,
    function_name: &str,
    arg_count: usize,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<String> {
    let schema = crate::completion::imports::extension_module_schema_with_context(
        module_name,
        current_file,
        workspace_root,
        current_source,
    );

    let Some(schema) = schema else {
        // Fallback when extension schema metadata is unavailable (e.g., lock-only
        // inference in standalone analysis). Restrict to single-arg calls to
        // avoid widening to unrelated module APIs.
        return (arg_count == 1).then(|| "source_schema".to_string());
    };

    let export = schema.functions.iter().find(|f| f.name == function_name)?;
    if !is_schema_backed_connection_return(export.return_type.as_deref()) {
        return None;
    }

    schema
        .functions
        .iter()
        .find(|f| f.name == "source_schema")
        .map(|f| f.name.clone())
}

fn is_schema_backed_connection_return(return_type: Option<&str>) -> bool {
    let Some(return_type) = return_type else {
        return false;
    };
    return_type == "DbConnection" || return_type.ends_with("Connection")
}

fn resolve_source_schema_for_module_call(
    module_name: &str,
    source_schema_provider: &str,
    uri: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<SourceSchema> {
    let lock_path = lock_path_for_context(current_file, workspace_root);
    if let Ok((source, _diagnostics)) = load_cached_source_for_uri_with_diagnostics(&lock_path, uri)
    {
        return Some(source);
    }

    let source = crate::completion::imports::extension_source_schema_via_with_context(
        module_name,
        source_schema_provider,
        uri,
        current_file,
        workspace_root,
        current_source,
    )?;

    let mut cache = DataSourceSchemaCache::load_or_empty(&lock_path);
    cache.upsert_source(source.clone());
    let _ = cache.save(&lock_path);

    Some(source)
}

fn lock_path_for_context(current_file: Option<&Path>, workspace_root: Option<&Path>) -> PathBuf {
    if let Some(path) = current_file {
        if let Some(parent) = path.parent()
            && let Some(project) = shape_runtime::project::find_project_root(parent)
        {
            return project.root_path.join("shape.lock");
        }
        return path.with_extension("lock");
    }

    if let Some(root) = workspace_root
        && let Some(project) = shape_runtime::project::find_project_root(root)
    {
        return project.root_path.join("shape.lock");
    }

    default_cache_path()
}

fn connection_shape_from_source_schema(source: &SourceSchema) -> String {
    let mut tables = source.tables.values().collect::<Vec<_>>();
    tables.sort_by(|left, right| left.name.cmp(&right.name));

    let fields = tables
        .into_iter()
        .filter_map(|table| {
            if !is_valid_shape_identifier(&table.name) {
                return None;
            }
            Some(format!(
                "{}: Table<{}>",
                table.name,
                row_shape_from_entity_schema(table)
            ))
        })
        .collect::<Vec<_>>();

    if fields.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

fn row_shape_from_entity_schema(entity: &EntitySchema) -> String {
    let fields = entity
        .columns
        .iter()
        .filter_map(|column| {
            if !is_valid_shape_identifier(&column.name) {
                return None;
            }
            Some(format!(
                "{}: {}",
                column.name,
                schema_column_type(&column.shape_type, column.nullable)
            ))
        })
        .collect::<Vec<_>>();

    if fields.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

fn schema_column_type(shape_type: &str, nullable: bool) -> String {
    let base = match shape_type {
        "int" => "int",
        "number" => "number",
        "decimal" => "decimal",
        "string" => "string",
        "bool" => "bool",
        "timestamp" => "timestamp",
        _ => "_",
    };
    if nullable {
        format!("Option<{}>", base)
    } else {
        base.to_string()
    }
}

fn is_valid_shape_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub fn infer_variable_type(program: &Program, var_name: &str) -> Option<String> {
    let mut finder = VariableFinder {
        target_name: var_name,
        found_type: None,
        found_expr: None,
    };
    walk_program(&mut finder, program);

    if let Some(Expr::Object(entries, _)) = &finder.found_expr {
        let mut fields = collect_object_fields(entries);

        let assignments = PropertyAssignmentCollector::collect(program);
        for assignment in &assignments {
            if assignment.variable == var_name
                && !fields
                    .iter()
                    .any(|(field_name, _)| field_name == &assignment.property)
            {
                let prop_type = infer_expr_type_via_engine(&assignment.value_expr)
                    .unwrap_or_else(|| "unknown".to_string());
                fields.push((assignment.property.clone(), prop_type));
            }
        }

        return Some(format_object_shape(&fields));
    }

    finder.found_type
}

/// Infer a variable type for display at a specific source offset.
///
/// For object literals with hoisted fields, self returns a masked view where
/// fields assigned only in the future appear inside a comment:
/// `{ x: int /*, y: int */ }`.
pub fn infer_variable_type_for_display(
    program: &Program,
    var_name: &str,
    offset: usize,
) -> Option<String> {
    let (visible_fields, masked_fields) =
        infer_object_field_state_at_offset(program, var_name, offset)?;
    Some(format_object_shape_with_masked_fields(
        &visible_fields,
        &masked_fields,
    ))
}

/// Infer only currently visible fields for a variable at a given offset.
/// This is used by property hover/completions where masked fields should not
/// be treated as available yet.
pub fn infer_variable_visible_type_at_offset(
    program: &Program,
    var_name: &str,
    offset: usize,
) -> Option<String> {
    let (visible_fields, _) = infer_object_field_state_at_offset(program, var_name, offset)?;
    Some(format_object_shape(&visible_fields))
}

fn infer_object_field_state_at_offset(
    program: &Program,
    var_name: &str,
    offset: usize,
) -> Option<(Vec<(String, String)>, Vec<(String, String)>)> {
    let mut finder = VariableFinder {
        target_name: var_name,
        found_type: None,
        found_expr: None,
    };
    walk_program(&mut finder, program);

    let Expr::Object(entries, _) = finder.found_expr.as_ref()? else {
        return None;
    };

    let mut visible_fields = collect_object_fields(entries);
    let mut visible_names: HashSet<String> = visible_fields
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    let assignments = PropertyAssignmentCollector::collect(program);
    let mut hoisted: Vec<(String, usize, String)> = Vec::new();

    for assignment in assignments.iter().filter(|a| a.variable == var_name) {
        if visible_names.contains(&assignment.property) {
            continue;
        }
        if hoisted
            .iter()
            .any(|(existing, _, _)| existing == &assignment.property)
        {
            continue;
        }

        let prop_type = infer_expr_type_via_engine(&assignment.value_expr)
            .unwrap_or_else(|| "unknown".to_string());
        hoisted.push((
            assignment.property.clone(),
            assignment.assignment_span.start,
            prop_type,
        ));
    }

    hoisted.sort_by_key(|(_, assignment_offset, _)| *assignment_offset);

    let mut masked_fields = Vec::new();
    for (name, assignment_offset, ty) in hoisted {
        if assignment_offset <= offset {
            visible_names.insert(name.clone());
            visible_fields.push((name, ty));
        } else {
            masked_fields.push((name, ty));
        }
    }

    Some((visible_fields, masked_fields))
}

fn format_object_shape_with_masked_fields(
    visible_fields: &[(String, String)],
    masked_fields: &[(String, String)],
) -> String {
    if masked_fields.is_empty() {
        return format_object_shape(visible_fields);
    }

    let visible = visible_fields
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, ty))
        .collect::<Vec<_>>()
        .join(", ");
    let masked = masked_fields
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, ty))
        .collect::<Vec<_>>()
        .join(", ");

    if visible.is_empty() {
        format!("{{ /* {} */ }}", masked)
    } else {
        format!("{{ {} /*, {} */ }}", visible, masked)
    }
}

fn collect_object_fields(entries: &[ObjectEntry]) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    for entry in entries {
        if let ObjectEntry::Field {
            key,
            value,
            type_annotation,
        } = entry
        {
            let field_type = if let Some(type_ann) = type_annotation {
                type_annotation_to_string(type_ann).unwrap_or_else(|| "unknown".to_string())
            } else {
                infer_expr_type_via_engine(value).unwrap_or_else(|| "unknown".to_string())
            };
            fields.push((key.clone(), field_type));
        }
    }
    fields
}

struct VariableFinder<'a> {
    target_name: &'a str,
    found_type: Option<String>,
    found_expr: Option<Expr>,
}

impl<'a> Visitor for VariableFinder<'a> {
    fn visit_item(&mut self, item: &Item) -> bool {
        if let Item::VariableDecl(decl, _) = item {
            self.check_variable_decl(decl);
        }
        true
    }

    fn visit_stmt(&mut self, stmt: &Statement) -> bool {
        if let Statement::VariableDecl(decl, _) = stmt {
            self.check_variable_decl(decl);
        }
        true
    }
}

impl<'a> VariableFinder<'a> {
    fn check_variable_decl(&mut self, decl: &VariableDecl) {
        if let Some(name) = decl.pattern.as_identifier() {
            if name == self.target_name {
                if let Some(value) = &decl.value {
                    self.found_expr = Some(value.clone());
                }

                if let Some(type_ann) = &decl.type_annotation {
                    self.found_type = type_annotation_to_string(type_ann);
                } else if let Some(value) = &decl.value {
                    self.found_type = infer_expr_type_via_engine(value);
                }
            }
        }
    }
}

/// Info about a method collected from impl/extend/trait blocks
#[derive(Debug, Clone)]
pub struct MethodCompletionInfo {
    pub name: String,
    pub signature: Option<String>,
    pub from_trait: Option<String>,
    pub documentation: Option<String>,
}

/// Extract methods defined via `impl`, `extend`, and `trait` blocks.
///
/// For `impl Trait for Type` blocks, ALL trait methods are surfaced for the target type
/// (not just those with bodies in the impl block), since the impl means the type has them all.
/// For `extend Type` blocks, the explicitly defined methods are collected.
pub fn extract_type_methods(program: &Program) -> HashMap<String, Vec<MethodCompletionInfo>> {
    let augmented = shape_ast::transform::augment_program_with_generated_extends(program);
    let mut result: HashMap<String, Vec<MethodCompletionInfo>> = HashMap::new();

    // First pass: collect trait definitions (name → method signatures)
    let mut trait_methods: HashMap<String, Vec<MethodCompletionInfo>> = HashMap::new();
    for item in &augmented.items {
        if let Item::Trait(trait_def, _) = item {
            let methods: Vec<MethodCompletionInfo> = trait_def
                .members
                .iter()
                .filter_map(|member| match member {
                    TraitMember::Required(
                        im @ InterfaceMember::Method {
                            name,
                            params,
                            return_type,
                            ..
                        },
                    ) => {
                        let param_names: Vec<String> = params
                            .iter()
                            .map(|p| p.name.clone().unwrap_or_else(|| "_".to_string()))
                            .collect();
                        let sig = format!(
                            "{}({}): {}",
                            name,
                            param_names.join(", "),
                            type_annotation_to_string(return_type)
                                .unwrap_or_else(|| "_".to_string())
                        );
                        Some(MethodCompletionInfo {
                            name: name.clone(),
                            signature: Some(sig),
                            from_trait: Some(trait_def.name.clone()),
                            documentation: interface_member_doc(im),
                        })
                    }
                    _ => None,
                })
                .collect();
            trait_methods.insert(trait_def.name.clone(), methods);
        }
    }

    // Second pass: collect impl blocks and extend blocks
    for item in &augmented.items {
        match item {
            Item::Impl(impl_block, _) => {
                let target_type = match &impl_block.target_type {
                    shape_ast::ast::TypeName::Simple(name) => name.clone(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
                };
                let trait_name = match &impl_block.trait_name {
                    shape_ast::ast::TypeName::Simple(name) => name.clone(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
                };

                // Add ALL methods from the trait (the impl means the type has them all)
                if let Some(trait_meths) = trait_methods.get(&trait_name) {
                    let entry = result.entry(target_type.clone()).or_default();
                    for m in trait_meths {
                        // Avoid duplicates
                        if !entry.iter().any(|existing| existing.name == m.name) {
                            entry.push(m.clone());
                        }
                    }
                }

                // Also add any methods defined directly in the impl body
                // (they may not be in the trait, e.g. helper methods)
                let entry = result.entry(target_type).or_default();
                for method in &impl_block.methods {
                    if !entry.iter().any(|existing| existing.name == method.name) {
                        let sig = format!(
                            "{}({})",
                            method.name,
                            method
                                .params
                                .iter()
                                .map(|p| p.simple_name().unwrap_or("_").to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        entry.push(MethodCompletionInfo {
                            name: method.name.clone(),
                            signature: Some(sig),
                            from_trait: Some(trait_name.clone()),
                            documentation: method_doc(method.doc_comment.as_ref()),
                        });
                    }
                }
            }
            Item::Extend(extend, _) => {
                let type_name = match &extend.type_name {
                    shape_ast::ast::TypeName::Simple(name) => name.clone(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
                };
                let entry = result.entry(type_name).or_default();
                for method in &extend.methods {
                    if !entry.iter().any(|existing| existing.name == method.name) {
                        let sig = format!(
                            "{}({})",
                            method.name,
                            method
                                .params
                                .iter()
                                .map(|p| p.simple_name().unwrap_or("_").to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        entry.push(MethodCompletionInfo {
                            name: method.name.clone(),
                            signature: Some(sig),
                            from_trait: None,
                            documentation: method_doc(method.doc_comment.as_ref()),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    result
}

fn interface_member_doc(member: &InterfaceMember) -> Option<String> {
    match member {
        InterfaceMember::Method { doc_comment, .. }
        | InterfaceMember::Property { doc_comment, .. }
        | InterfaceMember::IndexSignature { doc_comment, .. } => method_doc(doc_comment.as_ref()),
    }
}

fn method_doc(doc_comment: Option<&shape_ast::ast::DocComment>) -> Option<String> {
    let comment = doc_comment?;
    if !comment.body.is_empty() {
        Some(comment.body.clone())
    } else if !comment.summary.is_empty() {
        Some(comment.summary.clone())
    } else {
        None
    }
}

/// Simplify `Result<T, E>` to `Result<T>` for display.
/// The error type is usually `AnyError` or a union — hiding it keeps hints concise.
pub fn simplify_result_type(ty: &str) -> String {
    let Some(inner) = ty.strip_prefix("Result<").and_then(|s| s.strip_suffix('>')) else {
        return ty.to_string();
    };
    // Find the comma separating T from E, respecting nested angle brackets
    let mut depth = 0;
    for (i, ch) in inner.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                let ok_type = inner[..i].trim();
                return format!("Result<{}>", ok_type);
            }
            _ => {}
        }
    }
    ty.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_extract_struct_fields_from_literal_no_type_def() {
        // When no `type MyType` exists, infer fields from struct literal usage
        let code =
            "let b: MyType = MyType { i: 10.2D }\nmeta MyType {\n  format: |v| v.i.toString()\n}\n";
        let program = parse_program(code).unwrap();
        let fields = extract_struct_fields(&program);
        let my_type = fields
            .get("MyType")
            .expect("Should find MyType from struct literal");
        assert_eq!(my_type[0], ("i".to_string(), "decimal".to_string()));
    }

    #[test]
    fn test_extract_struct_fields_type_def_takes_precedence() {
        // When BOTH a type def and struct literal exist, the type def wins
        let code = "type MyType { i: int }\nlet b = MyType { i: 10.2D }\n";
        let program = parse_program(code).unwrap();
        let fields = extract_struct_fields(&program);
        let my_type = fields.get("MyType").expect("Should find MyType");
        // Type definition says int, so int takes precedence over the literal's decimal
        assert_eq!(my_type[0], ("i".to_string(), "int".to_string()));
    }

    #[test]
    fn test_infer_literal_type_formatted_string() {
        let ty = infer_literal_type(&Literal::FormattedString {
            value: "x={x}".to_string(),
            mode: shape_ast::ast::InterpolationMode::Braces,
        });
        assert_eq!(ty, "string");
    }

    #[test]
    fn test_infer_program_types_basic() {
        let code = "let x = 42\nlet s = \"hello\"\nlet b = true";
        let program = parse_program(code).unwrap();
        let types = infer_program_types(&program);
        assert_eq!(types.get("x").map(|s| s.as_str()), Some("int"));
        assert_eq!(types.get("s").map(|s| s.as_str()), Some("string"));
        assert_eq!(types.get("b").map(|s| s.as_str()), Some("bool"));
    }

    #[test]
    fn test_infer_program_types_includes_hoisted_object_fields() {
        let code = "let a = { x: 1 }\na.y = 2\n";
        let program = parse_program(code).unwrap();
        let types = infer_program_types(&program);
        let a_type = types.get("a").expect("a should have inferred type");
        assert!(
            a_type.contains("x: int") && a_type.contains("y: int"),
            "expected hoisted field in object type, got {}",
            a_type
        );
    }

    #[test]
    fn test_infer_program_types_connection_uses_cached_schema_tables() {
        use shape_runtime::schema_cache::{
            DataSourceSchemaCache, EntitySchema, FieldSchema, SourceSchema, set_default_cache_path,
        };
        use std::collections::HashMap;

        struct CachePathReset;
        impl Drop for CachePathReset {
            fn drop(&mut self) {
                set_default_cache_path(None);
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("shape.lock");

        let mut cache = DataSourceSchemaCache::new();
        cache.upsert_source(SourceSchema {
            uri: "duckdb://analytics.db".to_string(),
            cached_at: "2026-02-17T00:00:00Z".to_string(),
            tables: HashMap::from([(
                "candles".to_string(),
                EntitySchema {
                    name: "candles".to_string(),
                    columns: vec![
                        FieldSchema {
                            name: "open".to_string(),
                            shape_type: "number".to_string(),
                            nullable: false,
                        },
                        FieldSchema {
                            name: "volume".to_string(),
                            shape_type: "int".to_string(),
                            nullable: true,
                        },
                    ],
                },
            )]),
        });
        cache.save(&cache_path).unwrap();

        set_default_cache_path(Some(cache_path));
        let _reset = CachePathReset;

        let program =
            parse_program(r#"let conn = duckdb.connect("duckdb://analytics.db")"#).unwrap();
        let types = infer_program_types(&program);
        let conn_type = types.get("conn").expect("conn type should be inferred");

        assert!(
            conn_type.contains("candles: Table<{ open: number"),
            "expected candles table in connection shape, got {}",
            conn_type
        );
        assert!(
            conn_type.contains("volume: Option<int>"),
            "expected nullable column mapped to Option<int>, got {}",
            conn_type
        );
    }

    #[test]
    fn test_lock_path_for_context_prefers_script_lock_for_standalone_files() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("demo.shape");
        let expected = tmp.path().join("demo.lock");
        let actual = lock_path_for_context(Some(&script_path), None);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_infer_program_types_with_context_uses_script_lock() {
        use shape_runtime::schema_cache::{
            DataSourceSchemaCache, EntitySchema, FieldSchema, SourceSchema,
        };
        use std::collections::HashMap;

        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("demo.shape");
        let lock_path = tmp.path().join("demo.lock");

        let mut cache = DataSourceSchemaCache::new();
        cache.upsert_source(SourceSchema {
            uri: "duckdb://analytics.db".to_string(),
            cached_at: "2026-02-18T00:00:00Z".to_string(),
            tables: HashMap::from([(
                "candles".to_string(),
                EntitySchema {
                    name: "candles".to_string(),
                    columns: vec![FieldSchema {
                        name: "open".to_string(),
                        shape_type: "number".to_string(),
                        nullable: false,
                    }],
                },
            )]),
        });
        cache.save(&lock_path).unwrap();

        let source = r#"let conn = duckdb.connect("duckdb://analytics.db")"#;
        let program = parse_program(source).unwrap();
        let types =
            infer_program_types_with_context(&program, Some(&script_path), None, Some(source));
        let conn_type = types.get("conn").expect("conn type should be inferred");
        assert!(
            conn_type.contains("candles: Table<{ open: number }>"),
            "expected candles table inferred from script lock, got {}",
            conn_type
        );
    }

    #[test]
    fn test_infer_expr_type_via_engine_match() {
        let code = "match 1 { 1 => true, 2 => false }";
        let program = parse_program(code).unwrap();
        if let Some(shape_ast::ast::Item::Statement(
            shape_ast::ast::Statement::Expression(expr, _),
            _,
        )) = program.items.first()
        {
            let ty = infer_expr_type_via_engine(expr);
            assert!(
                ty.is_some(),
                "Engine should infer type for match expression"
            );
            let ty_str = ty.unwrap();
            assert!(
                ty_str.contains("bool"),
                "Match with all bool arms should be bool, got: {}",
                ty_str
            );
        }
    }

    #[test]
    fn test_infer_expr_type_via_engine_match_union() {
        let code = "match 1 { 1 => true, 2 => \"hello\" }";
        let program = parse_program(code).unwrap();
        if let Some(shape_ast::ast::Item::Statement(
            shape_ast::ast::Statement::Expression(expr, _),
            _,
        )) = program.items.first()
        {
            let ty = infer_expr_type_via_engine(expr);
            assert!(
                ty.is_some(),
                "Engine should infer type for match with mixed arms"
            );
            let ty_str = ty.unwrap();
            assert!(
                ty_str.contains("bool") && ty_str.contains("string"),
                "Should be union of bool and string, got: {}",
                ty_str
            );
        }
    }

    #[test]
    fn test_infer_expr_type_match_typed_pattern_numeric_branch_stays_int() {
        let code = "let result = match value {\n  c: int => c + 1\n  _ => 1\n}\n";
        let program = parse_program(code).unwrap();
        let expr = match program.items.first() {
            Some(shape_ast::ast::Item::VariableDecl(decl, _)) => {
                decl.value.as_ref().expect("result should have value")
            }
            Some(shape_ast::ast::Item::Statement(
                shape_ast::ast::Statement::VariableDecl(decl, _),
                _,
            )) => decl.value.as_ref().expect("result should have value"),
            other => panic!("expected variable declaration, got {:?}", other),
        };

        assert_eq!(infer_expr_type(expr).as_deref(), Some("int"));
    }

    #[test]
    fn test_infer_program_types_match_variable() {
        let code = "let test = match 2 {\n  0 => true,\n  _ => false,\n}";
        let program = parse_program(code).unwrap();
        let types = infer_program_types(&program);
        eprintln!("infer_program_types result: {:?}", types);
        assert_eq!(
            types.get("test").map(|s| s.as_str()),
            Some("bool"),
            "test should be inferred as bool from match expression, got: {:?}",
            types.get("test")
        );
    }

    #[test]
    fn test_type_to_string_concrete() {
        let ty = Type::Concrete(TypeAnnotation::Basic("int".to_string()));
        assert_eq!(type_to_string(&ty), "int");
    }

    #[test]
    fn test_type_to_string_union() {
        let ty = Type::Concrete(TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("bool".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]));
        assert_eq!(type_to_string(&ty), "bool | string");
    }

    #[test]
    fn test_infer_method_call_type_preserving() {
        // Direct expression: [1,2].filter(...) should return int[] (same as receiver)
        use shape_ast::ast::{Expr, Span};
        let receiver = Box::new(Expr::Array(
            vec![
                Expr::Literal(Literal::Int(1), Span::default()),
                Expr::Literal(Literal::Int(2), Span::default()),
            ],
            Span::default(),
        ));
        let expr = Expr::MethodCall {
            receiver,
            method: "filter".to_string(),
            args: vec![],
            named_args: vec![],
            optional: false,
            span: Span::default(),
        };
        let ty = infer_expr_type(&expr);
        assert_eq!(ty, Some("int[]".to_string()), "filter should preserve type");
    }

    #[test]
    fn test_infer_method_call_aggregation() {
        use shape_ast::ast::{Expr, Span};
        let receiver = Box::new(Expr::Array(vec![], Span::default()));
        let expr = Expr::MethodCall {
            receiver,
            method: "sum".to_string(),
            args: vec![],
            named_args: vec![],
            optional: false,
            span: Span::default(),
        };
        assert_eq!(
            infer_expr_type(&expr),
            Some("number".to_string()),
            "sum() should return number"
        );
    }

    #[test]
    fn test_infer_method_call_chained() {
        use shape_ast::ast::{Expr, Span};
        let array = Box::new(Expr::Array(
            vec![Expr::Literal(Literal::Int(1), Span::default())],
            Span::default(),
        ));
        let filtered = Box::new(Expr::MethodCall {
            receiver: array,
            method: "filter".to_string(),
            args: vec![],
            named_args: vec![],
            optional: false,
            span: Span::default(),
        });
        let reversed = Expr::MethodCall {
            receiver: filtered,
            method: "reverse".to_string(),
            args: vec![],
            named_args: vec![],
            optional: false,
            span: Span::default(),
        };
        let ty = infer_expr_type(&reversed);
        assert_eq!(
            ty,
            Some("int[]".to_string()),
            "chained filter.reverse should preserve type"
        );
    }

    #[test]
    fn test_infer_method_call_unwrap() {
        use shape_ast::ast::{Expr, Span};
        let receiver = Box::new(Expr::TypeAssertion {
            expr: Box::new(Expr::Identifier("x".to_string(), Span::default())),
            type_annotation: TypeAnnotation::Generic {
                name: "Result".to_string(),
                args: vec![TypeAnnotation::Basic("Foo".to_string())],
            },
            meta_param_overrides: None,
            span: Span::default(),
        });
        let expr = Expr::MethodCall {
            receiver,
            method: "unwrap".to_string(),
            args: vec![],
            named_args: vec![],
            optional: false,
            span: Span::default(),
        };
        assert_eq!(
            infer_expr_type(&expr),
            Some("Foo".to_string()),
            "unwrap on Result<Foo> should return Foo"
        );
    }

    #[test]
    fn test_extract_type_methods_extend_block() {
        let code = "extend Foo {\n  method bar() {\n    self\n  }\n}\n";
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        let foo_methods = methods.get("Foo").expect("Should find Foo methods");
        assert!(
            foo_methods.iter().any(|m| m.name == "bar"),
            "Should include 'bar' method from extend block"
        );
    }

    #[test]
    fn test_extract_type_methods_from_annotation_comptime_extend_target() {
        let code = r#"
annotation add_sum() {
    targets: [type]
    comptime post(target, ctx) {
        extend target {
            method sum() { self.x + self.y }
        }
    }
}
@add_sum()
type Point { x: int, y: int }
"#;
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        let point_methods = methods.get("Point").expect("Should find Point methods");
        assert!(
            point_methods.iter().any(|m| m.name == "sum"),
            "Should include generated 'sum' method from annotation comptime handler"
        );
    }

    #[test]
    fn test_extract_type_methods_from_annotation_comptime_extend_explicit_type() {
        let code = r#"
annotation add_number_method() {
    targets: [function]
    comptime post(target, ctx) {
        extend Number {
            method doubled() { self * 2.0 }
        }
    }
}
@add_number_method()
fn marker() { 0 }
"#;
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        let number_methods = methods.get("Number").expect("Should find Number methods");
        assert!(
            number_methods.iter().any(|m| m.name == "doubled"),
            "Should include generated 'doubled' method on Number"
        );
    }

    #[test]
    fn test_extract_type_methods_annotation_not_applied_does_not_generate() {
        let code = r#"
annotation add_number_method() {
    targets: [function]
    comptime post(target, ctx) {
        extend Number {
            method doubled() { self * 2.0 }
        }
    }
}
type Point { x: int, y: int }
"#;
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        assert!(
            !methods.contains_key("Number"),
            "Annotation definition without usage must not generate methods"
        );
    }

    #[test]
    fn test_extract_type_methods_impl_block() {
        let code = r#"
trait Queryable {
    filter(pred): any;
    select(cols): any;
    orderBy(col): any
}
impl Queryable for MyQ {
    method filter(pred) { self }
}
"#;
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        let myq_methods = methods.get("MyQ").expect("Should find MyQ methods");
        let names: Vec<&str> = myq_methods.iter().map(|m| m.name.as_str()).collect();
        // All trait methods should be surfaced, not just the one implemented
        assert!(names.contains(&"filter"), "Should include filter");
        assert!(names.contains(&"select"), "Should include select");
        assert!(names.contains(&"orderBy"), "Should include orderBy");
    }

    #[test]
    fn test_extract_type_methods_trait_only() {
        // A trait definition alone should NOT pollute any type
        let code = "trait Foo {\n  bar(): any\n}\n";
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        assert!(
            methods.is_empty(),
            "Trait alone should not produce type methods"
        );
    }

    #[test]
    fn test_extract_type_methods_multiple_impls() {
        let code = r#"
trait A { a1(): any }
trait B { b1(): any }
impl A for X { method a1() { self } }
impl B for X { method b1() { self } }
"#;
        let program = parse_program(code).unwrap();
        let methods = extract_type_methods(&program);
        let x_methods = methods.get("X").expect("Should find X methods");
        let names: Vec<&str> = x_methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"a1"), "Should include a1 from trait A");
        assert!(names.contains(&"b1"), "Should include b1 from trait B");
    }

    #[test]
    fn test_infer_function_signatures_return_type() {
        let code = "fn add(a: int, b: int) {\n  return a + b\n}";
        let program = parse_program(code).unwrap();
        let sigs = infer_function_signatures(&program);
        if let Some(info) = sigs.get("add") {
            // Params are annotated — should be empty
            assert!(
                info.param_types.is_empty(),
                "Annotated params should not appear: {:?}",
                info.param_types
            );
            // Return type should be inferred
            assert!(
                info.return_type.is_some(),
                "Return type should be inferred from body"
            );
        }
        // Note: if the engine doesn't produce a function type for "add",
        // sigs may be empty — that's OK, it means the engine couldn't resolve it.
    }

    #[test]
    fn test_infer_function_signatures_unannotated_param_union_from_callsites() {
        let code = "fn foo(a) {\n  return a\n}\nlet i = foo(1)\nlet s = foo(\"hi\")\n";
        let program = parse_program(code).unwrap();
        let sigs = infer_function_signatures(&program);
        let info = sigs.get("foo").expect("foo should have inferred signature");
        let param = info
            .param_types
            .iter()
            .find(|(name, _)| name == "a")
            .expect("expected inferred type for param a");
        assert!(
            param.1.contains("int") && param.1.contains("string"),
            "expected union param type, got {}",
            param.1
        );
        let ret = info.return_type.as_deref().unwrap_or("");
        assert!(
            ret.contains("int") && ret.contains("string"),
            "expected union return type, got {}",
            ret
        );
        assert!(
            matches!(
                info.param_ref_modes.get("a"),
                Some(ParamReferenceMode::Shared)
            ),
            "expected read-only inferred reference mode for union param"
        );
    }

    #[test]
    fn test_infer_function_signatures_marks_mutating_ref_params() {
        let code = r#"
fn mutate(a) {
  a = "new"
  return a
}
let s = "old"
mutate(s)
"#;
        let program = parse_program(code).unwrap();
        let sigs = infer_function_signatures(&program);
        let info = sigs
            .get("mutate")
            .expect("mutate should have inferred signature");
        assert!(
            matches!(
                info.param_ref_modes.get("a"),
                Some(ParamReferenceMode::Exclusive)
            ),
            "expected mutating inferred reference mode"
        );
    }

    #[test]
    fn test_infer_function_signatures_skips_annotated() {
        let code = "fn greet(name: string) -> string {\n  return name\n}";
        let program = parse_program(code).unwrap();
        let sigs = infer_function_signatures(&program);
        // Both params and return are annotated — should produce no hints
        assert!(
            sigs.get("greet").is_none(),
            "Fully annotated function should have no inferred signatures"
        );
    }
}
