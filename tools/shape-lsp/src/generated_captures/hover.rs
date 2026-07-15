//! Hover rendering for a set of compiler-issued generated capture occurrences.

use std::collections::BTreeSet;
use std::path::Path;

use shape_ast::ast::Program;
use shape_ast::parser::parse_program;
use shape_vm::compiler::{CaptureSiteRole, GeneratedCapturePosition};
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use super::{CaptureAnalysis, CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession};
use crate::module_cache::ModuleCache;
use crate::util::{parser_source, position_to_offset};

/// Route hover through one fresh exact parse and one compiler query session.
/// Parse failure is `NotCapture`; unavailable compiler evidence remains a
/// terminal quarantine and must not fall through to name-based hover.
pub(super) fn generated_capture_hover_from_source(
    text: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> GeneratedCaptureLookup<Hover> {
    let parse_src = parser_source(text);
    let Ok(program) = parse_program(parse_src.as_ref()) else {
        return GeneratedCaptureLookup::NotCapture;
    };
    let context = CaptureQueryContext {
        file_path: current_file,
        module_cache,
        workspace_root: None,
    };
    let session = GeneratedQuerySession::new(&program, text, context);
    generated_capture_hover(&program, text, position, &session)
}

pub(super) fn generated_capture_hover(
    program: &Program,
    text: &str,
    position: Position,
    session: &GeneratedQuerySession,
) -> GeneratedCaptureLookup<Hover> {
    let Some(offset) = position_to_offset(text, position) else {
        return GeneratedCaptureLookup::NotCapture;
    };
    let captures = match super::analyze_session(session, program) {
        CaptureAnalysis::NotNeeded => return GeneratedCaptureLookup::NotCapture,
        CaptureAnalysis::Unavailable => return GeneratedCaptureLookup::Unavailable,
        CaptureAnalysis::Ready(captures) => captures,
    };
    let site = match captures.capture_at(0, offset) {
        None => return GeneratedCaptureLookup::NotCapture,
        Some(GeneratedCapturePosition::Unavailable) => {
            return GeneratedCaptureLookup::Unavailable;
        }
        Some(GeneratedCapturePosition::Available(site)) => site,
    };
    if site.role() == CaptureSiteRole::Binding {
        return GeneratedCaptureLookup::NotCapture;
    }

    let role = match site.role() {
        CaptureSiteRole::Declaration => "declaration",
        CaptureSiteRole::Use => "captured use",
        CaptureSiteRole::Binding => unreachable!("binding sites use ordinary hover"),
    };
    let capture_set = site.captures();
    let names = strings(capture_set.iter().map(|capture| capture.display_name()));
    let modes = strings(
        capture_set
            .iter()
            .map(|capture| capture.mode().variant_name()),
    );
    let capture_types = strings(capture_set.iter().flat_map(|capture| {
        capture
            .specializations()
            .iter()
            .map(|specialization| specialization.capture_type().to_string())
    }));
    let specializations = strings(capture_set.iter().flat_map(|capture| {
        capture
            .specializations()
            .iter()
            .map(|specialization| specialization.identity().canonical_descriptor())
    }));
    let owners = strings(capture_set.iter().map(|capture| {
        format!(
            "{} (`{}`)",
            capture.owner_display(),
            capture.owner_node_path()
        )
    }));
    let identities = strings(
        capture_set
            .iter()
            .map(|capture| capture.identity().canonical_descriptor()),
    );
    let occurrences = strings(
        capture_set
            .iter()
            .map(|capture| capture.occurrence_identity().canonical_descriptor()),
    );
    let applications = strings(capture_set.iter().filter_map(|capture| {
        capture.application().map(|anchor| {
            format!(
                "file:{}:{}..{}",
                anchor.file_id(),
                anchor.span().start,
                anchor.span().end,
            )
        })
    }));

    let declaration = if names.len() == 1 && modes.len() == 1 && capture_types.len() == 1 {
        format!(
            "{} {}: {}",
            capture_set[0].mode().spelling(),
            names[0],
            capture_types[0],
        )
    } else {
        format!("capture {}", names.join(" | "))
    };
    let type_detail = if capture_types.len() == 1 {
        format!("- Exact static type: `{}`", capture_types[0])
    } else {
        format!(
            "- Exact specialization types ({}): {}\n\
             - Structural specializations: {}",
            capture_types.len(),
            backticked(&capture_types),
            backticked(&specializations),
        )
    };
    let value = format!(
        "**Generated Capture** (`{role}`)\n\n\
         ```shape\n{declaration}\n```\n\n\
         - Canonical mode{}: {}\n\
         {type_detail}\n\
         - Stage: `generated-only`\n\
         - Application{}: {}\n\
         - Owner{}: {}\n\
         - Binding identit{}: {}\n\
         - Capture occurrence{}: {}",
        plural_suffix(&modes),
        backticked(&modes),
        plural_suffix(&applications),
        backticked(&applications),
        plural_suffix(&owners),
        owners.join(", "),
        if identities.len() == 1 { "y" } else { "ies" },
        backticked(&identities),
        plural_suffix(&occurrences),
        backticked(&occurrences),
    );
    GeneratedCaptureLookup::Found(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

fn strings<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let values: BTreeSet<String> = values.into_iter().map(Into::into).collect();
    values.into_iter().collect()
}

fn backticked(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn plural_suffix(values: &[String]) -> &'static str {
    if values.len() == 1 { "" } else { "s" }
}
