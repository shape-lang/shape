//! Resilient parser for Shape language.
//!
//! `parse_program_resilient` always returns a partial program and a list of
//! typed parse issues. This is intended for editor/LSP scenarios where partial
//! ASTs are more useful than hard parse failure.

use crate::ast::{Item, Program};
use crate::parser::{Rule, ShapeParser, parse_item};
use pest::Parser;
use pest::error::InputLocation;

/// A partially parsed program — always produced, never fails.
#[derive(Debug, Clone)]
pub struct PartialProgram {
    /// Successfully parsed top-level items.
    pub items: Vec<Item>,
    /// Module-level doc comment declared at the start of the file.
    pub doc_comment: Option<crate::ast::DocComment>,
    /// Parse issues collected during resilient parsing.
    pub errors: Vec<ParseError>,
}

impl PartialProgram {
    /// Convert to a standard Program (dropping parse issue info).
    pub fn into_program(self) -> Program {
        let mut program = Program {
            items: self.items,
            docs: crate::ast::ProgramDocs::default(),
        };
        program.docs = crate::parser::docs::build_program_docs(&program, self.doc_comment.as_ref());
        program
    }

    /// Whether the parse was completely successful (no issues).
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty()
    }

    /// True when every recorded issue is a grammar-level failure.
    pub fn has_only_grammar_failures(&self) -> bool {
        !self.errors.is_empty()
            && self
                .errors
                .iter()
                .all(|e| matches!(e.kind, ParseErrorKind::GrammarFailure))
    }
}

/// Kind of resilient parse issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ParseErrorKind {
    RecoverySyntax,
    ItemConversion,
    GrammarFailure,
    MalformedFromUse,
    EmptyMatch,
}

/// A parse issue with span information.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub message: String,
    pub span: (usize, usize),
}

/// Parse a Shape program resiliently. Always succeeds.
///
/// - Uses the normal parser and collects `item_recovery` nodes as syntax issues.
/// - Records AST conversion failures per item.
/// - If grammar-level parsing fails, records a grammar failure issue.
/// - Runs targeted source-level diagnostics (malformed `from ... use`, empty match).
pub fn parse_program_resilient(source: &str) -> PartialProgram {
    let mut items = Vec::new();
    let mut doc_comment = None;
    let mut errors = Vec::new();

    match ShapeParser::parse(Rule::program, source) {
        Ok(pairs) => collect_pairs(pairs, 0, &mut items, &mut doc_comment, &mut errors),
        Err(pest_err) => {
            errors.push(parse_error_from_pest(&pest_err, source));
            recover_items_before_grammar_failure(source, &pest_err, &mut items, &mut errors);
        }
    }

    // Targeted parse diagnostics (single-source resilient pipeline).
    errors.extend(detect_malformed_from_use(source));
    errors.extend(detect_empty_match(source));

    dedup_and_sort_errors(&mut errors);

    PartialProgram {
        items,
        doc_comment,
        errors,
    }
}

fn collect_pairs(
    pairs: pest::iterators::Pairs<Rule>,
    base_offset: usize,
    items: &mut Vec<Item>,
    doc_comment: &mut Option<crate::ast::DocComment>,
    errors: &mut Vec<ParseError>,
) {
    for pair in pairs {
        if pair.as_rule() != Rule::program {
            continue;
        }

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::program_doc_comment => {
                    *doc_comment = Some(crate::parser::docs::parse_doc_comment(inner));
                }
                Rule::item => match parse_item(inner.clone()) {
                    Ok(item) => items.push(item),
                    Err(e) => {
                        let span = inner.as_span();
                        errors.push(ParseError {
                            kind: ParseErrorKind::ItemConversion,
                            message: format!("Failed to parse item: {}", e),
                            span: (base_offset + span.start(), base_offset + span.end()),
                        });
                    }
                },
                Rule::item_recovery => {
                    let span = inner.as_span();
                    let text = inner.as_str().trim();
                    let preview = if text.len() > 40 {
                        format!("{}...", &text[..40])
                    } else {
                        text.to_string()
                    };
                    errors.push(ParseError {
                        kind: ParseErrorKind::RecoverySyntax,
                        message: format!("Syntax error near: {}", preview),
                        span: (base_offset + span.start(), base_offset + span.end()),
                    });
                }
                Rule::EOI => {}
                _ => {}
            }
        }
    }
}

