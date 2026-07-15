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
    let range = issue_range(issue.anchor(), text)?;
    Some(Diagnostic {
        range,
        severity: Some(issue_severity(issue.code())),
        code: Some(NumberOrString::String(issue.code().to_string())),
        source: Some("shape-capture-query".to_string()),
        message: issue.message().to_string(),
        ..Default::default()
    })
}

fn issue_severity(code: &str) -> DiagnosticSeverity {
    if code == shape_vm::compiler::GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE {
        DiagnosticSeverity::ERROR
    } else {
        DiagnosticSeverity::INFORMATION
    }
}

fn issue_range(anchor: Option<shape_vm::compiler::SourceAnchor>, text: &str) -> Option<Range> {
    match anchor {
        Some(anchor) if anchor.file_id() == 0 => Some(range_from_anchor(anchor, text)),
        Some(_) => None,
        None => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_conflict_is_an_error() {
        assert_eq!(
            issue_severity(shape_vm::compiler::GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE),
            DiagnosticSeverity::ERROR,
        );
    }

    #[test]
    fn anchorless_conflict_does_not_invent_a_document_range() {
        assert_eq!(issue_range(None, "let value = 1",), None,);
    }

    #[test]
    fn anchorless_source_unavailable_issue_has_no_invented_range() {
        assert_eq!(issue_range(None, "let value = 1",), None,);
    }
}
