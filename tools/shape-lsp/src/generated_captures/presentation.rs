//! Source-position and diagnostic presentation for generated capture queries.

use shape_ast::ast::Program;
use shape_vm::compiler::GeneratedCaptureQueryIssue;
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticSeverity, Location, NumberOrString, Position, Range, Uri,
};

use crate::util::offset_to_line_col;

pub(super) fn capture_query_diagnostics(
    compiler: &shape_vm::BytecodeCompiler,
    program: &Program,
    text: &str,
) -> Vec<Diagnostic> {
    compiler
        .generated_capture_query(program)
        .issues()
        .iter()
        .filter_map(|issue| issue_to_diagnostic(issue, text))
        .collect()
}

fn issue_to_diagnostic(issue: &GeneratedCaptureQueryIssue, text: &str) -> Option<Diagnostic> {
    let anchor = issue.application()?;
    if anchor.file_id() != 0 {
        return None;
    }
    Some(Diagnostic {
        range: range_from_anchor(anchor, text),
        severity: Some(DiagnosticSeverity::INFORMATION),
        code: Some(NumberOrString::String(issue.code().to_string())),
        source: Some("shape-capture-query".to_string()),
        message: issue.message().to_string(),
        ..Default::default()
    })
}

pub(super) fn push_anchor(
    locations: &mut Vec<Location>,
    anchor: shape_vm::compiler::SourceAnchor,
    text: &str,
    uri: &Uri,
) {
    if anchor.file_id() != 0 {
        return;
    }
    let location = Location {
        uri: uri.clone(),
        range: range_from_anchor(anchor, text),
    };
    if !locations.contains(&location) {
        locations.push(location);
    }
}

fn range_from_anchor(anchor: shape_vm::compiler::SourceAnchor, text: &str) -> Range {
    let span = anchor.span();
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}
