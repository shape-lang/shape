//! JSON renderer for [`crate::Diagnostic`].
//!
//! Serializes a single LSDS [`crate::Diagnostic`] to its canonical JSON wire
//! form (one object per line — the field names are the stability contract
//! documented on [`crate::Diagnostic`]). This is the payload machine callers
//! read: `shape run --diagnostics json`, and later the LSP / MCP consumers.
//!
//! The renderer is read-only: it never mutates the diagnostic.

use crate::Diagnostic;

/// Render one diagnostic as a single-line JSON object.
///
/// Serialization never fails for a well-formed `Diagnostic` (all fields are
/// plain data); the `Err` arm degrades to an empty object rather than
/// panicking so a diagnostic emission never aborts the process.
pub fn render(diagnostic: &Diagnostic) -> String {
    serde_json::to_string(diagnostic).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use crate::{DiagnosticBuilder, DiagnosticNote, Location, Severity};

    #[test]
    fn renders_error_with_location_and_notes() {
        let loc = Location::new(Some("prog.shape".to_string()), 3, 5, 20, 45);
        let diag = DiagnosticBuilder::new("C0001", Severity::Error, loc, "field X needs a type")
            .with_note(DiagnosticNote::new(
                "during compile-time evaluation of a compile-time block",
                None,
            ))
            .build();
        let json = super::render(&diag);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["severity"], "error");
        assert_eq!(v["location"]["line"], 3);
        assert_eq!(v["message"], "field X needs a type");
        assert!(v["notes"].as_array().is_some_and(|n| !n.is_empty()));
    }

    #[test]
    fn renders_warning_severity() {
        let diag = DiagnosticBuilder::new(
            "C0002",
            Severity::Warning,
            Location::new(Some("prog.shape".to_string()), 2, 1, 0, 10),
            "consider narrowing this type",
        )
        .build();
        let v: serde_json::Value = serde_json::from_str(&super::render(&diag)).expect("valid JSON");
        assert_eq!(v["severity"], "warning");
    }
}
