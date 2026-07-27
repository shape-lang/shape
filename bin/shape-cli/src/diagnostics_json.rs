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
}
