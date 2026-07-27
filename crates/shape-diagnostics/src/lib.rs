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

use std::sync::atomic::{AtomicU8, Ordering};

/// Wire-format schema version. Bumped on breaking changes.
pub const SCHEMA_VERSION: u32 = 1;

/// How a diagnostic renders when a producer surfaces it to an output stream.
///
/// This is a process-wide rendering choice — the same LSDS [`Diagnostic`] is
/// the source of truth regardless of format; the format only selects which
/// renderer ([`render::terminal`] vs. [`render::json`]) a surfacing site uses.
/// The CLI sets it once at startup (`shape run --diagnostics json`) so that
/// both the compile-error path and any non-fatal warning surfaced mid-compile
/// emit the same shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable terminal text (the default).
    #[default]
    Human,
    /// One LSDS JSON object per diagnostic.
    Json,
}

static OUTPUT_FORMAT: AtomicU8 = AtomicU8::new(0);

/// Set the process-wide diagnostic output format. Call once at startup.
pub fn set_output_format(format: OutputFormat) {
    let encoded = match format {
        OutputFormat::Human => 0,
        OutputFormat::Json => 1,
    };
    OUTPUT_FORMAT.store(encoded, Ordering::Relaxed);
}

/// Read the process-wide diagnostic output format (defaults to
/// [`OutputFormat::Human`]).
pub fn output_format() -> OutputFormat {
    match OUTPUT_FORMAT.load(Ordering::Relaxed) {
        1 => OutputFormat::Json,
        _ => OutputFormat::Human,
    }
}

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
    /// surfaced by tooling consumers.
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

/// One exact replacement: a byte span into the source the emitter compiled,
/// plus the text that replaces it.
///
/// `span` is `[start, end)` in bytes. An insertion is the degenerate case
/// `start == end`. Per ADR-017 §4 this is the machine-applicable form of a
/// fix — consumers apply it verbatim and add nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredEdit {
    /// Absolute byte span `[start, end)` to replace.
    pub span: [u32; 2],
    /// Replacement text. Empty means deletion.
    pub new_text: String,
}

impl StructuredEdit {
    /// Replace `[start, end)` with `new_text`.
    pub fn replacement(start: u32, end: u32, new_text: impl Into<String>) -> Self {
        Self {
            span: [start, end],
            new_text: new_text.into(),
        }
    }

    /// Insert `new_text` at byte offset `at`.
    pub fn insertion(at: u32, new_text: impl Into<String>) -> Self {
        Self::replacement(at, at, new_text)
    }

    /// Start of the replaced span, in bytes.
    pub fn start(&self) -> u32 {
        self.span[0]
    }

    /// End of the replaced span, in bytes (exclusive).
    pub fn end(&self) -> u32 {
        self.span[1]
    }
}

/// Why a [`EditPlan`] refused to apply.
///
/// Every variant is a refusal, never a partial application: a plan either
/// applies whole or changes nothing. ADR-017 §4 requires an evidence-backed
/// fix, and evidence proved against source the emitter no longer recognizes
/// is not evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixRejection {
    /// The source changed after the emitter proved the fix. The plan's spans
    /// describe a document that no longer exists.
    SourceChanged {
        /// Digest recorded when the fix was emitted.
        expected: String,
        /// Digest of the source the consumer offered.
        actual: String,
    },
    /// A span reaches past the end of the source.
    SpanOutOfBounds {
        /// The offending span.
        span: [u32; 2],
        /// Length of the source in bytes.
        source_len: u32,
    },
    /// A span endpoint falls inside a UTF-8 code point.
    SpanNotCharBoundary {
        /// The byte offset that is not a boundary.
        offset: u32,
    },
    /// Two edits in the same plan cover overlapping bytes.
    OverlappingEdits {
        /// The earlier span.
        first: [u32; 2],
        /// The span that overlaps it.
        second: [u32; 2],
    },
    /// The plan carries no edits, so applying it is a no-op the caller
    /// should not present as a fix.
    Empty,
}

impl std::fmt::Display for FixRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceChanged { expected, actual } => write!(
                f,
                "source changed since the fix was proved (expected digest {expected}, found {actual})"
            ),
            Self::SpanOutOfBounds { span, source_len } => write!(
                f,
                "edit span [{}, {}) reaches past the {source_len}-byte source",
                span[0], span[1]
            ),
            Self::SpanNotCharBoundary { offset } => {
                write!(f, "edit offset {offset} is not a UTF-8 character boundary")
            }
            Self::OverlappingEdits { first, second } => write!(
                f,
                "edits [{}, {}) and [{}, {}) overlap",
                first[0], first[1], second[0], second[1]
            ),
            Self::Empty => write!(f, "edit plan carries no edits"),
        }
    }
}

impl std::error::Error for FixRejection {}

/// The complete machine-applicable edit set for one fix, bound to the source
/// revision the emitter proved it against.
///
/// `source_digest` is what makes the plan falsifiable: a consumer that holds
/// a different revision of the file gets [`FixRejection::SourceChanged`]
/// instead of a silently misplaced edit. It is a staleness check, not a
/// security check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditPlan {
    /// Digest of the source text the emitter compiled. See [`source_digest`].
    pub source_digest: String,
    /// Edits to apply. Order is irrelevant; they may not overlap.
    pub edits: Vec<StructuredEdit>,
}

