//! Generated capture/symbol rename routing from one compiler query session.

use std::collections::HashMap;
use std::path::Path;

use shape_ast::ast::Program;
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{Location, Position, Range, TextEdit, Uri, WorkspaceEdit};

use super::{is_builtin_function, is_keyword, is_valid_identifier};
use crate::generated_captures::{
    CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession, generated_capture_rename,
};
use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, offset_to_line_col, parser_source, position_to_offset};

/// Report emitted when a generated name has no editable source binder.
pub const GENERATOR_CONTROLLED_NAME_RENAME_REPORT: &str = "this generated name is generator-controlled: it is computed by its \
     generator and is never renamed by text edit; change the generator \
     definition instead (ADR-009 Decision 68)";

/// Generator-controlled refusal with the real generator location.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorControlledRename {
    pub message: String,
    pub generator: Location,
}

/// Identity-driven generated-symbol rename result.
#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedRename {
    /// Source-binder edits that recompute the generated declaration.
    Edits(WorkspaceEdit),
    /// Refusal for a wholly generator-controlled name.
    GeneratorControlled(GeneratorControlledRename),
}

/// Terminal generated-query result for one LSP rename request.
pub(crate) enum GeneratedRenameRequest {
    Edits(WorkspaceEdit),
    GeneratorControlled(GeneratorControlledRename),
    Unavailable,
    NotGenerated,
}

/// Run generated capture and generated symbol rename through one compiler.
#[allow(clippy::too_many_arguments)]
pub(crate) fn generated_rename_request(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
    file_path: Option<&Path>,
    module_cache: &ModuleCache,
    workspace_root: Option<&Path>,
) -> GeneratedRenameRequest {
    if !is_valid_identifier(new_name) || is_keyword(new_name) {
        return GeneratedRenameRequest::NotGenerated;
    }
    let parse_src = parser_source(text);
    let Ok(program) = parse_program(parse_src.as_ref()) else {
        return generated_rename(text, uri, position, new_name, cached_program)
            .map_or(GeneratedRenameRequest::NotGenerated, request_result);
    };
    let Some(offset) = position_to_offset(text, position) else {
        return GeneratedRenameRequest::NotGenerated;
    };
    let session = GeneratedQuerySession::new(
        &program,
        text,
        CaptureQueryContext {
            file_path,
            module_cache: Some(module_cache),
            workspace_root,
        },
    );
    match generated_capture_rename(&program, text, offset, uri, new_name, &session) {
        GeneratedCaptureLookup::Found(edit) => return GeneratedRenameRequest::Edits(edit),
        GeneratedCaptureLookup::Unavailable => return GeneratedRenameRequest::Unavailable,
        GeneratedCaptureLookup::NotCapture => {}
    }
    session
        .compiler()
        .and_then(|compiler| {
            generated_rename_from_compiler(text, uri, position, new_name, &program, compiler)
        })
        .map_or(GeneratedRenameRequest::NotGenerated, request_result)
}

fn request_result(result: GeneratedRename) -> GeneratedRenameRequest {
    match result {
        GeneratedRename::Edits(edit) => GeneratedRenameRequest::Edits(edit),
        GeneratedRename::GeneratorControlled(report) => {
            GeneratedRenameRequest::GeneratorControlled(report)
        }
    }
}

/// Classify generated-symbol rename, compiling a query when no request
/// compiler is available.
pub fn generated_rename(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    cached_program: Option<&Program>,
) -> Option<GeneratedRename> {
    if !is_valid_identifier(new_name) || is_keyword(new_name) {
        return None;
    }
    let old_name = get_word_at_position(text, position)?;
    if is_keyword(&old_name) || is_builtin_function(&old_name) {
        return None;
    }
    let offset = position_to_offset(text, position)?;
    let program = parse_program(text)
        .ok()
        .or_else(|| cached_program.cloned())?;
    let classification =
        crate::generated_symbols::classify_generated_rename(&program, text, &old_name, offset)?;
    build_generated_rename(text, uri, new_name, classification)
}

pub(crate) fn generated_rename_from_compiler(
    text: &str,
    uri: &Uri,
    position: Position,
    new_name: &str,
    program: &Program,
    compiler: &shape_vm::BytecodeCompiler,
) -> Option<GeneratedRename> {
    if !is_valid_identifier(new_name) || is_keyword(new_name) {
        return None;
    }
    let old_name = get_word_at_position(text, position)?;
    if is_keyword(&old_name) || is_builtin_function(&old_name) {
        return None;
    }
    let offset = position_to_offset(text, position)?;
    let classification = crate::generated_symbols::classify_generated_rename_from_compiler(
        program, text, &old_name, offset, compiler,
    )?;
    build_generated_rename(text, uri, new_name, classification)
}

fn build_generated_rename(
    text: &str,
    uri: &Uri,
    new_name: &str,
    classification: crate::generated_symbols::GeneratedRenameClassification,
) -> Option<GeneratedRename> {
    match classification {
        crate::generated_symbols::GeneratedRenameClassification::SourceBinder {
            binder_spans,
            call_site_spans,
            generated_ranges,
        } => {
            let mut spans = call_site_spans;
            spans.retain(|span| {
                !generated_ranges
                    .iter()
                    .any(|generated| span.start < generated.end && generated.start < span.end)
            });
            spans.extend(binder_spans);
            spans.sort_by_key(|span| span.start);
            spans.dedup();
            if spans.is_empty() {
                return None;
            }
            let edits = spans
                .into_iter()
                .map(|span| TextEdit {
                    range: range(text, span.start, span.end),
                    new_text: new_name.to_string(),
                })
                .collect();
            Some(GeneratedRename::Edits(WorkspaceEdit {
                changes: Some(HashMap::from([(uri.clone(), edits)])),
                document_changes: None,
                change_annotations: None,
            }))
        }
        crate::generated_symbols::GeneratedRenameClassification::GeneratorControlled {
            decl_names,
            generator_span,
        } => {
            let generator = Location {
                uri: uri.clone(),
                range: range(text, generator_span.start, generator_span.end),
            };
            let line = generator.range.start.line + 1;
            let message = format!(
                "{GENERATOR_CONTROLLED_NAME_RENAME_REPORT}: `{}` is named by the generator defined at line {line}",
                decl_names.join("`, `"),
            );
            Some(GeneratedRename::GeneratorControlled(
                GeneratorControlledRename { message, generator },
            ))
        }
    }
}

fn range(text: &str, start: usize, end: usize) -> Range {
    let (start_line, start_col) = offset_to_line_col(text, start);
    let (end_line, end_col) = offset_to_line_col(text, end);
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
