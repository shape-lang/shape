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
//! BLOCK EXPRESSION (`expressions/misc.rs`), a loop / `if` body, or the SAME
//! function's other scopes all register through `track_drop_local` too, so they
//! are covered by construction; a per-copy flag site would silently UNDER-detect
//! every scope but the statement one (unsound — it installs exactly what wave40
//! must not inherit).
//!
//! NESTED-CLOSURE CAVEAT (D6 review finding): a drop local living inside a
//! NESTED CLOSURE is NOT covered by construction — the closure body compiles in
//! its own `compile_function` frame, whose save/restore of the drop flag
//! (functions.rs, stopping monomorphization bleed) also isolates the closure's
//! flag, so a generated body whose drop obligation AND suspension both live
//! inside a nested closure would install unrejected by the enclosing function's
//! gate. This is LATENT: that construct cannot currently compile (it fails
//! Future unification before the borrow check — battery row 7,
//! BLOCKED-BY-INFERENCE). WHEN ROW 7 FLIPS, this case MUST be re-verified; the
//! bounded full fix is to run this gate over the closure itself by threading the
//! closure's generated origin at its compile seam (`expressions/closures.rs`,
//! which already holds it). See `helpers.rs::track_drop_local`.
//!
//! This also catches an INFERRED drop type (`let x = make_droppable()`
//! / `let x = p.acquire()`) an AST annotation scan would miss. Drop-obligated
//! PARAMETERS are resolved separately by their annotation (a parameter is always
//! explicitly typed, so an annotation query is sound — no inference gap).
//!
//! # Placement and provenance gate
//!
//! The gate runs at the END of `compile_function_inner` (functions.rs), on the
//! EFFECTIVE definition (`effective_def`) that actually compiled — the fresh
//! generated body for extend methods / free functions, and the REPLACEMENT body
//! for a `replace body` edit (whose swap lands on `effective_def` mid-compile,
//! never on the outer `func_def`). Evaluating over `effective_def` is what gives
//! `ReplaceBody` edits real coverage: the suspension scan reads the body that
//! ships, not the pre-edit user body. It runs on a body that compiled
//! successfully AND carries AUTHENTICATED generated provenance, so it covers
//! every generated body uniformly. Authentication is the issuer-recognition
//! capability (`GeneratedNodeIssuer::recognizes`, the `capture_plan/surface.rs`
//! pattern), never a name heuristic — the origin is a node-borne
//! `GeneratedNodeOrigin` projected at the generation site (extend/free-fn call
//! site) or at the `replace body` stamp, never re-derived here. A rejection
//! rides the driver-level install transaction's atomic no-publish. A non-async
//! body has no suspension point, so the scan short-circuits it.
//!
//! # UNCOVERED CLASS: generic generated bodies (D6 review finding, Route 2)
//!
//! The gate does NOT cover GENERIC generated bodies (an `extend Vec<T> { async
//! … }` or a generated `async method run<T>(…)`). Two facts combine: (1) a
//! generic template EARLY-RETURNS from `compile_function_with_generated_origin`
//! (functions.rs, the non-empty `type_params` guard) before the gate — so the
//! template body never hits it; (2) its specialization is later compiled by
//! MONOMORPHIZATION via the plain `compile_function` delegate (origin `None`), so
//! the gate short-circuits (not a recognized generated body). A generic
//! generated async body with a drop-obligated local + a suspension therefore
//! INSTALLS UNREJECTED — a fail-OPEN for the whole generic class.
//!
//! This is Route 2 (documented + pinned), not fixed, by supervisor ruling: the
//! bounded fix (store the origin keyed by the template name at registration and
//! re-arm `compile_function_with_generated_origin(Some(origin))` on the
//! specialization compile) has to touch SIX monomorphization compile sites
//! (`monomorphization/cache.rs` ×3 — `ensure_monomorphic_function{,_with_consts,
//! _with_closures}`; `expressions/function_calls.rs` ×3 — `ensure_const_specialization`
//! / `try_specialize_implicit_generic_free_function_call` /
//! `try_specialize_concrete_user_method_call`), each with its OWN substitute →
//! register → compile and no shared helper, and the two specialized-name mangling
//! schemes (`semantic_specialization::keys`) block deriving the base name from a
//! specialized name for a single-point lookup. That exceeds a bounded seam; the
//! uncovered class is pinned as a live tripwire
//! (`c2_slice2_battery_tests::battery_row10b_generic_monomorphization_uncovered_installs`)
//! that FLIPS to `[C0922]` when the re-arm lands.

use shape_ast::ast::{FunctionDef, GeneratedNodeOrigin};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

mod walk;
use walk::body_has_suspension_point;

impl BytecodeCompiler {
    /// The D6 async-drop-context install guard, called at the end of
    /// `compile_function_inner` over the EFFECTIVE definition (see module docs).
    /// `origin` is the node-borne provenance of the body just compiled (`None`
    /// ⇒ not a generated body); `saw_drop_obligated_local` is the
    /// emission-authority signal the RAII drop-plan set during this compile.
    /// `Ok(())` ⇒ not this check's concern; `Err` ⇒ the named installation
    /// rejection.
    pub(in crate::compiler) fn reject_generated_drop_obligated_across_suspension(
        &self,
        func_def: &FunctionDef,
        origin: Option<&GeneratedNodeOrigin>,
        saw_drop_obligated_local: bool,
    ) -> Result<()> {
        // Not a generated body → not policed here.
        let Some(node_origin) = origin else {
            return Ok(());
        };
        // Provenance authentication (surface.rs pattern): only an origin THIS
        // compiler instance issued is a generated body we own. A foreign or
        // serialized origin is not recognized and is left alone.
        if !self.generated_node_issuer.recognizes(node_origin) {
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
    /// point of rejection. The `[C0922]` prefix follows the C1 C09xx convention
    /// (bracketed code at the head of the message, as in `capture_plan.rs`'s
    /// `[C0906]`/`surface.rs`'s `[C0903]`). This is a genuinely NEW install
    /// class (D4): no shipped check combines drop-obligation with suspension, so
    /// there is no underlying analyzer/solver code to reuse.
    ///
    /// The message MUST avoid every `COMPTIME_JARGON_MARKERS` token (helpers.rs)
    /// so the comptime-diagnostics firewall (`sanitize_comptime_internal`) passes
    /// it through the generated-declaration envelope UNCHANGED — as it does for
    /// the `[C0902]` / `[C0923]` semantic errors — instead of replacing it with
    /// the internal-error envelope ("not available in compile-time code"). One of
    /// those markers is a synonym for "reject"; this text uses "rejected".
    fn async_drop_context_rejection(&self) -> ShapeError {
        ShapeError::SemanticError {
            message: "[C0922] generated body holds a drop-obligated value across a suspension \
                      point; installation is rejected pending the AsyncDrop protocol (wave40). \
                      This is a CONSERVATIVE, fail-closed rejection: any drop-obligated local or \
                      parameter plus any suspension point (await / async scope / async let / \
                      join / for-await) in the same generated body is rejected, without liveness \
                      precision — the precise across-suspension analysis is a future refinement."
                .to_string(),
            location: None,
        }
    }
}