impl EditPlan {
    /// Build a plan for `source` from the edits it was proved against.
    pub fn new(source: &str, edits: Vec<StructuredEdit>) -> Self {
        Self {
            source_digest: source_digest(source),
            edits,
        }
    }

    /// Check the plan against `source` without applying it.
    ///
    /// Consumers that need to decide whether to *offer* a fix (an LSP
    /// deciding whether to publish a code action) call this; consumers that
    /// apply call [`EditPlan::apply`], which repeats the check.
    pub fn validate(&self, source: &str) -> Result<(), FixRejection> {
        if self.edits.is_empty() {
            return Err(FixRejection::Empty);
        }

        let actual = source_digest(source);
        if actual != self.source_digest {
            return Err(FixRejection::SourceChanged {
                expected: self.source_digest.clone(),
                actual,
            });
        }

        let source_len = source.len() as u32;
        for edit in &self.edits {
            let (start, end) = (edit.start(), edit.end());
            if start > end || end > source_len {
                return Err(FixRejection::SpanOutOfBounds {
                    span: edit.span,
                    source_len,
                });
            }
            for offset in [start, end] {
                if !source.is_char_boundary(offset as usize) {
                    return Err(FixRejection::SpanNotCharBoundary { offset });
                }
            }
        }

        let mut ordered: Vec<&StructuredEdit> = self.edits.iter().collect();
        ordered.sort_by_key(|edit| (edit.start(), edit.end()));
        for pair in ordered.windows(2) {
            if pair[1].start() < pair[0].end() {
                return Err(FixRejection::OverlappingEdits {
                    first: pair[0].span,
                    second: pair[1].span,
                });
            }
        }

        Ok(())
    }

    /// Apply the plan to `source`, or refuse.
    ///
    /// This is the single authority for turning structured edits into text.
    /// Every consumer — CLI `--fix`, LSP code action, MCP `apply_fix` —
    /// applies through here or through a mechanical projection of the same
    /// spans, so a fix cannot mean two things.
    pub fn apply(&self, source: &str) -> Result<String, FixRejection> {
        self.validate(source)?;

        let mut ordered: Vec<&StructuredEdit> = self.edits.iter().collect();
        ordered.sort_by_key(|edit| (edit.start(), edit.end()));

        let mut out = String::with_capacity(source.len());
        let mut cursor = 0usize;
        for edit in ordered {
            out.push_str(&source[cursor..edit.start() as usize]);
            out.push_str(&edit.new_text);
            cursor = edit.end() as usize;
        }
        out.push_str(&source[cursor..]);
        Ok(out)
    }
}

/// Digest of a source buffer, used to bind an [`EditPlan`] to the revision
/// it was proved against.
///
/// FNV-1a over the bytes, salted with the length so that a same-length
/// permutation and a length change are both visible. Rendered as
/// `<len>:<hash>` so a mismatch is readable in a diagnostic payload.
pub fn source_digest(source: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x1000_0000_01b3;

    let mut hash = OFFSET;
    for byte in source.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{}:{:016x}", source.len(), hash)
}

/// A suggested fix — a ranked proposal that a renderer (LSP code action, MCP
/// `apply_fix` tool call) can apply.
///
/// `confidence` is in `[0.0, 1.0]`. A fix carrying an [`EditPlan`] is
/// machine-applicable; one carrying only a `label` (and possibly a `diff`) is
/// advice the user applies by hand.
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
    /// Exact spans plus replacement text, when the emitter proved a
    /// machine-applicable edit (ADR-017 §4). Appended field — absent on
    /// fixes that carry only advice.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edit_plan: Option<EditPlan>,
}

impl SuggestedFix {
    /// Construct a suggestion with a label and a confidence.
    pub fn new(label: impl Into<String>, confidence: f32) -> Self {
        Self {
            label: label.into(),
            diff: None,
            confidence,
            edit_plan: None,
        }
    }

    /// Attach a unified-diff fragment.
    pub fn with_diff(mut self, diff: impl Into<String>) -> Self {
        self.diff = Some(diff.into());
        self
    }

    /// Attach the machine-applicable edits, binding them to `source`.
    pub fn with_edits(mut self, source: &str, edits: Vec<StructuredEdit>) -> Self {
        self.edit_plan = Some(EditPlan::new(source, edits));
        self
    }

    /// Attach an already-built plan.
    pub fn with_edit_plan(mut self, plan: EditPlan) -> Self {
        self.edit_plan = Some(plan);
        self
    }

