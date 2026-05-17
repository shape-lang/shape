//! LLM-Structured Diagnostic Schema (LSDS).
//!
//! Per ADR-006 §9, LSDS is the primary compiler diagnostic format. Renderers
//! (terminal, LSP, MCP) consume LSDS and produce human-readable / machine-
//! readable output. **LSDS is the source of truth** — text strings, LSP
//! `Diagnostic` payloads, and MCP tool responses are all derived from it.
//!
//! # Crate layout
//!
//! - [`Diagnostic`] — the canonical struct. JSON-serializable. Stable across
//!   versions per the ADR.
//! - [`Severity`], [`Location`], [`TypeWitness`], [`SuggestedFix`],
//!   [`ContextWindow`] — sub-structures referenced from `Diagnostic`.
//! - [`render`] — built-in renderers. Currently:
//!   - [`render::terminal`] — human-readable text output.
//!   LSP and MCP renderers are reserved for subsequent Phase 2 sessions.
//!
//! # Stability contract
//!
//! Field names in [`Diagnostic`] (and nested types) are part of the public
//! wire format. They must not be renamed or reordered without bumping the
//! schema version. Add new optional fields only; never remove or rename
//! existing ones.
//!
//! The schema version is exposed as [`SCHEMA_VERSION`].
//!
//! # Cross-references
//!
//! - ADR-006 §9 (`docs/adr/006-value-and-memory-model.md`) — binding spec.
//! - ADR-006 §13.5 success metric — average payload ≤500 cl100k tokens.
//! - `crates/shape-vm/src/mir/analysis.rs` — `BorrowError` /
//!   `BorrowErrorKind` / `BorrowErrorCode`, the source for the B-series
//!   diagnostics.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

pub mod render;

/// Wire-format schema version. Bumped on breaking changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Severity of a diagnostic.
///
/// Lower-cased in the wire format (`"error"`, `"warning"`, `"info"`,
/// `"hint"`). Renderers map these to terminal colours, LSP severities, and
/// MCP severity strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Compilation cannot proceed.
    Error,
    /// Compilation proceeds but the user should look.
    Warning,
    /// Informational — used for `var` inference inlay-hint suggestions
    /// (ADR-006 §1.3) and similar non-actionable feedback.
    Info,
    /// Hint — soft suggestions, e.g. style nits or refactor proposals
    /// surfaced by `@ai`-tuned consumers.
    Hint,
}

/// Source location of a diagnostic — a 1-based line/column plus an
/// absolute byte span.
///
/// `file` is the canonical path string; absent for synthetic / REPL
/// diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// File path; absent for synthetic / REPL / in-memory sources.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file: Option<String>,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number.
    pub col: u32,
    /// Absolute byte span `[start, end)` into the source buffer.
    pub span: [u32; 2],
}

impl Location {
    /// Construct a `Location` with a 1-based line/column and an absolute
    /// byte span.
    pub fn new(file: Option<String>, line: u32, col: u32, span_start: u32, span_end: u32) -> Self {
        Self {
            file,
            line,
            col,
            span: [span_start, span_end],
        }
    }

    /// Synthetic location with no file and zero positions — used for
    /// diagnostics not anchored to source (e.g., compiler-internal
    /// configuration errors).
    pub fn synthetic() -> Self {
        Self::new(None, 0, 0, 0, 0)
    }
}

/// A type witness — a concrete value that satisfies (`expected`) or
/// violates (`found`) the type constraint at the diagnostic site, per
/// ADR-006 §9.3.
///
/// `r#type` is the type's surface name (e.g. `"int"`, `"string"`,
/// `"Array<int>"`). `witness` is an optional concrete example value.
///
/// For simple primitive types (`int`, `number`, `bool`, `string`), the
/// emitter is encouraged to populate `witness`. For recursive / generic /
/// trait-bounded types, `witness` may be `None`; the surface name alone
/// communicates the constraint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeWitness {
    /// Surface name of the type (`"int"`, `"Option<string>"`, ...).
    #[serde(rename = "type")]
    pub r#type: String,
    /// Optional concrete value satisfying or violating the constraint.
    /// Encoded as a JSON value; LLM consumers parse it directly.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub witness: Option<serde_json::Value>,
}

impl TypeWitness {
    /// Construct a witness from a type name and an optional JSON value.
    pub fn new(type_name: impl Into<String>, witness: Option<serde_json::Value>) -> Self {
        Self {
            r#type: type_name.into(),
            witness,
        }
    }

