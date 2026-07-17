//! ADR-009 C2 #13 (slice 4) — the D7 edit-transaction shape guards
//! (`[C0924]` split/two-identity, `[C0925]` incomplete environment).
//!
//! An existing-body edit (`replace body`, incl. a capture-set change) must
//! commit its capture-set change and its body change as ONE atomic rewrite under
//! ONE expansion identity, beginning from the complete current capture set (D7 /
//! C2-R7 / C2-R8). In the shipped architecture that is STRUCTURAL: a `replace
//! body` edit is one `compile_in_place` install transaction, its replacement is
//! stamped with a single expansion identity (`stamp_generated_replacement_body`),
//! and its capture environment is discovered from the replacement body and
//! validated by the C1 capture-plan gates. So neither `[C0924]` nor `[C0925]` is
//! constructible from a real program without production sabotage.
//!
//! These guards are therefore DEFENSE-IN-DEPTH: `guard_edit_transaction_shape`
//! asserts the two invariants over real values at the edit-commit seam, and
//! surfaces the named code if a FUTURE refactor ever splits the identity or
//! installs an edit from an incomplete / foreign environment. The failure
//! branches are structurally unreachable today (the pins are marked
//! not-constructible with that reason in `c2_slice4_edit_tests`), but the
//! constructors are exercised directly there so the messages stay well-formed
//! and jargon-clean. Both follow `[C0922]`'s construction: a `SemanticError`
//! with a bracketed code at the head and a MARKER-FREE message (no
//! `COMPTIME_JARGON_MARKERS` token — "rejected", not the banned "refused"; no
//! "wave-"), so the comptime-diagnostics firewall passes them through unchanged.

use shape_ast::ast::GeneratedNodeOrigin;
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::ExpansionSite;

impl BytecodeCompiler {
    /// Assert the D7 edit-transaction invariants for a `replace body` edit whose
    /// replacement was just stamped with `replacement_origin` at `site`. Called
    /// from the `replace body` directive arm. `Ok(())` in the shipped
    /// architecture (both invariants hold by construction); an `Err` is the named
    /// installation rejection a future refactor would trip.
    pub(in crate::compiler) fn guard_edit_transaction_shape(
        &self,
        replacement_origin: &GeneratedNodeOrigin,
        site: &ExpansionSite,
    ) -> Result<()> {
        // [C0924] one rewrite transaction, one expansion identity: the
        // replacement's expansion identity must be the SAME as its application
        // site's. `stamp_generated_replacement_body` derives the origin from
        // `site.identity()`, so a divergence here means the body change and the
        // capture-set change were minted under two identities (a split rewrite).
        let site_fingerprint = site.identity().fingerprint();
        let (origin_high, origin_low) = replacement_origin.expansion_fingerprint();
        if origin_high != site_fingerprint.high || origin_low != site_fingerprint.low {
            return Err(self.split_rewrite_transaction_rejection());
        }

        // [C0925] complete current environment: the edit's environment provenance
        // must be issued by THIS compilation (a complete, non-partial,
        // non-serialized environment) and carry a rooted declaration anchor. A
        // foreign / serialized / anchorless origin means the edit was installed
        // without its complete current capture environment.
        let environment_is_complete = self.generated_node_issuer.recognizes(replacement_origin)
            && !replacement_origin.path().typed_segments().is_empty();
        if !environment_is_complete {
            return Err(self.incomplete_environment_rejection());
        }

        Ok(())
    }

    /// The named `[C0924]` rejection (ADR-009 C2 #13, D7 / C2-R7). Marker-free
    /// per the `[C0922]` convention so the comptime-diagnostics firewall passes
    /// it through unchanged.
    fn split_rewrite_transaction_rejection(&self) -> ShapeError {
        ShapeError::SemanticError {
            message: "[C0924] an existing-body edit published its capture-set change and its \
                      body change under two rewrite transactions or two expansion identities; a \
                      generated-body edit must commit both as one atomic change under a single \
                      expansion identity. This edit is rejected."
                .to_string(),
            location: None,
        }
    }

    /// The named `[C0925]` rejection (ADR-009 C2 #13, D7 / C2-R8). Marker-free
    /// per the `[C0922]` convention.
    fn incomplete_environment_rejection(&self) -> ShapeError {
        ShapeError::SemanticError {
            message: "[C0925] an existing-body edit was installed without its complete current \
                      capture environment; a generated-body edit must begin from the whole \
                      current capture set and publish the environment layout, the ownership and \
                      drop plan, and the generated references together. This edit is rejected."
                .to_string(),
            location: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use crate::compiler::helpers::comptime_message_has_jargon;
    use shape_ast::error::ShapeError;

    fn rendered(error: ShapeError) -> String {
        assert!(
            matches!(error, ShapeError::SemanticError { .. }),
            "the D7 guard codes are SemanticErrors (the [C0922] convention)"
        );
        error.to_string()
    }

    /// The `[C0924]` constructor carries its bracketed code and is jargon-clean,
    /// so the comptime-diagnostics firewall passes it through the generated-
    /// declaration envelope unchanged (as it does for [C0922] / [C0923]).
    #[test]
    fn c0924_message_is_well_formed_and_marker_free() {
        let compiler = BytecodeCompiler::new();
        let message = rendered(compiler.split_rewrite_transaction_rejection());
        assert!(
            message.contains("[C0924]"),
            "the split-transaction rejection carries its bracketed code: {message}"
        );
        assert!(
            !comptime_message_has_jargon(&message),
            "the [C0924] message must be free of COMPTIME_JARGON_MARKERS: {message}"
        );
    }

    /// The `[C0925]` constructor carries its bracketed code and is jargon-clean.
    #[test]
    fn c0925_message_is_well_formed_and_marker_free() {
        let compiler = BytecodeCompiler::new();
        let message = rendered(compiler.incomplete_environment_rejection());
        assert!(
            message.contains("[C0925]"),
            "the incomplete-environment rejection carries its bracketed code: {message}"
        );
        assert!(
            !comptime_message_has_jargon(&message),
            "the [C0925] message must be free of COMPTIME_JARGON_MARKERS: {message}"
        );
    }
}
