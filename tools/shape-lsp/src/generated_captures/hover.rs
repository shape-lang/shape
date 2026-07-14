//! Hover rendering for one compiler-issued generated capture occurrence.

use shape_ast::ast::Program;
use shape_vm::compiler::CaptureSiteRole;
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::util::position_to_offset;

pub(super) fn generated_capture_hover(
    program: &Program,
    text: &str,
    position: Position,
) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let captures = super::query(program, text);
    let site = captures.capture_at(0, offset)?;
    let capture = site.capture();
    let role = match site.role() {
        CaptureSiteRole::Declaration => "declaration",
        CaptureSiteRole::Use => "captured use",
    };
    let mode = capture.mode();
    let (declaration, type_detail) = if let Some(capture_type) = capture.uniform_capture_type() {
        (
            format!(
                "{} {}: {}",
                mode.spelling(),
                capture.display_name(),
                capture_type,
            ),
            format!("- Exact static type: `{capture_type}`"),
        )
    } else {
        let mut capture_types: Vec<_> = capture
            .specializations()
            .iter()
            .map(|specialization| specialization.capture_type().clone())
            .collect();
        capture_types.sort_by_key(|capture_type| capture_type.mono_key());
        capture_types.dedup();
        let rendered_types = capture_types
            .iter()
            .map(|capture_type| format!("`{capture_type}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let rendered_specializations = capture
            .specializations()
            .iter()
            .map(|specialization| format!("`{}`", specialization.identity().canonical_descriptor()))
            .collect::<Vec<_>>()
            .join(", ");
        (
            format!("{} {}", mode.spelling(), capture.display_name()),
            format!(
                "- Exact specialization types ({}): {}\n\
                 - Structural specializations: {}",
                capture_types.len(),
                rendered_types,
                rendered_specializations,
            ),
        )
    };
    let value = format!(
        "**Generated Capture** (`{role}`)\n\n\
         ```shape\n{}\n```\n\n\
         - Canonical mode: `{}`\n\
         {}\n\
         - Stage: `generated-only`\n\
         - Owner: `{}` (`{}`)\n\
         - Binding identity: `{}`\n\
         - Capture occurrence: `{}`",
        declaration,
        mode.variant_name(),
        type_detail,
        capture.owner_display(),
        capture.owner_node_path(),
        capture.identity().canonical_descriptor(),
        capture.occurrence_identity().canonical_descriptor(),
    );
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}
