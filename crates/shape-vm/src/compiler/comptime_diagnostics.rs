//! Comptime diagnostics presentation (comptime-excellence §4.4).
//!
//! Turns the raw outcome of a comptime block / annotation-handler execution
//! into a user-facing diagnostic that meets the Zig/Rust bar:
//!
//! - **LSDS-routed.** Every comptime error/warning is first built as a
//!   canonical [`shape_diagnostics::Diagnostic`] (the source of truth per
//!   ADR-006 §9), then derived into the terminal channel — errors via the
//!   [`diagnostic_to_shape_error`](super::functions::diagnostic_to_shape_error)
//!   bridge, warnings via `shape_diagnostics::render::terminal`.
//! - **Spanned.** The diagnostic is anchored at the driving construct — the
//!   `comptime` block or the `@annotation` application site — whose span the
//!   driver always knows (this matches the Rust proc-macro baseline). The
//!   rich source snippet + caret come from
//!   [`span_to_source_location`](super::BytecodeCompiler::span_to_source_location).
//! - **Traced.** A `comptime_trace` note records the compile-time context so
//!   the failure is attributed ("during compile-time evaluation of …").
//! - **Jargon-free.** The message is routed through the firewall
//!   ([`clean_comptime_message`](super::helpers::clean_comptime_message)) so
//!   no internal audit vocabulary ever reaches the user (acceptance P10).

use shape_ast::ast::Span;
use shape_ast::error::ShapeError;

use super::BytecodeCompiler;
use super::comptime_builtins::ComptimeDiagnostic;

/// Comptime diagnostic-id namespace (LSDS `diagnostic_id`).
const COMPTIME_ERROR_ID: &str = "C0001";
const COMPTIME_WARNING_ID: &str = "C0002";

impl BytecodeCompiler {
    /// Build the LSDS [`shape_diagnostics::Location`] for a comptime
    /// construct's `span`, carrying the file/line/col + byte span.
    fn comptime_lsds_location(&self, span: Span) -> shape_diagnostics::Location {
        let sl = self.span_to_source_location(span);
        shape_diagnostics::Location::new(
            sl.file.clone(),
            sl.line as u32,
            sl.column as u32,
            span.start as u32,
            span.end as u32,
        )
    }

    /// Convert a failed comptime execution into a spanned, jargon-free,
    /// LSDS-routed compile error.
    ///
    /// `context` describes the driving construct for the comptime trace
    /// (e.g. `"a compile-time block"` or `"the @json_schema annotation on
    /// User"`).
    pub(crate) fn build_comptime_failure(
        &self,
        e: &ShapeError,
        span: Span,
        context: &str,
    ) -> ShapeError {
        let message = super::helpers::clean_comptime_message(e);
        let lsds_loc = self.comptime_lsds_location(span);
        let trace = format!("during compile-time evaluation of {}", context);

        let diag = shape_diagnostics::DiagnosticBuilder::new(
            COMPTIME_ERROR_ID,
            shape_diagnostics::Severity::Error,
            lsds_loc,
            message,
        )
        .with_note(shape_diagnostics::DiagnosticNote::new(trace, None))
        .build();

        // LSDS is the source of truth; derive the terminal `ShapeError`.
        let mut err = super::functions::diagnostic_to_shape_error(&diag);

        // Enrich with the rich source snippet the compact LSDS `Location`
        // does not carry, so the terminal renderer draws the caret under the
        // failing construct.
        if let ShapeError::SemanticError {
            location: Some(loc),
            ..
        } = &mut err
        {
            let sl = self.span_to_source_location(span);
            loc.source_line = sl.source_line;
            if sl.length.is_some() {
                loc.length = sl.length;
            }
        }
        err
    }

    /// Re-emit the non-fatal `warning()` diagnostics collected during a
    /// comptime run, each anchored at the driving construct's `span` and
    /// rendered through the LSDS terminal renderer (spanned + LSDS-routed,
    /// replacing the deleted span-less `eprintln!`).
    pub(crate) fn surface_comptime_warnings(&self, warnings: &[ComptimeDiagnostic], span: Span) {
        if warnings.is_empty() {
            return;
        }
        let lsds_loc = self.comptime_lsds_location(span);
        for w in warnings {
            let diag = shape_diagnostics::DiagnosticBuilder::new(
                COMPTIME_WARNING_ID,
                shape_diagnostics::Severity::Warning,
                lsds_loc.clone(),
                w.message.clone(),
            )
            .build();
            eprintln!(
                "{}",
                shape_diagnostics::render::terminal::render(&diag).trim_end()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::helpers::{
        clean_comptime_message, comptime_message_has_jargon, sanitize_comptime_internal,
    };
    use shape_ast::error::ShapeError;

    /// The exact fragment set acceptance probe P10 greps rendered comptime
    /// output for — none may survive the firewall.
    const P10_FORBIDDEN: &[&str] = &[
        "ckpt",
        "ADR-",
        "V3-S5",
        "REFUSED",
        "§",
        "phase-2c",
        "SNAPSHOT_FUTURE",
        "NotImplemented(SURFACE",
    ];

    fn has_forbidden(s: &str) -> bool {
        P10_FORBIDDEN.iter().any(|f| s.contains(f))
    }

    #[test]
    fn firewall_strips_every_p10_jargon_fragment() {
        // A representative internal executor dump carrying multiple jargon
        // fragments at once.
        let dirty = "op_transform SURFACE V3-S5 ckpt-5 REFUSED ON SIGHT per ADR-006 §2.7.7 (phase-2c)";
        assert!(comptime_message_has_jargon(dirty));
        let clean = sanitize_comptime_internal(dirty);
        assert!(
            !has_forbidden(&clean),
            "firewall leaked jargon: {:?}",
            clean
        );
        assert_eq!(clean, "this operation is not available in compile-time code");
    }

    #[test]
    fn firewall_maps_watchdog_interrupt_to_budget_sentence() {
        let clean = sanitize_comptime_internal("Execution interrupted");
        assert_eq!(clean, "compile-time execution exceeded the 5-second limit");
        assert!(!has_forbidden(&clean));
    }

    #[test]
    fn firewall_passes_clean_internal_message_through() {
        // No jargon, no known-mapping → verbatim.
        let msg = "Undefined property: name";
        assert_eq!(sanitize_comptime_internal(msg), msg);
    }

    #[test]
    fn user_error_text_preserved_and_marker_stripped() {
        // `error()` prefixes its payload with the internal `[comptime error] `
        // marker; the user's text (even if it happens to contain a `§`) is
        // preserved verbatim, the marker removed.
        let e = ShapeError::RuntimeError {
            message: "Comptime handler execution failed: [comptime error] see spec §4 for the rule"
                .to_string(),
            location: None,
        };
        let cleaned = clean_comptime_message(&e);
        assert_eq!(cleaned, "see spec §4 for the rule");
    }

    #[test]
    fn internal_failure_without_marker_is_firewalled() {
        let e = ShapeError::RuntimeError {
            message: "Comptime handler execution failed: Not implemented: op_foo SURFACE ckpt-6"
                .to_string(),
            location: None,
        };
        let cleaned = clean_comptime_message(&e);
        assert!(!has_forbidden(&cleaned), "leaked jargon: {:?}", cleaned);
    }
}