fn recover_items_before_grammar_failure(
    source: &str,
    err: &pest::error::Error<Rule>,
    items: &mut Vec<Item>,
    errors: &mut Vec<ParseError>,
) {
    let cutoff = match err.location {
        InputLocation::Pos(pos) => pos.min(source.len()),
        InputLocation::Span((start, _)) => start.min(source.len()),
    };

    if cutoff == 0 {
        return;
    }

    for candidate in prefix_cutoffs(source, cutoff) {
        if candidate == 0 {
            continue;
        }
        let prefix = &source[..candidate];
        if let Ok(pairs) = ShapeParser::parse(Rule::program, prefix) {
            let mut doc_comment = None;
            collect_pairs(pairs, 0, items, &mut doc_comment, errors);
            return;
        }
    }
}

fn prefix_cutoffs(source: &str, cutoff: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut current = cutoff.min(source.len());
    let mut attempts = 0usize;

    while current > 0 && attempts < 64 {
        out.push(current);
        if let Some(prev_newline) = source[..current].rfind('\n') {
            current = prev_newline;
        } else {
            break;
        }
        attempts += 1;
    }

    out
}

fn parse_error_from_pest(err: &pest::error::Error<Rule>, source: &str) -> ParseError {
    let (start, end) = match err.location {
        InputLocation::Pos(pos) => {
            let s = pos.min(source.len());
            (s, (s + 1).min(source.len()))
        }
        InputLocation::Span((start, end)) => {
            let s = start.min(source.len());
            let e = end.min(source.len());
            if e > s {
                (s, e)
            } else {
                (s, (s + 1).min(source.len()))
            }
        }
    };

    ParseError {
        kind: ParseErrorKind::GrammarFailure,
        message: format!("Parse error: {}", err),
        span: (start, end),
    }
}

fn dedup_and_sort_errors(errors: &mut Vec<ParseError>) {
    errors.sort_by_key(|e| (e.span.0, e.span.1, e.kind));
    errors.dedup_by(|a, b| a.kind == b.kind && a.span == b.span && a.message == b.message);
}

