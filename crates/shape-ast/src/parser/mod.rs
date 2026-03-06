//! Parser module for Shape language
//!
//! This module contains the complete parser implementation using Pest.
//! It's organized into submodules for different language constructs.

use crate::ast::Span;
use crate::error::{Result, ShapeError, SourceLocation};
use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

/// Extract a lightweight Span from a Pest pair for AST nodes
pub fn pair_span(pair: &Pair<Rule>) -> Span {
    let span = pair.as_span();
    Span::new(span.start(), span.end())
}

/// Extract source location from a Pest pair for error reporting
pub(crate) fn pair_location(pair: &Pair<Rule>) -> SourceLocation {
    let span = pair.as_span();
    let (line, col) = span.start_pos().line_col();
    let source_line = span.start_pos().line_of().to_string();
    let length = span.end() - span.start();

    SourceLocation::new(line, col)
        .with_length(length)
        .with_source_line(source_line)
}

// Submodules for different parsing concerns
pub mod data_sources;
pub mod expressions;
pub mod extensions;
pub mod functions;
pub mod items;
pub mod modules;
pub mod preprocessor;
pub mod queries;
pub mod resilient;
pub mod statements;
pub mod stream;
pub mod string_literals;
pub mod time;
pub mod types;

#[cfg(test)]
mod tests;

use crate::ast::{Item, Program};

#[derive(Parser)]
#[grammar = "src/shape.pest"]
pub struct ShapeParser;

