//! Identity-controlled rename for generated capture descriptors.

use shape_ast::ast::Program;
use tower_lsp_server::ls_types::{TextEdit, Uri, WorkspaceEdit};

use super::navigation::complete_binding_anchors;
use super::presentation::range_from_anchor;
use super::{CaptureAnalysis, GeneratedCaptureLookup, GeneratedQuerySession};

/// Rename the complete real-source graph of a generated capture binding.
///
/// The cursor span only asks the compiler query which descriptor was selected.
/// Every edit is then joined through the opaque binding identity. Missing or
/// conflicting query evidence is terminal and must never fall through to the
/// ordinary name-based rename provider.
pub(crate) fn generated_capture_rename(
    program: &Program,
    text: &str,
    offset: usize,
    uri: &Uri,
    new_name: &str,
    session: &GeneratedQuerySession,
) -> GeneratedCaptureLookup<WorkspaceEdit> {
    let captures = match super::analyze_session(session, program) {
        CaptureAnalysis::NotNeeded => return GeneratedCaptureLookup::NotCapture,
        CaptureAnalysis::Unavailable => return GeneratedCaptureLookup::Unavailable,
        CaptureAnalysis::Ready(captures) => captures,
    };
    let site = match captures.capture_at(0, offset) {
        None if unavailable_evidence_covers(&captures, program, offset) => {
            return GeneratedCaptureLookup::Unavailable;
        }
        None => return GeneratedCaptureLookup::NotCapture,
        Some(shape_vm::compiler::GeneratedCapturePosition::Unavailable) => {
            return GeneratedCaptureLookup::Unavailable;
        }
        Some(shape_vm::compiler::GeneratedCapturePosition::Available(site)) => site,
    };
    let Some(anchors) = complete_binding_anchors(&captures, &site) else {
        return GeneratedCaptureLookup::Unavailable;
    };
    if anchors.is_empty() || anchors.iter().any(|anchor| anchor.file_id() != 0) {
        return GeneratedCaptureLookup::Unavailable;
    }

    let edits = anchors
        .into_iter()
        .map(|anchor| TextEdit {
            range: range_from_anchor(anchor, text),
            new_text: new_name.to_string(),
        })
        .collect();
    let changes = std::collections::HashMap::from([(uri.clone(), edits)]);
    GeneratedCaptureLookup::Found(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// Unmapped generated strings are refusal evidence only. They never identify
/// a capture or authorize an edit, but they must stop the generic text rename
/// provider from treating generated source text as an ordinary identifier.
fn unavailable_evidence_covers(
    query: &shape_vm::compiler::GeneratedCaptureQuery,
    program: &Program,
    offset: usize,
) -> bool {
    let has_unavailable_evidence = query.issues().iter().any(|issue| {
        issue.code() == shape_vm::compiler::GENERATED_CAPTURE_SOURCE_UNAVAILABLE_CODE
            || issue.code() == shape_vm::compiler::GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE
    });
    if !has_unavailable_evidence {
        return false;
    }
    if query.issues().iter().any(|issue| {
        issue
            .anchor()
            .is_some_and(|anchor| anchor.contains(0, offset))
    }) {
        return true;
    }

    struct PlainStringAt {
        offset: usize,
        found: bool,
    }
    impl shape_runtime::visitor::Visitor for PlainStringAt {
        fn visit_expr_literal(
            &mut self,
            expr: &shape_ast::ast::Expr,
            span: shape_ast::ast::Span,
        ) -> bool {
            if matches!(
                expr,
                shape_ast::ast::Expr::Literal(shape_ast::ast::Literal::String(_), _)
            ) && span.start <= self.offset
                && self.offset < span.end
            {
                self.found = true;
            }
            false
        }
    }
    let mut finder = PlainStringAt {
        offset,
        found: false,
    };
    shape_runtime::visitor::walk_program(&mut finder, program);
    finder.found
}
