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
use super::comptime_builtins::expansion_provenance::{ExpansionSite, GeneratedNodePath};

/// Comptime diagnostic-id namespace (LSDS `diagnostic_id`).
const COMPTIME_ERROR_ID: &str = "C0001";
const COMPTIME_WARNING_ID: &str = "C0002";
/// LSDS id for an error raised on a GENERATED declaration (ADR-009 D1
/// rejection row 7): the diagnostic carries generated-node + application +
/// generator locations as notes.
const GENERATED_DECL_ERROR_ID: &str = "C0003";

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

    /// ADR-009 D1 (S4), rejection row 7: convert an error raised while
    /// registering or compiling a GENERATED declaration into an LSDS
    /// diagnostic that carries the full expansion provenance — the
    /// generated node (declaration name + node path, anchored where the
    /// checked declaration lives), the application site, and the
    /// generator definition — as location-bearing [`shape_diagnostics::
    /// DiagnosticNote`]s. The LSP diagnostic bridge maps these notes to
    /// `relatedInformation`, so an editor can jump to all three.
    ///
    /// The checked generated declaration anchors at the application span
    /// until D2 virtual documents give it its own addressable text (S3
    /// anchoring rule) — the generated-node and application notes carry
    /// distinct roles at one location today.
    pub(crate) fn build_generated_decl_failure(
        &self,
        e: &ShapeError,
        decl_name: &str,
        node_path: &GeneratedNodePath,
        site: &ExpansionSite,
    ) -> ShapeError {
        let message = format!(
            "error in generated declaration `{decl_name}`: {}",
            super::helpers::clean_comptime_message(e)
        );
        let application_loc = self.comptime_lsds_location(site.application_span());
        let generator_loc = self.comptime_lsds_location(site.generator_span());

        let diag = shape_diagnostics::DiagnosticBuilder::new(
            GENERATED_DECL_ERROR_ID,
            shape_diagnostics::Severity::Error,
            application_loc.clone(),
            message,
        )
        .with_note(shape_diagnostics::DiagnosticNote::new(
            format!(
                "in generated declaration `{decl_name}` (generated node {}), \
                 whose checked declaration anchors here",
                node_path.render()
            ),
            Some(application_loc.clone()),
        ))
        .with_note(shape_diagnostics::DiagnosticNote::new(
            "generated from this application site",
            Some(application_loc),
        ))
        .with_note(shape_diagnostics::DiagnosticNote::new(
            "generator defined here",
            Some(generator_loc),
        ))
        .build();

        // LSDS is the source of truth; derive the terminal `ShapeError` and
        // enrich the caret exactly like `build_comptime_failure`.
        let mut err = super::functions::diagnostic_to_shape_error(&diag);
        if let ShapeError::SemanticError {
            location: Some(loc),
            ..
        } = &mut err
        {
            let sl = self.span_to_source_location(site.application_span());
            loc.source_line = sl.source_line;
            if sl.length.is_some() {
                loc.length = sl.length;
            }
        }
        err
    }

    /// ADR-009 D2 (Decision 67): route a declaration-discovery convergence
    /// failure — cycle / oscillation / unbounded generation / header mutated
    /// / reserved-identity-undefined — through the C0003 generated-declaration
    /// diagnostic family. When the failure is attributable to a specific
    /// expansion application (`site` is `Some`), the diagnostic carries the
    /// application + generator locations as notes, exactly like the row-7
    /// generated-declaration failure; a whole-graph convergence failure with
    /// no single owning application surfaces the named diagnostic message
    /// directly (still surface-and-stop, never a silent skip).
    pub(crate) fn build_discovery_failure(
        &self,
        message: String,
        site: Option<&ExpansionSite>,
    ) -> ShapeError {
        let Some(site) = site else {
            return ShapeError::SemanticError {
                message,
                location: None,
            };
        };
        let application_loc = self.comptime_lsds_location(site.application_span());
        let generator_loc = self.comptime_lsds_location(site.generator_span());
        let diag = shape_diagnostics::DiagnosticBuilder::new(
            GENERATED_DECL_ERROR_ID,
            shape_diagnostics::Severity::Error,
            application_loc.clone(),
            message,
        )
        .with_note(shape_diagnostics::DiagnosticNote::new(
            "generated from this application site",
            Some(application_loc.clone()),
        ))
        .with_note(shape_diagnostics::DiagnosticNote::new(
            "generator defined here",
            Some(generator_loc),
        ))
        .build();
        let mut err = super::functions::diagnostic_to_shape_error(&diag);
        if let ShapeError::SemanticError {
            location: Some(loc),
            ..
        } = &mut err
        {
            let sl = self.span_to_source_location(site.application_span());
            loc.source_line = sl.source_line;
            if sl.length.is_some() {
                loc.length = sl.length;
            }
        }
        err
    }

    /// ADR-009 D1 (S4): outer directive-processing wrap that PRESERVES a
    /// provenance-carrying generated-declaration failure. The row-7
    /// diagnostic built by [`Self::build_generated_decl_failure`] already
    /// carries its three location notes (generated node + application +
    /// generator); flattening it into a `format!("... failed: {e}")` string
    /// would silently drop the locations. Everything else keeps the
    /// existing directive-processing context wrap, byte-for-byte.
    pub(crate) fn preserve_or_wrap_directive_failure(
        &self,
        e: ShapeError,
        context: &str,
        span: Span,
    ) -> ShapeError {
        if let ShapeError::SemanticError {
            location: Some(location),
            ..
        } = &e
        {
            // Structural predicate: at the directive-processing sites the
            // only located-and-noted SemanticError is the row-7 generated-
            // declaration failure (its notes ARE the provenance).
            if !location.notes.is_empty() {
                return e;
            }
        }
        // A location-less RuntimeError is a bare directive message
        // ([`Self::directive_error`]); render it without the "Runtime
        // error:" Display prefix, matching the pre-S4 string shape.
        let rendered = match &e {
            ShapeError::RuntimeError {
                message,
                location: None,
            } => message.clone(),
            other => other.to_string(),
        };
        ShapeError::RuntimeError {
            message: format!("{context} directive processing failed: {rendered}"),
            location: Some(self.span_to_source_location(span)),
        }
    }

    /// A plain comptime-directive rejection message, located later by the
    /// directive-processing wrap ([`Self::preserve_or_wrap_directive_failure`]).
    pub(crate) fn directive_error(message: impl Into<String>) -> ShapeError {
        ShapeError::RuntimeError {
            message: message.into(),
            location: None,
        }
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
            // LSDS is the source of truth; pick the renderer from the
            // process-wide output format. Both channels go to stderr so a
            // non-fatal warning during a successful compile never corrupts the
            // program's own stdout.
            match shape_diagnostics::output_format() {
                shape_diagnostics::OutputFormat::Json => {
                    eprintln!("{}", shape_diagnostics::render::json::render(&diag));
                }
                shape_diagnostics::OutputFormat::Human => {
                    eprintln!(
                        "{}",
                        shape_diagnostics::render::terminal::render(&diag).trim_end()
                    );
                }
            }
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

    /// WF-3D F3 gate: the comptime `error()` / `warning()` diagnostics are
    /// built as canonical LSDS `Diagnostic`s (ids `C0001` / `C0002`) and the
    /// `shape_diagnostics::render::json` serializer emits the machine-readable
    /// shape the CLI `--diagnostics json` path ships. This gates the JSON
    /// contract independently of the process-wide `output_format` global.
    #[test]
    fn f3_comptime_error_renders_lsds_json_shape() {
        let loc = shape_diagnostics::Location::new(Some("prog.shape".to_string()), 2, 5, 10, 30);
        let diag = shape_diagnostics::DiagnosticBuilder::new(
            super::COMPTIME_ERROR_ID,
            shape_diagnostics::Severity::Error,
            loc,
            "boom about widget".to_string(),
        )
        .with_note(shape_diagnostics::DiagnosticNote::new(
            "during compile-time evaluation of the @json_schema annotation on Bad".to_string(),
            None,
        ))
        .build();

        let json = shape_diagnostics::render::json::render(&diag);
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("comptime error JSON must parse");
        assert_eq!(parsed["diagnostic_id"], "C0001");
        assert_eq!(parsed["severity"], "error");
        assert_eq!(parsed["message"], "boom about widget");
        assert_eq!(parsed["location"]["line"], 2);
        assert!(
            parsed["notes"][0]["message"]
                .as_str()
                .is_some_and(|m| m.contains("during compile-time evaluation")),
            "expected the comptime-trace note in the JSON payload: {json}"
        );
    }

    /// WF-3D F3 gate (warning side): the non-fatal `warning()` diagnostic uses
    /// the `C0002` id and renders through the same JSON serializer.
    #[test]
    fn f3_comptime_warning_renders_lsds_json_shape() {
        let loc = shape_diagnostics::Location::new(Some("prog.shape".to_string()), 7, 1, 40, 55);
        let diag = shape_diagnostics::DiagnosticBuilder::new(
            super::COMPTIME_WARNING_ID,
            shape_diagnostics::Severity::Warning,
            loc,
            "heads up about Foo".to_string(),
        )
        .build();

        let json = shape_diagnostics::render::json::render(&diag);
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("comptime warning JSON must parse");
        assert_eq!(parsed["diagnostic_id"], "C0002");
        assert_eq!(parsed["severity"], "warning");
        assert_eq!(parsed["message"], "heads up about Foo");
    }
}