    /// Construct a witness with only a type name and no concrete value.
    pub fn type_only(type_name: impl Into<String>) -> Self {
        Self::new(type_name, None)
    }
}

/// A suggested fix — a ranked, optionally-diff-bearing proposal that a
/// renderer (LSP code action, MCP `apply_fix` tool call) can apply.
///
/// `confidence` is in `[0.0, 1.0]`. Phase-2 first-session emitters may
/// produce empty `fixes` lists; richer fix generation is later-session
/// scope per the dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedFix {
    /// Short user-facing label (e.g. `"convert string to int"`).
    pub label: String,
    /// Optional unified-diff fragment. Renderers that can apply diffs
    /// (LSP, MCP) consume this directly. May be empty.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub diff: Option<String>,
    /// Confidence in the fix, `0.0..=1.0`. Renderers may rank by this.
    pub confidence: f32,
}

impl SuggestedFix {
    /// Construct a suggestion with a label and a confidence.
    pub fn new(label: impl Into<String>, confidence: f32) -> Self {
        Self {
            label: label.into(),
            diff: None,
            confidence,
        }
    }

    /// Attach a unified-diff fragment.
    pub fn with_diff(mut self, diff: impl Into<String>) -> Self {
        self.diff = Some(diff.into());
        self
    }
}

/// A token-budgeted context window — the smallest set of source spans
/// needed to understand the diagnostic, with a token count.
///
/// Per ADR-006 §9.5. LLM consumers use this to bound the source they
/// must include alongside the diagnostic. `tokens` is an estimate against
/// the cl100k tokenizer (per ADR-006 §13.5 success metric).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextWindow {
    /// Estimated token count for the included spans (cl100k).
    pub tokens: u32,
    /// Spans that comprise the context window.
    pub spans: Vec<ContextSpan>,
}

impl ContextWindow {
    /// Construct an empty context window with a token budget of zero.
    pub fn empty() -> Self {
        Self {
            tokens: 0,
            spans: Vec::new(),
        }
    }
}

/// A span of source — a file plus an inclusive line range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSpan {
    /// File path; absent for synthetic / REPL.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file: Option<String>,
    /// Inclusive 1-based line range `[start, end]`.
    pub lines: [u32; 2],
}

/// The canonical LSDS diagnostic.
///
/// JSON shape matches ADR-006 §9.2. Field names are part of the public
/// wire format; see crate-level docs for the stability contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable diagnostic identifier — e.g. `"B0013"`, `"E0100"`. The
    /// scheme matches the existing `BorrowErrorCode` (`B`-series for
    /// borrow / lifetime / aliasing) and `ErrorCode` (`E`-series for
    /// type, parse, semantic) namespaces.
    pub diagnostic_id: String,
    /// Severity bucket.
    pub severity: Severity,
    /// Primary source location.
    pub location: Location,
    /// Expected type at this site, when applicable. `None` for
    /// non-type-related diagnostics (e.g. parse errors).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expected: Option<TypeWitness>,
    /// Found type at this site, when applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub found: Option<TypeWitness>,
    /// Human-readable message body (does NOT include the
    /// `[B00XX]` prefix — that's the `diagnostic_id` field's job;
    /// renderers prepend it on output).
    pub message: String,
    /// Ranked suggested fixes; may be empty.
    #[serde(default)]
    pub fixes: Vec<SuggestedFix>,
    /// Token-budgeted context window for LLM consumers.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_window: Option<ContextWindow>,
    /// Citation pointing at the binding spec section that governs this
    /// diagnostic. E.g. `"ADR-006-§1.1"` or `"ADR-005-§4"`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rule: Option<String>,
    /// Auxiliary notes. Each note has its own location (e.g. "borrow
    /// originates here") so renderers can present them as related-info
    /// callouts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<DiagnosticNote>,
}

/// Auxiliary note attached to a diagnostic — e.g. "borrow originates
/// here", "binding declared here". Mirrors the existing `ErrorNote`
/// structure used by `ShapeError::SemanticError.location.notes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticNote {
    /// Note message.
    pub message: String,
    /// Location the note refers to; `None` for synthetic notes.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<Location>,
}

