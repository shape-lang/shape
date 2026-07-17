//! ADR-009 C2 #13 (slice 2) — the D6 conservative async-drop-context install
//! rejection (validation-battery row 10b; slice 3 assigns the `C0922` code).
//! Full rationale + the forward-soundness argument live in [`super::battery`]
//! (row 10b); this file is the gate, and [`walk`] is its suspension scan.
//!
//! CONSERVATIVE, no liveness precision (D6, supervisor-ruled): a GENERATED body
//! that contains BOTH a drop-obligated binding AND any suspension point
//! (`await`, `async scope`, `async let`, `join`, or a `for await` loop) is
//! REJECTED at install. It does NOT prove the value is live at the suspension
//! point. This over-rejects and never installs unsoundly (C2-R6); wave40's
//! `AsyncDrop` protocol supplies the precision and RELAXES these rejections. A
//! NAMED install rejection, never a soft-fail or runtime fallback.
//!
//! # Drop-obligation is read from the EMISSION AUTHORITY, not an AST scan
//!
//! The drop-obligated-LOCAL signal (`saw_drop_obligated_local`) is set by the
//! RAII drop-plan during `compile_function` (`statements.rs`, where
//! `local_drop_kind` / annotation / initializer-call-return resolve `drop_kind`
//! — the SAME query the drop emission uses). This catches an INFERRED drop type
//! (`let x = make_droppable()`), which an AST annotation scan would
//! UNDER-detect — and under-detection here is unsound (it installs exactly what
//! wave40 must not inherit), not conservative. Drop-obligated PARAMETERS are
//! resolved separately by their annotation (a parameter is always explicitly
//! typed, so an annotation query is sound — no inference gap).
//!
//! # Placement and provenance gate
//!
//! The gate runs at `compile_function`'s end (functions.rs), on a body that
//! compiled successfully AND carries AUTHENTICATED generated provenance, so it
//! covers every generated body that reaches `compile_function` uniformly
//! (fresh-body extend methods + generated free functions). Authentication is
//! the issuer-recognition capability (`GeneratedNodeIssuer::recognizes`, the
//! `capture_plan/surface.rs` pattern), never a name heuristic. A rejection
//! rides the driver-level install transaction's atomic no-publish. A non-async
//! body has no suspension point, so the scan short-circuits it.

use shape_ast::ast::FunctionDef;
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::GeneratedOrigin;

mod walk;
use walk::body_has_suspension_point;

impl BytecodeCompiler {
    /// The D6 async-drop-context install guard, called at `compile_function`'s
    /// end (see module docs). `origin` is the provenance of the body just
    /// compiled (`None` ⇒ not a generated body); `saw_drop_obligated_local` is
    /// the emission-authority signal the RAII drop-plan set during this compile.
    /// `Ok(())` ⇒ not this check's concern; `Err` ⇒ the named installation
    /// rejection.
    pub(in crate::compiler) fn reject_generated_drop_obligated_across_suspension(
        &self,
        func_def: &FunctionDef,
        origin: Option<&GeneratedOrigin>,
        saw_drop_obligated_local: bool,
    ) -> Result<()> {
        // Not a generated body → not policed here.
        let Some(origin) = origin else {
            return Ok(());
        };
        // Provenance authentication (surface.rs pattern): only an origin THIS
        // compiler instance issued is a generated body we own. A foreign or
        // serialized origin is not recognized and is left alone.
        let node_origin = origin.to_node_origin(&self.generated_node_issuer, &func_def.name);
        if !self.generated_node_issuer.recognizes(&node_origin) {
            return Ok(());
        }

        // Drop obligation: a drop-obligated LOCAL (from the emission authority,
        // so an inferred drop type is caught) OR a drop-obligated PARAMETER
        // (always annotated — an annotation query is sound, no inference gap).
        let saw_drop_obligated = saw_drop_obligated_local
            || func_def.params.iter().any(|param| {
                param
                    .type_annotation
                    .as_ref()
                    .is_some_and(|annotation| self.annotation_drop_kind(annotation).is_some())
            });
        if !saw_drop_obligated {
            return Ok(());
        }

        // Suspension point? (A sync body has none, so this short-circuits it.)
        if !body_has_suspension_point(&func_def.body) {
            return Ok(());
        }

        Err(self.async_drop_context_rejection())
    }

    /// The named (code-free this slice; slice 3 assigns `C0922`) rejection,
    /// naming BOTH facts and stating the conservatism so the wave40 relaxation
    /// is documented at the point of refusal.
    fn async_drop_context_rejection(&self) -> ShapeError {
        ShapeError::SemanticError {
            message: "generated body holds a drop-obligated value across a suspension point; \
                      installation is rejected pending the AsyncDrop protocol (wave40). This is \
                      a CONSERVATIVE, fail-closed rejection: any drop-obligated local or \
                      parameter plus any suspension point (await / async scope / async let / \
                      join / for-await) in the same generated body is refused, without liveness \
                      precision — the precise across-suspension analysis is wave40's."
                .to_string(),
            location: None,
        }
    }
}
