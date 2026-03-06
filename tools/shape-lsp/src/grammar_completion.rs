//! Grammar-driven completion provider
//!
//! Uses Pest's parser error reporting to determine valid next tokens
//! based on the grammar definition. This ensures the LSP is always
//! in sync with the actual language syntax.

use pest::{Parser, error::ErrorVariant};
use shape_ast::parser::{Rule, ShapeParser};
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind, InsertTextFormat};

/// Get completions based on parser expectations at the end of the input
pub fn get_grammar_completions(source: &str) -> Vec<CompletionItem> {
    // Attempt to parse the source up to the cursor
    // Since the source is likely incomplete/truncated at cursor,
    // Pest will return an error indicating what it expected next.
    match ShapeParser::parse(Rule::program, source) {
        Ok(_) => {
            // If it parses successfully (unlikely for partial input),
            // it means we are at a valid end of program.
            // We can suggest top-level items.
            get_top_level_completions()
        }
        Err(e) => {
            match e.variant {
                ErrorVariant::ParsingError { positives, .. } => {
                    // 'positives' contains the Rules that the parser expected
                    positives.iter().flat_map(map_rule_to_completions).collect()
                }
                _ => vec![], // Custom errors don't give us grammar hints
            }
        }
    }
}

/// Map a Pest Rule to a list of valid completion items
fn map_rule_to_completions(rule: &Rule) -> Vec<CompletionItem> {
    match rule {
        // --- Keywords & Structure ---
        Rule::item => get_top_level_completions(),
        Rule::statement => get_statement_completions(),
        Rule::expression => get_expression_completions(),

        // --- Type System ---
        Rule::type_alias_def => vec![keyword("type", "Define a type alias")],
        Rule::type_annotation => vec![
            // Basic types
            type_item("Number", "Numeric value"),
            type_item("String", "Text value"),
            type_item("Bool", "Boolean value"),
            type_item("Integer", "Integer value"),
            type_item("Table", "Typed table container: Table<T>"),
            type_item("Array", "Ordered list: Array<T>"),
            type_item("Object", "Key-value collection"),
            type_item("Option", "Optional value: Option<T>"),
            type_item("Result", "Result type: Result<T> or Result<T, E>"),
        ],

        // --- Specific Keywords ---
        Rule::keyword => vec![
            keyword("let", "Declare a variable"),
            keyword("const", "Declare a constant"),
            keyword("fn", "Define a function"),
            keyword("if", "Conditional statement"),
            keyword("match", "Pattern match expression"),
            keyword("return", "Return value"),
        ],

        // Default: no suggestion
        _ => vec![],
    }
}

// --- Helpers ---

fn get_top_level_completions() -> Vec<CompletionItem> {
    vec![
        keyword("from", "Import from a module"),
        keyword("use", "Import module exports"),
        keyword("pub", "Make a definition publicly visible"),
        keyword("type", "Define a type"),
        keyword("enum", "Define an enum"),
        keyword("trait", "Define a trait"),
        keyword("impl", "Implement a trait for a type"),
        keyword("fn", "Define a function"),
        keyword("async", "Define an async function"),
        keyword("const", "Define a constant"),
        keyword("let", "Declare a variable"),
    ]
}

fn get_statement_completions() -> Vec<CompletionItem> {
    vec![
        keyword("let", "Declare variable"),
        keyword("const", "Declare constant"),
        keyword("if", "Conditional"),
        keyword("match", "Pattern match"),
        keyword("for", "Loop"),
        keyword("while", "Loop"),
        keyword("return", "Return"),
        keyword("await", "Await async value"),
    ]
}

fn get_expression_completions() -> Vec<CompletionItem> {
    vec![
        keyword("true", "Boolean true"),
        keyword("false", "Boolean false"),
        keyword("None", "Option none value"),
        snippet("Some", "Option constructor", "Some(${1:value})"),
    ]
}

fn keyword(label: &str, doc: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        detail: Some(doc.to_string()),
        ..Default::default()
    }
}

fn type_item(label: &str, doc: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::STRUCT),
        detail: Some(doc.to_string()),
        ..Default::default()
    }
}

fn snippet(label: &str, doc: &str, snippet: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(doc.to_string()),
        insert_text: Some(snippet.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}
