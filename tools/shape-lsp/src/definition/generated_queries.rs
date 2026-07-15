//! Compiler-backed generated-capture routing for definition/reference requests.

use shape_ast::ast::Program;
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{GotoDefinitionResponse, Location, Position, Range, Uri};

use crate::generated_captures::{
    CaptureQueryContext, GeneratedCaptureLookup, GeneratedQuerySession,
};
use crate::module_cache::ModuleCache;
use crate::util::{get_word_at_position, offset_to_line_col, parser_source, position_to_offset};

pub(super) fn definition(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    uri: &Uri,
    module_cache: Option<&ModuleCache>,
) -> GeneratedCaptureLookup<GotoDefinitionResponse> {
    let current_path = uri.to_file_path().map(|path| path.into_owned());
    let context = CaptureQueryContext {
        file_path: current_path.as_deref(),
        module_cache,
        workspace_root: None,
    };
    let session = GeneratedQuerySession::new(program, text, context);
    match crate::generated_captures::generated_capture_definition(
        program, text, offset, uri, &session,
    ) {
        found @ GeneratedCaptureLookup::Found(_) => return found,
        GeneratedCaptureLookup::Unavailable => return GeneratedCaptureLookup::Unavailable,
        GeneratedCaptureLookup::NotCapture => {}
    }

    match session.compiler().and_then(|compiler| {
        crate::generated_symbols::generated_definition_from_compiler(
            program, text, word, offset, uri, compiler,
        )
    }) {
        Some(response) => GeneratedCaptureLookup::Found(response),
        None => GeneratedCaptureLookup::NotCapture,
    }
}

pub(super) enum CrossFileReferences {
    Found(Vec<Location>),
    Unavailable,
    Continue(Option<GeneratedQuerySession>),
}

pub(super) fn cross_file_references(
    parse_source: &str,
    text: &str,
    position: Position,
    uri: &Uri,
    module_cache: Option<&ModuleCache>,
    workspace_root: Option<&std::path::Path>,
) -> CrossFileReferences {
    let Ok(program) = parse_program(parse_source) else {
        return CrossFileReferences::Continue(None);
    };
    let Some(offset) = position_to_offset(text, position) else {
        return CrossFileReferences::Continue(None);
    };
    let current_path = uri.to_file_path().map(|path| path.into_owned());
    let context = CaptureQueryContext {
        file_path: current_path.as_deref(),
        module_cache,
        workspace_root,
    };
    let session = GeneratedQuerySession::new(&program, text, context);
    match crate::generated_captures::generated_capture_references_with_session(
        &program, text, offset, uri, &session,
    ) {
        GeneratedCaptureLookup::Found(locations) => CrossFileReferences::Found(locations),
        GeneratedCaptureLookup::Unavailable => CrossFileReferences::Unavailable,
        GeneratedCaptureLookup::NotCapture => CrossFileReferences::Continue(Some(session)),
    }
}

pub(super) fn references_with_fallback(
    text: &str,
    position: Position,
    uri: &Uri,
    cached_program: Option<&Program>,
    check_generated_capture: bool,
    shared_generated_session: Option<&GeneratedQuerySession>,
) -> Option<Vec<Location>> {
    let offset = position_to_offset(text, position)?;

    // Generated capture references require an exact current-source AST; a
    // cached or resilient tree can carry stale offsets.
    let parse_src = parser_source(text);
    let (program, has_exact_source_ast) = match parse_program(parse_src.as_ref()) {
        Ok(program) => (program, true),
        Err(_) => {
            if let Some(cached) = cached_program {
                (cached.clone(), false)
            } else {
                let partial = shape_ast::parse_program_resilient(parse_src.as_ref());
                if partial.items.is_empty() {
                    return None;
                }
                (partial.into_program(), false)
            }
        }
    };
    let owned_generated_session = (check_generated_capture
        && has_exact_source_ast
        && shared_generated_session.is_none())
    .then(|| GeneratedQuerySession::new(&program, text, CaptureQueryContext::unavailable()));
    let generated_session = shared_generated_session.or(owned_generated_session.as_ref());
    if check_generated_capture && has_exact_source_ast {
        let session = generated_session.expect("exact generated capture check has a session");
        match crate::generated_captures::generated_capture_references_with_session(
            &program, text, offset, uri, session,
        ) {
            GeneratedCaptureLookup::Found(locations) => return Some(locations),
            GeneratedCaptureLookup::Unavailable => return None,
            GeneratedCaptureLookup::NotCapture => {}
        }
    }

    // Generated declarations are resolved from the compiler's structural
    // query surface before the ordinary scope/text providers run.
    if let Some(word) = get_word_at_position(text, position) {
        let generated_locations =
            if let Some(compiler) = generated_session.and_then(GeneratedQuerySession::compiler) {
                crate::generated_symbols::generated_references_from_compiler(
                    &program, text, &word, offset, uri, compiler,
                )
            } else if !has_exact_source_ast {
                crate::generated_symbols::generated_references(&program, text, &word, offset, uri)
            } else {
                None
            };
        if let Some(locations) = generated_locations {
            return Some(locations);
        }
    }

    let tree = crate::scope::ScopeTree::build(&program, text);
    let spans = tree.references_of(offset)?;
    let locations: Vec<Location> = spans
        .into_iter()
        .map(|(start, end)| {
            let (start_line, start_col) = offset_to_line_col(text, start);
            let (end_line, end_col) = offset_to_line_col(text, end);
            Location {
                uri: uri.clone(),
                range: Range {
                    start: Position {
                        line: start_line,
                        character: start_col,
                    },
                    end: Position {
                        line: end_line,
                        character: end_col,
                    },
                },
            }
        })
        .collect();

    if locations.is_empty() {
        let word = get_word_at_position(text, position)?;
        let fallback = super::find_all_references(&program, &word, uri, text);
        (!fallback.is_empty()).then_some(fallback)
    } else {
        Some(locations)
    }
}
