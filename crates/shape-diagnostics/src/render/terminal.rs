//! Terminal renderer — produces a plain-text representation of an LSDS
//! [`crate::Diagnostic`].
//!
//! The output shape approximates the existing `ShapeError::SemanticError`
//! rendering for borrow errors so a shape-runtime CLI consumer (or the
//! existing `eprintln!` paths) can switch to LSDS without users noticing
//! a regression.
//!
//! Layout (roughly):
//!
//! ```text
//! error[B0013]: cannot pass the same variable to multiple parameters that require non-aliased access
//!  --> src/main.shape:12:4
//!   = expected: int (witness: 42)
//!   = found:    string (witness: "hello")
//!   = hint: use separate variables or clone one of the arguments
//!   = note: conflicting argument originates here (src/main.shape:9:2)
//!   = rule: ADR-006-§1.1
//! ```
//!
//! No ANSI colour for now. A `ColorMode` parameter is the natural
//! follow-up; deferred to a later session per dispatch scope.

use crate::{Diagnostic, Severity};

/// Render a single [`Diagnostic`] to plain text.
pub fn render(diag: &Diagnostic) -> String {
    let mut out = String::new();

    // Header line: `severity[ID]: message`.
    let severity_word = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    };
    out.push_str(severity_word);
    out.push('[');
    out.push_str(&diag.diagnostic_id);
    out.push_str("]: ");
    out.push_str(&diag.message);
    out.push('\n');

    // Location pointer.
    out.push_str(" --> ");
    out.push_str(&format_location(&diag.location));
    out.push('\n');

    // Expected/found.
    if let Some(expected) = &diag.expected {
        out.push_str("  = expected: ");
        out.push_str(&format_witness(expected));
        out.push('\n');
    }
    if let Some(found) = &diag.found {
        out.push_str("  = found:    ");
        out.push_str(&format_witness(found));
        out.push('\n');
    }

    // Suggested fixes (label + confidence; diff intentionally omitted in
    // terminal render for compactness).
    for fix in &diag.fixes {
        out.push_str("  = fix: ");
        out.push_str(&fix.label);
        if fix.confidence > 0.0 {
            out.push_str(&format!(" (confidence: {:.0}%)", fix.confidence * 100.0));
        }
        out.push('\n');
    }

    // Notes.
    for note in &diag.notes {
        out.push_str("  = note: ");
        out.push_str(&note.message);
        if let Some(loc) = &note.location {
            out.push_str(" (");
            out.push_str(&format_location(loc));
            out.push(')');
        }
        out.push('\n');
    }

    // Rule citation.
    if let Some(rule) = &diag.rule {
        out.push_str("  = rule: ");
        out.push_str(rule);
        out.push('\n');
    }

    out
}

fn format_location(loc: &crate::Location) -> String {
    let file = loc.file.as_deref().unwrap_or("<synthetic>");
    format!("{}:{}:{}", file, loc.line, loc.col)
}

fn format_witness(w: &crate::TypeWitness) -> String {
    match &w.witness {
        Some(v) => format!("{} (witness: {})", w.r#type, v),
        None => w.r#type.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextWindow, DiagnosticBuilder, DiagnosticNote, Location, Severity, SuggestedFix,
        TypeWitness,
    };
    use serde_json::json;

    fn sample_b0013() -> Diagnostic {
        DiagnosticBuilder::new(
            "B0013",
            Severity::Error,
            Location::new(Some("src/main.shape".into()), 12, 4, 102, 145),
            "cannot pass the same variable to multiple parameters that require non-aliased access",
        )
        .expected(TypeWitness::new("int", Some(json!(42))))
        .found(TypeWitness::new("string", Some(json!("hello"))))
        .with_fix(SuggestedFix::new(
            "use separate variables or clone one of the arguments",
            0.85,
        ))
        .with_note(DiagnosticNote::new(
            "conflicting argument originates here",
            Some(Location::new(Some("src/main.shape".into()), 9, 2, 60, 70)),
        ))
        .context_window(ContextWindow::empty())
        .rule("ADR-006-§1.1")
        .build()
    }

    #[test]
    fn terminal_render_snapshot() {
        let diag = sample_b0013();
        let out = render(&diag);
        // Snapshot: pinning exact text shape so callers can rely on it.
        let expected = "\
error[B0013]: cannot pass the same variable to multiple parameters that require non-aliased access
 --> src/main.shape:12:4
  = expected: int (witness: 42)
  = found:    string (witness: \"hello\")
  = fix: use separate variables or clone one of the arguments (confidence: 85%)
  = note: conflicting argument originates here (src/main.shape:9:2)
  = rule: ADR-006-§1.1
";
        assert_eq!(out, expected);
    }

    #[test]
    fn terminal_render_minimal_diagnostic() {
        // No expected/found/fixes/notes/rule — renders header + location only.
        let diag = DiagnosticBuilder::new(
            "E0100",
            Severity::Error,
            Location::new(Some("test.shape".into()), 3, 1, 10, 20),
            "type mismatch",
        )
        .build();
        let out = render(&diag);
        assert_eq!(
            out,
            "error[E0100]: type mismatch\n --> test.shape:3:1\n"
        );
    }

    #[test]
    fn terminal_render_synthetic_location() {
        let diag = DiagnosticBuilder::new(
            "E0001",
            Severity::Warning,
            Location::synthetic(),
            "config notice",
        )
        .build();
        let out = render(&diag);
        assert!(out.starts_with("warning[E0001]: config notice"));
        assert!(out.contains("<synthetic>:0:0"));
    }
}
