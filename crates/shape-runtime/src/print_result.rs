//! Output formatting carriers — `PrintResult` and `PrintSpan`.
//!
//! Per ADR-006 §2.7.4 (output adapter ruling), these types live in
//! `shape-runtime` rather than `shape-value`. They are runtime-tier
//! formatting concerns with no value-tier dependency: the structure carries
//! a rendered string and a vector of spans (literals or value references),
//! and is consumed by the [`crate::output_adapter::OutputAdapter`] trait.
//!
//! The pre-bulldozer placement under `shape-value` (as the
//! `RareHeapData::PrintResult` arm) was load-bearing only because
//! `ValueWord::from_print_result` returned a tagged heap pointer to a
//! `PrintResult`. After `ValueWord` deletion, output adapters return a
//! [`shape_value::KindedSlot`] directly; the `PrintResult` carrier moves
//! up to the runtime tier where it is consumed.

/// A single span within a rendered print() output.
///
/// Spans capture either literal text (no value reference) or formatted
/// output for a specific source-level value. Used by the REPL to enable
/// post-execution reformatting via `:reformat`.
#[derive(Debug, Clone)]
pub enum PrintSpan {
    /// Literal text emitted by the formatter (no value reference).
    Literal {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
    },
    /// Formatted output for a specific value, with the source expression
    /// preserved so the REPL can re-render with a different format spec.
    Value {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
        source_expr: String,
    },
}

/// Output of a `print()` call: rendered string plus per-span metadata.
///
/// The `rendered` field is the fully formatted string ready for stdout.
/// `spans` lets REPL-mode adapters preserve enough metadata to rerender
/// individual values without re-evaluating the whole expression.
#[derive(Debug, Clone)]
pub struct PrintResult {
    pub rendered: String,
    pub spans: Vec<PrintSpan>,
}
