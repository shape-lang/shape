//! ADR-009 C2 #13 (slice 2) — the D6 conservative async-drop-context install
//! rejection (validation-battery row 10b; `C0922`, assigned slice 3). Full
//! rationale + the forward-soundness argument live in [`super::battery`]
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
//! The drop-obligated-LOCAL signal (`current_function_saw_drop_obligated_local`)
//! is defined as "a TYPED scope-exit drop obligation was registered for this
//! function", and it is set at the ONE universal chokepoint every drop-plan copy
//! funnels through: `track_drop_local` (`helpers.rs`) — the sole populator of
//! `drop_locals`, which `emit_drop_call_for_local` drains into every typed
//! scope-exit `DropCall`. The flag is set there gated on `local_drop_kind`
//! resolving (the SAME query the emission uses; `Some` iff the local's stamped
//! type carries a `Drop` impl). Living at that chokepoint — rather than at each
//! `VariableDecl` drop-plan copy — is load-bearing: a drop-obligated local in a
//! BLOCK EXPRESSION (`expressions/misc.rs`), a loop / `if` body, or a CLOSURE
//! body all register through `track_drop_local` too, so they are covered by
//! construction; a per-copy flag site would silently UNDER-detect every scope
//! but the statement one (unsound — it installs exactly what wave40 must not
//! inherit). This also catches an INFERRED drop type (`let x = make_droppable()`
//! / `let x = p.acquire()`) an AST annotation scan would miss. Drop-obligated
//! PARAMETERS are resolved separately by their annotation (a parameter is always
//! explicitly typed, so an annotation query is sound — no inference gap).
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

    /// The named `C0922` rejection (ADR-009 C2 slice 3), naming BOTH facts and
    /// stating the conservatism so the wave40 relaxation is documented at the
    /// point of refusal. The `[C0922]` prefix follows the C1 C09xx convention
    /// (bracketed code at the head of the message, as in `capture_plan.rs`'s
    /// `[C0906]`/`surface.rs`'s `[C0903]`). This is a genuinely NEW install
    /// class (D4): no shipped check combines drop-obligation with suspension, so
    /// there is no underlying analyzer/solver code to reuse.
    fn async_drop_context_rejection(&self) -> ShapeError {
        ShapeError::SemanticError {
            message: "[C0922] generated body holds a drop-obligated value across a suspension \
                      point; installation is rejected pending the AsyncDrop protocol (wave40). \
                      This is a CONSERVATIVE, fail-closed rejection: any drop-obligated local or \
                      parameter plus any suspension point (await / async scope / async let / \
                      join / for-await) in the same generated body is refused, without liveness \
                      precision — the precise across-suspension analysis is wave40's."
                .to_string(),
            location: None,
        }
    }
}