impl DiagnosticNote {
    /// Construct a note with a message and an optional location.
    pub fn new(message: impl Into<String>, location: Option<Location>) -> Self {
        Self {
            message: message.into(),
            location,
        }
    }
}

/// Builder for [`Diagnostic`]. Use this rather than struct literal at
/// emission sites so future schema evolution doesn't ripple through.
#[derive(Debug)]
pub struct DiagnosticBuilder {
    diagnostic_id: String,
    severity: Severity,
    location: Location,
    expected: Option<TypeWitness>,
    found: Option<TypeWitness>,
    message: String,
    fixes: Vec<SuggestedFix>,
    context_window: Option<ContextWindow>,
    rule: Option<String>,
    notes: Vec<DiagnosticNote>,
}

impl DiagnosticBuilder {
    /// Start building a diagnostic with the required minimum (id,
    /// severity, location, message).
    pub fn new(
        diagnostic_id: impl Into<String>,
        severity: Severity,
        location: Location,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostic_id: diagnostic_id.into(),
            severity,
            location,
            expected: None,
            found: None,
            message: message.into(),
            fixes: Vec::new(),
            context_window: None,
            rule: None,
            notes: Vec::new(),
        }
    }

    /// Attach an `expected` type witness.
    pub fn expected(mut self, witness: TypeWitness) -> Self {
        self.expected = Some(witness);
        self
    }

    /// Attach a `found` type witness.
    pub fn found(mut self, witness: TypeWitness) -> Self {
        self.found = Some(witness);
        self
    }

    /// Append a suggested fix.
    pub fn with_fix(mut self, fix: SuggestedFix) -> Self {
        self.fixes.push(fix);
        self
    }

    /// Attach a context window.
    pub fn context_window(mut self, window: ContextWindow) -> Self {
        self.context_window = Some(window);
        self
    }

    /// Attach a rule citation (`"ADR-006-§1.1"` etc.).
    pub fn rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Append an auxiliary note.
    pub fn with_note(mut self, note: DiagnosticNote) -> Self {
        self.notes.push(note);
        self
    }

    /// Finalize.
    pub fn build(self) -> Diagnostic {
        Diagnostic {
            diagnostic_id: self.diagnostic_id,
            severity: self.severity,
            location: self.location,
            expected: self.expected,
            found: self.found,
            message: self.message,
            fixes: self.fixes,
            context_window: self.context_window,
            rule: self.rule,
            notes: self.notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn diagnostic_round_trips_through_json() {
        let diag = DiagnosticBuilder::new(
            "B0013",
            Severity::Error,
            Location::new(Some("src/main.shape".into()), 12, 4, 102, 145),
            "expected int, found string",
        )
        .expected(TypeWitness::new("int", Some(json!(42))))
        .found(TypeWitness::new("string", Some(json!("hello"))))
        .with_fix(
            SuggestedFix::new("convert string to int", 0.85)
                .with_diff("let x: int = parse_int(value)?"),
        )
        .rule("ADR-006-§1.1")
        .build();

        let s = serde_json::to_string(&diag).expect("serialize");
        let back: Diagnostic = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(diag, back);
    }

    #[test]
    fn omitted_optional_fields_round_trip() {
        let diag = DiagnosticBuilder::new(
            "E0100",
            Severity::Error,
            Location::synthetic(),
            "type mismatch",
        )
        .build();

        let s = serde_json::to_string(&diag).expect("serialize");
        // No expected/found/fixes/context_window/rule/notes appear when empty.
        assert!(!s.contains("\"expected\""));
        assert!(!s.contains("\"found\""));
        // `fixes` is `default` (empty Vec) — not skipped, but encoded as `[]`.
        assert!(s.contains("\"fixes\":[]"));
        assert!(!s.contains("\"context_window\""));
        assert!(!s.contains("\"rule\""));
        assert!(!s.contains("\"notes\""));

        let back: Diagnostic = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(diag, back);
    }

    #[test]
    fn severity_serializes_lowercase() {
        let s = serde_json::to_string(&Severity::Error).unwrap();
        assert_eq!(s, "\"error\"");
        let s = serde_json::to_string(&Severity::Warning).unwrap();
        assert_eq!(s, "\"warning\"");
        let s = serde_json::to_string(&Severity::Info).unwrap();
        assert_eq!(s, "\"info\"");
        let s = serde_json::to_string(&Severity::Hint).unwrap();
        assert_eq!(s, "\"hint\"");
    }
}
