//! Shared compiler-session entry points for generated symbol and capture queries.

use shape_ast::ast::{Item, Program, Span};
use tower_lsp_server::ls_types::{GotoDefinitionResponse, Location, Uri};

use super::{
    GeneratedRenameClassification, binder_token_spans_in, call_site_kind_at, call_site_name_spans,
    comment_ranges, generated_decl_kind, generated_definition_for_kind, generated_query_compiler,
    generated_references_for_kind, ordinary_declaration_spans, program_may_generate_symbols,
};

/// Compile with the same imported-item registration used by semantic
/// diagnostics. A document containing imports is explicitly gated when the
/// request lacks module context; capture tooling must not silently execute a
/// different annotation environment from diagnostics.
pub(crate) fn compile_for_generated_capture_queries(
    program: &Program,
    text: &str,
    file_path: Option<&std::path::Path>,
    module_cache: Option<&crate::module_cache::ModuleCache>,
    workspace_root: Option<&std::path::Path>,
) -> Option<shape_vm::BytecodeCompiler> {
    let has_imports = program
        .items
        .iter()
        .any(|item| matches!(item, Item::Import(..)));
    if has_imports && (file_path.is_none() || module_cache.is_none()) {
        return None;
    }

    #[cfg(test)]
    GENERATED_CAPTURE_COMPILE_COUNT.with(|count| count.set(count.get() + 1));

    let mut compiler = generated_query_compiler(text);
    if let (Some(file_path), Some(module_cache)) = (file_path, module_cache) {
        let registration = crate::analysis::validate_imports_and_register_items(
            program,
            text,
            file_path,
            module_cache,
            workspace_root,
            &mut compiler,
        )
        .ok()?;
        if !registration.is_ready() {
            return None;
        }
    }
    if compiler.compile_in_place(program).is_err() && !compiler.generated_queries_available() {
        return None;
    }
    Some(compiler)
}

#[cfg(test)]
std::thread_local! {
    static GENERATED_CAPTURE_COMPILE_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_generated_capture_compile_count() {
    GENERATED_CAPTURE_COMPILE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn generated_capture_compile_count() -> usize {
    GENERATED_CAPTURE_COMPILE_COUNT.with(std::cell::Cell::get)
}

pub(crate) fn generated_definition_from_compiler(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    uri: &Uri,
    compiler: &shape_vm::BytecodeCompiler,
) -> Option<GotoDefinitionResponse> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    generated_definition_for_kind(program, text, word, uri, compiler, cursor_kind)
}

pub(crate) fn generated_references_from_compiler(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    uri: &Uri,
    compiler: &shape_vm::BytecodeCompiler,
) -> Option<Vec<Location>> {
    if !program_may_generate_symbols(program) {
        return None;
    }
    let sites = call_site_name_spans(program, text, word);
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    generated_references_for_kind(program, text, word, uri, compiler, &sites, cursor_kind)
}

pub(crate) fn classify_generated_rename_from_compiler(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
    compiler: &shape_vm::BytecodeCompiler,
) -> Option<GeneratedRenameClassification> {
    let sites = call_site_name_spans(program, text, word);
    let cursor_kind = call_site_kind_at(&sites, offset)?;
    let matches: Vec<_> = compiler
        .generated_symbol_query()
        .symbols_named(word)
        .into_iter()
        .filter(|provenance| generated_decl_kind(provenance.decl_name) == cursor_kind)
        .collect();
    if matches.is_empty() {
        return None;
    }
    // A hand-written declaration of the same callable kind makes bare-name
    // call sites ambiguous, so generated classification must abstain.
    if !ordinary_declaration_spans(program, word, cursor_kind).is_empty() {
        return None;
    }
    let comment_spans = comment_ranges(text);
    let mut binder_spans: Vec<Span> = Vec::new();
    let mut every_match_is_source_bound = true;
    for provenance in &matches {
        let mut spans =
            binder_token_spans_in(text, provenance.generator.span(), word, &comment_spans);
        spans.extend(binder_token_spans_in(
            text,
            provenance.application.span(),
            word,
            &comment_spans,
        ));
        if spans.is_empty() {
            every_match_is_source_bound = false;
        }
        binder_spans.extend(spans);
    }
    let generated_ranges: Vec<Span> = matches
        .iter()
        .map(|provenance| provenance.checked_decl.span())
        .collect();
    if every_match_is_source_bound {
        binder_spans.sort_by_key(|span| span.start);
        binder_spans.dedup();
        // Only kind-compatible call sites belong to the generated symbol.
        let call_site_spans: Vec<Span> = sites
            .iter()
            .filter(|(_, kind)| *kind == cursor_kind)
            .map(|(span, _)| *span)
            .collect();
        Some(GeneratedRenameClassification::SourceBinder {
            binder_spans,
            call_site_spans,
            generated_ranges,
        })
    } else {
        Some(GeneratedRenameClassification::GeneratorControlled {
            decl_names: matches
                .iter()
                .map(|provenance| provenance.decl_name.to_string())
                .collect(),
            generator_span: matches[0].generator.span(),
        })
    }
}
