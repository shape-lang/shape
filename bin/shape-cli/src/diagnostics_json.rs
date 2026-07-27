//! Adapter: `ShapeError` → LSDS [`shape_diagnostics::Diagnostic`].
//!
//! `shape run --diagnostics json` renders compile/runtime failures as LSDS
//! JSON. The compiler builds its diagnostics as canonical LSDS internally, but
//! they reach the CLI as `ShapeError` (anyhow-wrapped). This module maps a
//! surfaced `ShapeError` back to the `Diagnostic` shape so
//! [`shape_diagnostics::render::json`] can serialize it: severity from the
//! variant, a real file/line/col location, and the comptime trace preserved as
//! `notes`.

use shape_diagnostics::{Diagnostic, DiagnosticBuilder, DiagnosticNote, Location, Severity};
use shape_runtime::error::{ErrorNote, ShapeError, SourceLocation};

/// Convert a surfaced (anyhow-wrapped) error into one or more LSDS diagnostics.
///
/// A `MultiError` fans out to one diagnostic per contained error. A non-Shape
/// anyhow error degrades to a single synthetic-location diagnostic carrying its
/// display string.
pub fn anyhow_to_diagnostics(err: &anyhow::Error) -> Vec<Diagnostic> {
    match err.downcast_ref::<ShapeError>() {
        Some(shape_err) => shape_error_to_diagnostics(shape_err),
        None => vec![
            DiagnosticBuilder::new(
                "E0000",
                Severity::Error,
                Location::synthetic(),
                err.to_string(),
            )
            .build(),
        ],
    }
}

/// Convert a `ShapeError` into one or more LSDS diagnostics (flattening
/// `MultiError`).
pub fn shape_error_to_diagnostics(err: &ShapeError) -> Vec<Diagnostic> {
    if let ShapeError::MultiError(errors) = err {
        return errors.iter().flat_map(shape_error_to_diagnostics).collect();
    }
    vec![single(err)]
}

fn single(err: &ShapeError) -> Diagnostic {
    let (default_id, raw_message, location) = decompose(err);
    let (id, message) = split_leading_code(&raw_message, default_id);
    let lsds_loc = location
        .map(source_location_to_lsds)
        .unwrap_or_else(Location::synthetic);

    let mut builder = DiagnosticBuilder::new(id, Severity::Error, lsds_loc, message);
    if let Some(loc) = location {
        for note in &loc.notes {
            builder = builder.with_note(error_note_to_lsds(note));
        }
        // ADR-017 §4: machine-applicable fixes are carried verbatim from the
        // emitter. This adapter re-derives nothing.
        for fix in &loc.fixes {
            builder = builder.with_fix(fix.clone());
        }
    }
    builder.build()
}

/// Pull the fallback id, the raw message, and the source location out of a
/// `ShapeError` variant. Variants that carry no location contribute `None`.
fn decompose(err: &ShapeError) -> (&'static str, String, Option<&SourceLocation>) {
    match err {
        ShapeError::SemanticError { message, location } => {
            ("SEMANTIC", message.clone(), location.as_ref())
        }
        ShapeError::RuntimeError { message, location } => {
            ("RUNTIME", message.clone(), location.as_ref())
        }
        ShapeError::ParseError { message, location } => {
            ("PARSE", message.clone(), location.as_ref())
        }
        ShapeError::LexError { message, location } => ("LEX", message.clone(), location.as_ref()),
        ShapeError::TypeError(message) => ("TYPE", message.clone(), None),
        ShapeError::VMError(message) => ("VM", message.clone(), None),
        ShapeError::PatternError { message, .. } => ("PATTERN", message.clone(), None),
        ShapeError::ModuleError { message, .. } => ("MODULE", message.clone(), None),
        ShapeError::DataError { message, .. } => ("DATA", message.clone(), None),
        other => ("E0000", other.to_string(), None),
    }
}

/// A message built through the LSDS bridge carries a leading `[CODE]` prefix
/// (the `diagnostic_id`); the canonical `Diagnostic.message` must not repeat
/// it. Extract the code when present, else fall back to `default_id`.
fn split_leading_code(raw: &str, default_id: &'static str) -> (String, String) {
    if let Some(rest) = raw.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let code = &rest[..end];
        let looks_like_code =
            !code.is_empty() && code.len() <= 8 && code.chars().all(|c| c.is_ascii_alphanumeric());
        if looks_like_code {
            let message = rest[end + 1..].trim_start().to_string();
            return (code.to_string(), message);
        }
    }
    (default_id.to_string(), raw.to_string())
}

