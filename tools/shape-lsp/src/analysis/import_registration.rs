//! Result-bearing import setup for semantic and generated-query compilers.

use shape_ast::ast::{ImportItems, Item, Program};
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticSeverity, Position, Range,
};

use crate::module_cache::ModuleCache;
use crate::util::offset_to_line_col;

#[derive(Debug)]
pub struct ImportRegistrationOutcome {
    diagnostics: Vec<Diagnostic>,
    ready: bool,
}

impl ImportRegistrationOutcome {
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Validate every import and register named-import dependency surfaces.
///
/// Resolution diagnostics are accumulated as ordinary user diagnostics and
/// mark the compiler unavailable. A VM registration failure is returned at the
/// importing statement's span; callers must discard the compiler and publish
/// neither generated symbols nor generated captures.
pub fn validate_imports_and_register_items(
    program: &Program,
    text: &str,
    file_path: &std::path::Path,
    module_cache: &ModuleCache,
    workspace_root: Option<&std::path::Path>,
    compiler: &mut shape_vm::BytecodeCompiler,
) -> Result<ImportRegistrationOutcome, Diagnostic> {
    let mut diagnostics = Vec::new();
    let importable_modules = module_cache.list_importable_modules_with_context_and_source(
        file_path,
        workspace_root,
        Some(text),
    );
    let mut known_module_names = crate::completion::imports::module_names_with_context_and_source(
        Some(file_path),
        workspace_root,
        Some(text),
    );
    known_module_names.extend(importable_modules.iter().filter_map(|module_path| {
        module_path
            .split('.')
            .next()
            .map(|segment| segment.to_string())
    }));

    for item in &program.items {
        let Item::Import(import_stmt, import_span) = item else {
            continue;
        };
        match &import_stmt.items {
            ImportItems::Named(_) => {
                if let Some(module_info) = module_cache
                    .load_module_by_import_with_context_and_source(
                        &import_stmt.from,
                        file_path,
                        workspace_root,
                        Some(text),
                    )
                {
                    if let Err(error) = compiler
                        .register_imported_items(&import_stmt.from, &module_info.program.items)
                    {
                        return Err(make_span_diagnostic(
                            text,
                            *import_span,
                            error.to_string(),
                        ));
                    }
                } else {
                    diagnostics.push(unresolved_module_diagnostic(
                        text,
                        *import_span,
                        &import_stmt.from,
                    ));
                }
            }
            ImportItems::Namespace { name, .. } => {
                if !known_module_names.iter().any(|module| module == name) {
                    diagnostics.push(unresolved_module_diagnostic(text, *import_span, name));
                }
            }
        }
    }

    Ok(ImportRegistrationOutcome {
        ready: diagnostics.is_empty(),
        diagnostics,
    })
}

fn unresolved_module_diagnostic(
    text: &str,
    span: shape_ast::ast::Span,
    name: &str,
) -> Diagnostic {
    make_span_diagnostic(
        text,
        span,
        format!(
            "Cannot resolve module '{}'. Verify the import path and declare dependencies in shape.toml when needed.",
            name
        ),
    )
}

fn make_span_diagnostic(
    text: &str,
    span: shape_ast::ast::Span,
    message: String,
) -> Diagnostic {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Diagnostic {
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
        severity: Some(DiagnosticSeverity::ERROR),
        message,
        source: Some("shape".to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