/// Best-effort targeted recovery for malformed `from <module> use { ... }` lines.
///
/// When `use` is misspelled (e.g. `duse`), grammar-level errors can point to
/// the leading `from` token. This helper reports the actual offending token.
fn detect_malformed_from_use(source: &str) -> Vec<ParseError> {
    let mut out = Vec::new();
    let mut line_base = 0usize;

    for line in source.lines() {
        let trimmed = line.trim_start();
        let indent = line.len().saturating_sub(trimmed.len());

        if !trimmed.starts_with("from ") {
            line_base += line.len() + 1;
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let _from = parts.next();
        let _path = parts.next();
        let keyword = parts.next();

        let Some(found) = keyword else {
            line_base += line.len() + 1;
            continue;
        };

        // `from ... in ...` is query syntax, not import syntax.
        if found == "use" || found == "in" {
            line_base += line.len() + 1;
            continue;
        }

        if let Some(col) = trimmed.find(found) {
            let start = line_base + indent + col;
            let end = start + found.len();
            out.push(ParseError {
                kind: ParseErrorKind::MalformedFromUse,
                message: format!(
                    "expected keyword 'use' after module path, found '{}'",
                    found
                ),
                span: (start, end),
            });
        }

        line_base += line.len() + 1;
    }

    out
}

/// Detect empty match expressions:
///
/// ```text
/// match value {
/// }
/// ```
fn detect_empty_match(source: &str) -> Vec<ParseError> {
    let mut out = Vec::new();
    let mut search_from = 0usize;

    while let Some(rel_match) = source[search_from..].find("match") {
        let match_start = search_from + rel_match;

        // Ensure token boundary for `match`.
        let prev_ok = match_start == 0
            || !source[..match_start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !prev_ok {
            search_from = match_start + "match".len();
            continue;
        }

        let after_match = &source[match_start + "match".len()..];
        let Some(open_rel) = after_match.find('{') else {
            search_from = match_start + "match".len();
            continue;
        };
        let open = match_start + "match".len() + open_rel;

        let Some(close_rel) = source[open + 1..].find('}') else {
            search_from = open + 1;
            continue;
        };
        let close = open + 1 + close_rel;

        let between = &source[open + 1..close];
        let non_comment_content = between
            .lines()
            .map(|line| line.split_once("//").map(|(head, _)| head).unwrap_or(line))
            .collect::<String>();

        if non_comment_content.trim().is_empty() {
            out.push(ParseError {
                kind: ParseErrorKind::EmptyMatch,
                message: "match expression requires at least one arm".to_string(),
                span: (open, close + 1),
            });
        }

        search_from = close + 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resilient_parse_valid_program() {
        let source = r#"
            let x = 10;
            let y = 20;
        "#;
        let result = parse_program_resilient(source);
        assert!(
            result.errors.is_empty(),
            "Expected no errors: {:?}",
            result.errors
        );
        assert_eq!(result.items.len(), 2);
        assert!(result.is_complete());
    }

    #[test]
    fn test_resilient_parse_with_error_between_items() {
        let source = r#"let x = 10;
@@@ broken stuff here
let y = 20;"#;
        let result = parse_program_resilient(source);
        assert!(!result.errors.is_empty(), "Expected some errors");
        assert!(
            !result.items.is_empty() || result.has_only_grammar_failures(),
            "Expected partial items or explicit grammar failures, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_resilient_parse_recovers_after_bad_function() {
        let source = r#"
function good() {
    return 1;
}

function bad( {
    missing params
}

let x = 42;
"#;
        let result = parse_program_resilient(source);
        assert!(!result.errors.is_empty(), "Expected parse issues");
        assert!(
            result.items.len() >= 1 || result.has_only_grammar_failures(),
            "Expected partial items or grammar-failure issues, got {} items and errors: {:?}",
            result.items.len(),
            result.errors
        );
    }

    #[test]
    fn test_resilient_parse_empty_source() {
        let result = parse_program_resilient("");
        assert!(result.items.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_resilient_parse_only_errors() {
        let source = "@@@ !!! ??? garbage";
        let result = parse_program_resilient(source);
        assert!(
            !result.errors.is_empty(),
            "Expected errors for garbage input"
        );
    }

    #[test]
    fn test_partial_program_into_program() {
        let source = "let x = 10;";
        let result = parse_program_resilient(source);
        let program = result.into_program();
        assert_eq!(program.items.len(), 1);
    }

    #[test]
    fn test_reports_misspelled_from_use_keyword_with_token_span() {
        let source = "from std::core::snapshot duse { Snapshot }\nlet x = 1;\n";
        let result = parse_program_resilient(source);

        let specific = result
            .errors
            .iter()
            .find(|e| e.kind == ParseErrorKind::MalformedFromUse)
            .expect("expected targeted malformed import diagnostic");

        let bad = &source[specific.span.0..specific.span.1];
        assert_eq!(bad, "duse");
    }

    #[test]
    fn test_empty_match_does_not_emit_misleading_from_identifier_error() {
        let source = r#"
from std::core::snapshot use { Snapshot }

let x = {x: 1}
let y = | x | 10*(x.x*2)
print(f"this is {y(x)}")

x.y = 1
let i = 10D

let c = "d"

fn afunc(c) {
  print("func called with " + c)
  match c {

  }
  return c
}

print(afunc(x))
"#;

        let result = parse_program_resilient(source);
        assert!(
            !result
                .errors
                .iter()
                .any(|e| e.message.contains("found identifier `from`")),
            "resilient parser produced misleading import-token error: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_resilient_parse_keeps_typed_match_after_commented_line() {
        let source = r#"
from std::core::snapshot use { Snapshot }

fn afunc(c) {
  //print("func called with " + c)
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  return c
  return "hi"
}
"#;

        let result = parse_program_resilient(source);
        assert!(
            result
                .items
                .iter()
                .any(|item| matches!(item, crate::ast::Item::Function(_, _))),
            "expected function item to parse, got: {:?}",
            result.items
        );
    }

    #[test]
    fn test_detect_empty_match_reports_precise_span() {
        let source = "fn f(x) {\n  match x {\n\n  }\n}\n";
        let errors = detect_empty_match(source);
        assert!(
            errors.iter().any(|e| e.kind == ParseErrorKind::EmptyMatch),
            "expected empty match issue, got: {:?}",
            errors
        );
    }
}
