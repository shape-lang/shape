//! Deterministic semantic diagnostics.
//!
//! A diagnostic is structured data: a frozen code, a severity, and sorted
//! key/value arguments. The rendered message is presentation and is therefore
//! *not* part of any content identity — rewording a message must not change a
//! published fact. Compiler and LSP render from the same structured value
//! rather than parsing each other's strings.

use shape_ast::ast::span::Span;

use crate::identity::{CanonicalDigest, DigestWriter};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    /// A fact this slice deliberately does not compute. Published so the gap is
    /// visible in the fact instead of being silently approximated.
    Note,
}

/// Frozen diagnostic codes. A code is never reused for a different meaning.
pub mod codes {
    pub const PARSE_FAILED: &str = "SEMDB0001";
    pub const DUPLICATE_CALLABLE_DECLARATION: &str = "SEMDB0002";
    pub const IMPORT_SHADOWED_BY_LOCAL_DECLARATION: &str = "SEMDB0003";
    pub const UNRESOLVED_CALLABLE: &str = "SEMDB0004";
    pub const UNRESOLVED_IMPORT_UNIT: &str = "SEMDB0005";
    pub const IMPORTED_DEFINITION_NOT_PUBLIC: &str = "SEMDB0006";
    pub const IMPORTED_DEFINITION_NOT_FOUND: &str = "SEMDB0007";
    pub const RESULT_TYPE_NOT_DECLARED: &str = "SEMDB0008";
    pub const PARAMETER_TYPE_NOT_DECLARED: &str = "SEMDB0009";
    pub const CALL_ARGUMENT_COUNT_MISMATCH: &str = "SEMDB0010";
    pub const CALL_ARGUMENT_TYPE_MISMATCH: &str = "SEMDB0011";
    pub const CALL_ARGUMENT_TYPE_NOT_STATIC: &str = "SEMDB0012";
    pub const PARAMETER_PATTERN_NOT_SUPPORTED: &str = "SEMDB0013";
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SemanticDiagnostic {
    pub code: &'static str,
    pub severity: DiagnosticSeverity,
    /// Sorted by key. Constructed through [`SemanticDiagnostic::new`], which
    /// sorts, so ordering can never depend on construction order.
    args: Vec<(String, String)>,
    /// Attached by the facts layer. `None` at the contract layer, which is
    /// span-free so that a span shift cannot invalidate a contract.
    pub span: Option<Span>,
}

/// Diagnostics sort by their semantic content only. A span is provenance: two
/// diagnostics that differ only in where they point must not reorder a
/// published list, or a span shift would change a fact's content identity.
impl Ord for SemanticDiagnostic {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.code
            .cmp(other.code)
            .then_with(|| self.severity.cmp(&other.severity))
            .then_with(|| self.args.cmp(&other.args))
    }
}

impl PartialOrd for SemanticDiagnostic {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl SemanticDiagnostic {
    pub fn new(
        code: &'static str,
        severity: DiagnosticSeverity,
        args: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        let mut args: Vec<(String, String)> = args
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        args.sort();
        SemanticDiagnostic {
            code,
            severity,
            args,
            span: None,
        }
    }

    pub fn error(
        code: &'static str,
        args: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Error, args)
    }

    pub fn warning(
        code: &'static str,
        args: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        Self::new(code, DiagnosticSeverity::Warning, args)
    }

    pub fn note(code: &'static str, args: impl IntoIterator<Item = (&'static str, String)>) -> Self {
        Self::new(code, DiagnosticSeverity::Note, args)
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn args(&self) -> &[(String, String)] {
        &self.args
    }

    pub fn arg(&self, key: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Renders the human-readable message. Presentation only.
    pub fn message(&self) -> String {
        let arg = |key: &str| self.arg(key).unwrap_or("?");
        match self.code {
            codes::PARSE_FAILED => format!("failed to parse `{}`: {}", arg("unit"), arg("error")),
            codes::DUPLICATE_CALLABLE_DECLARATION => format!(
                "`{}` is declared more than once in `{}`",
                arg("name"),
                arg("unit")
            ),
            codes::IMPORT_SHADOWED_BY_LOCAL_DECLARATION => format!(
                "local declaration of `{}` shadows the import from `{}`",
                arg("name"),
                arg("from")
            ),
            codes::UNRESOLVED_CALLABLE => {
                format!("no callable named `{}` is in scope", arg("name"))
            }
            codes::UNRESOLVED_IMPORT_UNIT => {
                format!("imported unit `{}` is not in the program", arg("from"))
            }
            codes::IMPORTED_DEFINITION_NOT_PUBLIC => format!(
                "`{}` is declared in `{}` but is not public",
                arg("name"),
                arg("from")
            ),
            codes::IMPORTED_DEFINITION_NOT_FOUND => format!(
                "`{}` is not declared in `{}`",
                arg("name"),
                arg("from")
            ),
            codes::RESULT_TYPE_NOT_DECLARED => format!(
                "`{}` has no declared result type; this slice publishes declared contracts only",
                arg("name")
            ),
            codes::PARAMETER_TYPE_NOT_DECLARED => format!(
                "parameter `{}` of `{}` has no declared type",
                arg("param"),
                arg("name")
            ),
            codes::CALL_ARGUMENT_COUNT_MISMATCH => format!(
                "`{}` expects {} argument(s), {} given",
                arg("callee"),
                arg("expected"),
                arg("actual")
            ),
            codes::CALL_ARGUMENT_TYPE_MISMATCH => format!(
                "argument {} of `{}` expects `{}`, found `{}`",
                arg("index"),
                arg("callee"),
                arg("expected"),
                arg("actual")
            ),
            codes::CALL_ARGUMENT_TYPE_NOT_STATIC => format!(
                "argument {} of `{}` is not a literal; this slice checks literal arguments only",
                arg("index"),
                arg("callee")
            ),
            codes::PARAMETER_PATTERN_NOT_SUPPORTED => format!(
                "parameter {} of `{}` uses a destructuring pattern, which this slice does not publish",
                arg("index"),
                arg("name")
            ),
            other => format!("{other} {:?}", self.args),
        }
    }
}

impl CanonicalDigest for SemanticDiagnostic {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.str(self.code);
        writer.u8(match self.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Note => 3,
        });
        writer.seq(&self.args);
        // `span` is provenance, not semantic content: it is written by the
        // facts layer's own provenance section, never here.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_order_does_not_affect_identity() {
        let one = SemanticDiagnostic::error(
            codes::CALL_ARGUMENT_COUNT_MISMATCH,
            [
                ("callee", "add".to_string()),
                ("expected", "2".to_string()),
                ("actual", "1".to_string()),
            ],
        );
        let two = SemanticDiagnostic::error(
            codes::CALL_ARGUMENT_COUNT_MISMATCH,
            [
                ("actual", "1".to_string()),
                ("expected", "2".to_string()),
                ("callee", "add".to_string()),
            ],
        );
        assert_eq!(one, two);
        assert_eq!(
            one.canonical_digest("test"),
            two.canonical_digest("test")
        );
    }

    #[test]
    fn span_is_not_part_of_diagnostic_content() {
        let bare = SemanticDiagnostic::error(codes::UNRESOLVED_CALLABLE, [("name", "add".into())]);
        let located = bare.clone().with_span(Span::new(10, 20));
        assert_eq!(
            bare.canonical_digest("test"),
            located.canonical_digest("test")
        );
    }
}
