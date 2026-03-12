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
pub mod docs;
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

use crate::ast::{DocComment, ExportItem, Item, Program};

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
    let mut module_doc_comment = None;

    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::program_doc_comment => {
                        module_doc_comment = Some(docs::parse_doc_comment(inner));
                    }
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

    let mut program = Program {
        items,
        docs: crate::ast::ProgramDocs::default(),
    };
    program.docs = docs::build_program_docs(&program, module_doc_comment.as_ref());
    Ok(program)
}

/// Parse an individual item (pattern, query, assignment, or expression)
pub fn parse_item(pair: pest::iterators::Pair<Rule>) -> Result<Item> {
    let pair_loc = pair_location(&pair);
    let mut item_inner = pair.into_inner();
    let mut doc_comment = None;
    let mut inner =
        item_inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected item content".to_string(),
            location: Some(pair_loc.clone().with_hint(
                "provide a pattern, query, function, variable declaration, or expression",
            )),
        })?;

    if inner.as_rule() == Rule::doc_comment {
        doc_comment = Some(docs::parse_doc_comment(inner));
        inner = item_inner.next().ok_or_else(|| ShapeError::ParseError {
            message: "expected item after doc comment".to_string(),
            location: Some(pair_loc.clone()),
        })?;
    }

    if inner.as_rule() == Rule::item_core {
        inner = inner
            .into_inner()
            .next()
            .ok_or_else(|| ShapeError::ParseError {
                message: "expected item content".to_string(),
                location: Some(pair_loc.clone()),
            })?;
    }

    let span = pair_span(&inner);
    let mut item = match inner.as_rule() {
        Rule::query => Item::Query(queries::parse_query(inner)?, span),
        Rule::variable_decl => Item::VariableDecl(items::parse_variable_decl(inner)?, span),
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
            Item::Assignment(crate::ast::Assignment { pattern, value }, span)
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
            Item::Expression(expr, span)
        }
        Rule::import_stmt => Item::Import(modules::parse_import_stmt(inner)?, span),
        Rule::module_decl => Item::Module(modules::parse_module_decl(inner)?, span),
        Rule::pub_item => Item::Export(modules::parse_export_item(inner)?, span),
        Rule::struct_type_def => Item::StructType(types::parse_struct_type_def(inner)?, span),
        Rule::native_struct_type_def => {
            Item::StructType(types::parse_native_struct_type_def(inner)?, span)
        }
        Rule::builtin_type_decl => {
            Item::BuiltinTypeDecl(types::parse_builtin_type_decl(inner)?, span)
        }
        Rule::type_alias_def => Item::TypeAlias(types::parse_type_alias_def(inner)?, span),
        Rule::interface_def => Item::Interface(types::parse_interface_def(inner)?, span),
        Rule::trait_def => Item::Trait(types::parse_trait_def(inner)?, span),
        Rule::enum_def => Item::Enum(types::parse_enum_def(inner)?, span),
        Rule::extern_native_function_def => {
            Item::ForeignFunction(functions::parse_extern_native_function_def(inner)?, span)
        }
        Rule::foreign_function_def => {
            Item::ForeignFunction(functions::parse_foreign_function_def(inner)?, span)
        }
        Rule::function_def => Item::Function(functions::parse_function_def(inner)?, span),
        Rule::builtin_function_decl => {
            Item::BuiltinFunctionDecl(functions::parse_builtin_function_decl(inner)?, span)
        }
        Rule::stream_def => Item::Stream(stream::parse_stream_def(inner)?, span),
        Rule::test_def => {
            return Err(ShapeError::ParseError {
                message: "Embedded test definitions are no longer supported in this refactor"
                    .to_string(),
                location: None,
            });
        }
        Rule::statement => Item::Statement(statements::parse_statement(inner)?, span),
        Rule::extend_statement => Item::Extend(extensions::parse_extend_statement(inner)?, span),
        Rule::impl_block => Item::Impl(extensions::parse_impl_block(inner)?, span),
        Rule::optimize_statement => {
            Item::Optimize(extensions::parse_optimize_statement(inner)?, span)
        }
        Rule::annotation_def => Item::AnnotationDef(extensions::parse_annotation_def(inner)?, span),
        Rule::datasource_def => Item::DataSource(data_sources::parse_datasource_def(inner)?, span),
        Rule::query_decl => Item::QueryDecl(data_sources::parse_query_decl(inner)?, span),
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
            Item::Comptime(stmts, span)
        }
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("unexpected item type: {:?}", inner.as_rule()),
                location: Some(pair_location(&inner)),
            });
        }
    };

    if let Some(doc_comment) = doc_comment {
        attach_item_doc_comment(&mut item, doc_comment);
    }

    Ok(item)
}

fn attach_item_doc_comment(item: &mut Item, doc_comment: DocComment) {
    match item {
        Item::Module(module, _) => module.doc_comment = Some(doc_comment),
        Item::TypeAlias(alias, _) => alias.doc_comment = Some(doc_comment),
        Item::Interface(interface, _) => interface.doc_comment = Some(doc_comment),
        Item::Trait(trait_def, _) => trait_def.doc_comment = Some(doc_comment),
        Item::Enum(enum_def, _) => enum_def.doc_comment = Some(doc_comment),
        Item::Function(function, _) => function.doc_comment = Some(doc_comment),
        Item::AnnotationDef(annotation_def, _) => annotation_def.doc_comment = Some(doc_comment),
        Item::StructType(struct_def, _) => struct_def.doc_comment = Some(doc_comment),
        Item::BuiltinTypeDecl(ty, _) => ty.doc_comment = Some(doc_comment),
        Item::BuiltinFunctionDecl(function, _) => function.doc_comment = Some(doc_comment),
        Item::ForeignFunction(function, _) => function.doc_comment = Some(doc_comment),
        Item::Export(export, _) => attach_export_doc_comment(&mut export.item, doc_comment),
        _ => {}
    }
}

fn attach_export_doc_comment(item: &mut ExportItem, doc_comment: DocComment) {
    match item {
        ExportItem::Function(function) => function.doc_comment = Some(doc_comment),
        ExportItem::BuiltinFunction(function) => function.doc_comment = Some(doc_comment),
        ExportItem::BuiltinType(ty) => ty.doc_comment = Some(doc_comment),
        ExportItem::TypeAlias(alias) => alias.doc_comment = Some(doc_comment),
        ExportItem::Enum(enum_def) => enum_def.doc_comment = Some(doc_comment),
        ExportItem::Struct(struct_def) => struct_def.doc_comment = Some(doc_comment),
        ExportItem::Interface(interface) => interface.doc_comment = Some(doc_comment),
        ExportItem::Trait(trait_def) => trait_def.doc_comment = Some(doc_comment),
        ExportItem::Annotation(annotation_def) => annotation_def.doc_comment = Some(doc_comment),
        ExportItem::ForeignFunction(function) => function.doc_comment = Some(doc_comment),
        ExportItem::Named(_) => {}
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