    /// Whether this fix carries edits a tool can apply without guessing.
    pub fn is_machine_applicable(&self) -> bool {
        self.edit_plan.is_some()
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

    // --- ADR-017 §4 structured-edit channel ---

    const MATCH_SOURCE: &str = "match c {\n  Red => 1,\n}\n";

    fn insert_arm_plan() -> EditPlan {
        // Insert a second arm just before the closing brace on line 3.
        let at = MATCH_SOURCE.find("\n}").expect("closing brace") as u32 + 1;
        EditPlan::new(
            MATCH_SOURCE,
            vec![StructuredEdit::insertion(at, "  Blue => 2,\n")],
        )
    }

    #[test]
    fn edit_plan_applies_exact_spans() {
        let applied = insert_arm_plan().apply(MATCH_SOURCE).expect("applies");
        assert_eq!(applied, "match c {\n  Red => 1,\n  Blue => 2,\n}\n");
    }

    #[test]
    fn edit_plan_replacement_swaps_span_contents() {
        let source = "let x = 1";
        let plan = EditPlan::new(
            source,
            vec![StructuredEdit::replacement(4, 5, "renamed".to_string())],
        );
        assert_eq!(plan.apply(source).expect("applies"), "let renamed = 1");
    }

    #[test]
    fn edit_plan_applies_multiple_edits_without_offset_drift() {
        let source = "a b c";
        let plan = EditPlan::new(
            source,
            vec![
                StructuredEdit::replacement(4, 5, "third"),
                StructuredEdit::replacement(0, 1, "first"),
            ],
        );
        assert_eq!(plan.apply(source).expect("applies"), "first b third");
    }

    /// Tripwire 3: a fix whose source moved under it is rejected, not
    /// applied at the stale offsets.
    #[test]
    fn stale_plan_is_rejected_not_misapplied() {
        let plan = insert_arm_plan();
        // The user typed a line above the match before invoking the fix.
        let edited = format!("let c = Color::Red\n{MATCH_SOURCE}");

        let rejection = plan.apply(&edited).expect_err("stale plan must refuse");
        assert!(matches!(rejection, FixRejection::SourceChanged { .. }));
        assert!(plan.validate(&edited).is_err());

        // And the misapplication the rejection prevents would have been
        // real: at the stale offset the insertion lands mid-statement.
        let stale_offset = plan.edits[0].start() as usize;
        assert!(
            !edited[stale_offset..].starts_with('}'),
            "offsets must actually have moved for this tripwire to bite"
        );
    }

    #[test]
    fn plan_out_of_bounds_span_is_rejected() {
        let source = "short";
        let plan = EditPlan {
            source_digest: source_digest(source),
            edits: vec![StructuredEdit::replacement(0, 99, "x")],
        };
        assert!(matches!(
            plan.apply(source),
            Err(FixRejection::SpanOutOfBounds { .. })
        ));
    }

    #[test]
    fn plan_split_code_point_is_rejected() {
        let source = "é";
        let plan = EditPlan {
            source_digest: source_digest(source),
            edits: vec![StructuredEdit::replacement(0, 1, "e")],
        };
        assert!(matches!(
            plan.apply(source),
            Err(FixRejection::SpanNotCharBoundary { offset: 1 })
        ));
    }

    #[test]
    fn plan_overlapping_edits_are_rejected() {
        let source = "abcdef";
        let plan = EditPlan {
            source_digest: source_digest(source),
            edits: vec![
                StructuredEdit::replacement(0, 3, "x"),
                StructuredEdit::replacement(2, 5, "y"),
            ],
        };
        assert!(matches!(
            plan.apply(source),
            Err(FixRejection::OverlappingEdits { .. })
        ));
    }

    #[test]
    fn empty_plan_is_rejected() {
        let plan = EditPlan {
            source_digest: source_digest("x"),
            edits: Vec::new(),
        };
        assert_eq!(plan.apply("x"), Err(FixRejection::Empty));
    }

    #[test]
    fn source_digest_separates_same_length_permutations() {
        assert_ne!(source_digest("ab"), source_digest("ba"));
        assert_ne!(source_digest("ab"), source_digest("ab "));
        assert_eq!(source_digest("ab"), source_digest("ab"));
    }

    #[test]
    fn edit_plan_round_trips_through_json() {
        let fix =
            SuggestedFix::new("Add missing match arms", 0.95).with_edit_plan(insert_arm_plan());
        assert!(fix.is_machine_applicable());

        let encoded = serde_json::to_string(&fix).expect("serialize");
        let decoded: SuggestedFix = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(fix, decoded);
        assert_eq!(
            decoded
                .edit_plan
                .expect("plan survives")
                .apply(MATCH_SOURCE)
                .expect("applies"),
            "match c {\n  Red => 1,\n  Blue => 2,\n}\n"
        );
    }

    /// The schema is append-only: a payload written before `edit_plan`
    /// existed still deserializes, and a fix without edits still omits the
    /// field.
    #[test]
    fn edit_plan_field_is_append_only() {
        let legacy = r#"{"label":"advice only","confidence":0.5}"#;
        let decoded: SuggestedFix = serde_json::from_str(legacy).expect("legacy payload");
        assert_eq!(decoded.edit_plan, None);
        assert!(!decoded.is_machine_applicable());

        let encoded = serde_json::to_string(&SuggestedFix::new("advice only", 0.5)).unwrap();
        assert!(!encoded.contains("edit_plan"));
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