/// Parse a complete Shape program
pub fn parse_program(input: &str) -> Result<Program> {
    let processed = preprocessor::preprocess_semicolons(input);
    let pairs = ShapeParser::parse(Rule::program, &processed).map_err(|e| {
        // Use the structured error converter for rich error messages
        let structured = crate::error::pest_converter::convert_pest_error(&e, &processed);
        ShapeError::StructuredParse(Box::new(structured))
    })?;

    let mut items = Vec::new();

    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::item => {
                        items.push(parse_item(inner)?);
                    }
                    Rule::item_recovery => {
                        let span = inner.as_span();
                        let text = inner.as_str().trim();
                        let preview = if text.len() > 40 {
                            format!("{}...", &text[..40])
                        } else {
                            text.to_string()
                        };
                        return Err(ShapeError::ParseError {
                            message: format!("Syntax error near: {}", preview),
                            location: Some(
                                pair_location(&inner).with_length(span.end() - span.start()),
                            ),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(Program { items })
}

/// Parse an individual item (pattern, query, assignment, or expression)
pub fn parse_item(pair: pest::iterators::Pair<Rule>) -> Result<Item> {
    let pair_loc = pair_location(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected item content".to_string(),
            location: Some(pair_loc.clone().with_hint(
                "provide a pattern, query, function, variable declaration, or expression",
            )),
        })?;

    let span = pair_span(&inner);

    match inner.as_rule() {
        Rule::query => Ok(Item::Query(queries::parse_query(inner)?, span)),
        Rule::variable_decl => Ok(Item::VariableDecl(items::parse_variable_decl(inner)?, span)),
        Rule::assignment => {
            let inner_loc = pair_location(&inner);
            let mut inner = inner.into_inner();
            let pattern_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected pattern in assignment".to_string(),
                location: Some(inner_loc.clone()),
            })?;
            let pattern = items::parse_pattern(pattern_pair)?;
            let value_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected value expression in assignment".to_string(),
                location: Some(inner_loc.with_hint("provide a value after '='")),
            })?;
            let value = expressions::parse_expression(value_pair)?;
            Ok(Item::Assignment(
                crate::ast::Assignment { pattern, value },
                span,
            ))
        }
        Rule::expression_stmt => {
            let inner_loc = pair_location(&inner);
            let expr_pair = inner
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected expression in statement".to_string(),
                    location: Some(inner_loc),
                })?;
            let expr = expressions::parse_expression(expr_pair)?;
            Ok(Item::Expression(expr, span))
        }
        Rule::import_stmt => Ok(Item::Import(modules::parse_import_stmt(inner)?, span)),
        Rule::module_decl => Ok(Item::Module(modules::parse_module_decl(inner)?, span)),
        Rule::pub_item => Ok(Item::Export(modules::parse_export_item(inner)?, span)),
        Rule::struct_type_def => Ok(Item::StructType(types::parse_struct_type_def(inner)?, span)),
        Rule::native_struct_type_def => Ok(Item::StructType(
            types::parse_native_struct_type_def(inner)?,
            span,
        )),
        Rule::builtin_type_decl => Ok(Item::BuiltinTypeDecl(
            types::parse_builtin_type_decl(inner)?,
            span,
        )),
        Rule::type_alias_def => Ok(Item::TypeAlias(types::parse_type_alias_def(inner)?, span)),
        Rule::trait_def => Ok(Item::Trait(types::parse_trait_def(inner)?, span)),
        Rule::enum_def => Ok(Item::Enum(types::parse_enum_def(inner)?, span)),
        Rule::extern_native_function_def => Ok(Item::ForeignFunction(
            functions::parse_extern_native_function_def(inner)?,
            span,
        )),
        Rule::foreign_function_def => Ok(Item::ForeignFunction(
            functions::parse_foreign_function_def(inner)?,
            span,
        )),
        Rule::function_def => Ok(Item::Function(functions::parse_function_def(inner)?, span)),
        Rule::builtin_function_decl => Ok(Item::BuiltinFunctionDecl(
            functions::parse_builtin_function_decl(inner)?,
            span,
        )),
        Rule::stream_def => Ok(Item::Stream(stream::parse_stream_def(inner)?, span)),
        Rule::test_def => Err(ShapeError::ParseError {
            message: "Embedded test definitions are no longer supported in this refactor"
                .to_string(),
            location: None,
        }),
        Rule::statement => Ok(Item::Statement(statements::parse_statement(inner)?, span)),
        Rule::extend_statement => Ok(Item::Extend(
            extensions::parse_extend_statement(inner)?,
            span,
        )),
        Rule::impl_block => Ok(Item::Impl(extensions::parse_impl_block(inner)?, span)),
        Rule::optimize_statement => Ok(Item::Optimize(
            extensions::parse_optimize_statement(inner)?,
            span,
        )),
        Rule::annotation_def => Ok(Item::AnnotationDef(
            extensions::parse_annotation_def(inner)?,
            span,
        )),
        Rule::datasource_def => Ok(Item::DataSource(
            data_sources::parse_datasource_def(inner)?,
            span,
        )),
        Rule::query_decl => Ok(Item::QueryDecl(
            data_sources::parse_query_decl(inner)?,
            span,
        )),
        Rule::comptime_block => {
            let block_pair = inner
                .into_inner()
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected block after 'comptime'".to_string(),
                    location: None,
                })?;
            let block_expr = expressions::control_flow::parse_block_expr(block_pair)?;
            let stmts = expressions::primary::block_items_to_statements(block_expr, span);
            Ok(Item::Comptime(stmts, span))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected item type: {:?}", inner.as_rule()),
            location: Some(pair_location(&inner)),
        }),
    }
}

// Re-export commonly used functions for convenience
pub use expressions::parse_expression;
pub use items::{parse_pattern, parse_variable_decl};
pub use types::parse_type_annotation;

/// Parse a single expression from a string
///
/// This is useful for parsing expressions extracted from string interpolation.
pub fn parse_expression_str(input: &str) -> Result<crate::ast::Expr> {
    let pairs = ShapeParser::parse(Rule::expression, input).map_err(|e| {
        let structured = crate::error::pest_converter::convert_pest_error(&e, input);
        ShapeError::StructuredParse(Box::new(structured))
    })?;

    let pair = pairs
        .into_iter()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "Expected expression".to_string(),
            location: None,
        })?;

    expressions::parse_expression(pair)
}