/// `SourceLocation` carries line/col plus an optional length but no byte-offset
/// start; the LSDS span is best-effort `[0, length]`.
fn source_location_to_lsds(loc: &SourceLocation) -> Location {
    let span_end = loc.length.unwrap_or(0) as u32;
    Location::new(
        loc.file.clone(),
        loc.line as u32,
        loc.column as u32,
        0,
        span_end,
    )
}

fn error_note_to_lsds(note: &ErrorNote) -> DiagnosticNote {
    let location = note.location.as_ref().map(source_location_to_lsds);
    DiagnosticNote::new(note.message.clone(), location)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The comptime `error()` path lands as a `SemanticError` whose message
    /// carries a `[C0001]` prefix, a real line, and a comptime-trace note. The
    /// adapter must recover severity=error, the code, a non-line-1 location,
    /// and the trace as a note.
    #[test]
    fn comptime_semantic_error_maps_to_lsds() {
        let mut loc = SourceLocation::new(7, 3);
        loc.notes.push(ErrorNote {
            message: "during compile-time evaluation of a compile-time block".to_string(),
            location: None,
        });
        let err = ShapeError::SemanticError {
            message: "[C0001] field X needs a type".to_string(),
            location: Some(loc),
        };
        let diags = shape_error_to_diagnostics(&err);
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.diagnostic_id, "C0001");
        assert_eq!(d.message, "field X needs a type");
        assert_eq!(d.location.line, 7);
        assert!(!d.notes.is_empty());
        assert!(d.notes[0].message.contains("compile-time"));
    }

    #[test]
    fn multi_error_fans_out() {
        let a = ShapeError::SemanticError {
            message: "[C0001] one".to_string(),
            location: Some(SourceLocation::new(2, 1)),
        };
        let b = ShapeError::SemanticError {
            message: "[C0001] two".to_string(),
            location: Some(SourceLocation::new(5, 1)),
        };
        let diags = shape_error_to_diagnostics(&ShapeError::MultiError(vec![a, b]));
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[1].location.line, 5);
    }

    #[test]
    fn message_without_code_uses_fallback_id() {
        let (id, msg) = split_leading_code("plain message", "SEMANTIC");
        assert_eq!(id, "SEMANTIC");
        assert_eq!(msg, "plain message");
    }

    // --- ADR-017 §4 tripwire 1 ---

    /// A program whose match is missing one variant.
    const NON_EXHAUSTIVE: &str = "enum Status { Active, Inactive }\n\
                                  fn describe(s: Status) -> string {\n\
                                  \x20 match s {\n\
                                  \x20   Status::Active => \"on\",\n\
                                  \x20 }\n\
                                  }\n";

    /// Compile `source` the way `shape run` does — the run path sets the
    /// compiler's source (`shape_vm::execution`), which is what lets the
    /// checker bind a fix to a revision.
    fn compile_err(source: &str) -> ShapeError {
        let program = shape_ast::parse_program(source).expect("parse");
        let mut compiler = shape_vm::compiler::BytecodeCompiler::new();
        compiler.set_source(source);
        compiler
            .compile_in_place(&program)
            .expect_err("non-exhaustive match must fail to compile")
    }

    /// The CLI's consumer: `ShapeError` -> LSDS -> `--diagnostics json`
    /// bytes -> back -> apply. The JSON round trip is deliberate: the CLI
    /// emits text, so the fix has to survive serialization to be usable.
    fn apply_through_cli_json(source: &str, err: &ShapeError) -> String {
        let rendered: Vec<String> = shape_error_to_diagnostics(err)
            .iter()
            .map(shape_diagnostics::render::json::render)
            .collect();

        let plan = rendered
            .iter()
            .map(|line| {
                serde_json::from_str::<shape_diagnostics::Diagnostic>(line)
                    .expect("rendered LSDS is valid JSON")
            })
            .find_map(|diag| diag.fixes.into_iter().find_map(|fix| fix.edit_plan))
            .expect("a machine-applicable fix reaches --diagnostics json");

        plan.apply(source).expect("plan applies")
    }

    /// The LSP's consumer: analyze, take the published diagnostics, ask for
    /// code actions, apply the resulting `TextEdit`s.
    fn apply_through_lsp_code_action(source: &str) -> String {
        use tower_lsp_server::ls_types::{CodeActionOrCommand, TextEdit, Uri};

        let program = shape_ast::parse_program(source).expect("parse");
        let diagnostics =
            shape_lsp::analysis::analyze_program_semantics(&program, source, None, None, None);
        let uri = Uri::from_file_path("/tmp/tripwire.shape").expect("uri");

        let mut applied: Option<String> = None;
        for diagnostic in &diagnostics {
            let actions = shape_lsp::code_actions::get_code_actions(
                source,
                &uri,
                diagnostic.range,
                std::slice::from_ref(diagnostic),
                None,
                None,
            );
            for action in actions {
                let CodeActionOrCommand::CodeAction(action) = action else {
                    continue;
                };
                if !action.title.starts_with("Add missing match arm") {
                    continue;
                }
                let edits: Vec<TextEdit> = action
                    .edit
                    .as_ref()
                    .and_then(|e| e.changes.as_ref())
                    .and_then(|c| c.get(&uri))
                    .expect("edits for this document")
                    .clone();

                let mut out = source.to_string();
                // Apply back-to-front so earlier offsets stay valid.
                let mut ordered = edits;
                ordered.sort_by_key(|e| (e.range.start.line, e.range.start.character));
                for edit in ordered.iter().rev() {
                    let start = offset_of(source, edit.range.start);
                    let end = offset_of(source, edit.range.end);
                    out.replace_range(start..end, &edit.new_text);
                }
                applied = Some(out);
            }
        }

        applied.expect("the LSP offers the missing-arms code action")
    }

    fn offset_of(text: &str, position: tower_lsp_server::ls_types::Position) -> usize {
        let mut line = 0u32;
        let mut character = 0u32;
        for (offset, ch) in text.char_indices() {
            if line == position.line && character == position.character {
                return offset;
            }
            if ch == '\n' {
                line += 1;
                character = 0;
            } else {
                character += 1;
            }
        }
        text.len()
    }

    /// Tripwire 1: one compiler-emitted fix, two consumers, byte-identical
    /// result. This is the whole point of single-sourcing — if the CLI and
    /// the LSP could disagree, they would be two authorities again.
    #[test]
    fn compiler_fix_applies_identically_through_cli_json_and_lsp_code_action() {
        let err = compile_err(NON_EXHAUSTIVE);

        let via_cli = apply_through_cli_json(NON_EXHAUSTIVE, &err);
        let via_lsp = apply_through_lsp_code_action(NON_EXHAUSTIVE);

        assert_eq!(
            via_cli, via_lsp,
            "the same proved fix must produce the same bytes through both consumers"
        );

        // And it is the repair it claims to be, not merely a shared result.
        assert_eq!(
            via_cli,
            "enum Status { Active, Inactive }\n\
             fn describe(s: Status) -> string {\n\
             \x20 match s {\n\
             \x20   Status::Active => \"on\",\n\
             \x20   Status::Inactive => {\n\
             \x20   },\n\
             \x20 }\n\
             }\n"
        );
        assert!(shape_ast::parse_program(&via_cli).is_ok());
    }

    /// The CLI's JSON payload carries the spans themselves, not just a
    /// label — a machine caller can apply the fix without the compiler.
    #[test]
    fn cli_json_payload_carries_exact_spans() {
        let err = compile_err(NON_EXHAUSTIVE);
        let diags = shape_error_to_diagnostics(&err);
        let rendered = shape_diagnostics::render::json::render(
            diags
                .iter()
                .find(|d| !d.fixes.is_empty())
                .expect("a fix-bearing diagnostic"),
        );

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        let edit = &value["fixes"][0]["edit_plan"]["edits"][0];
        assert!(edit["span"].is_array(), "payload: {rendered}");
        assert!(
            edit["new_text"]
                .as_str()
                .expect("new_text")
                .contains("Status::Inactive")
        );
        assert!(value["fixes"][0]["edit_plan"]["source_digest"].is_string());
    }
}
