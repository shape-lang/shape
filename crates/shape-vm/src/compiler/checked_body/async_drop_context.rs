//! ADR-009 C2 #13 (slice 2) — the D6 conservative async-drop-context install
//! rejection (validation-battery row 10b; slice 3 assigns the `C0922` code).
//! Full rationale + the forward-soundness argument live in [`super::battery`]
//! (row 10b); this file is the check + hook, and [`walk`] is its exhaustive
//! read-only body traversal.
//!
//! CONSERVATIVE, no liveness precision (D6, supervisor-ruled): a GENERATED body
//! that contains BOTH a drop-obligated binding — a local or parameter whose
//! type carries a `Drop` impl (`drop_type_info` names it; the same
//! drop-obligation query the emission drop-plan resolves through
//! `local_drop_kind` / `annotation_drop_kind` / `initializer_call_return_drop_type`)
//! — AND any suspension point (`await`, `async scope`, `async let`, `join`, or
//! a `for await` loop) is REJECTED at install. It does NOT prove the value is
//! live at the suspension point: any drop-obligated binding plus any suspension
//! point in the same generated body rejects. This over-rejects and never
//! installs unsoundly (C2-R6); wave40's `AsyncDrop` protocol supplies the
//! precision and RELAXES these rejections. It is a NAMED install rejection,
//! never a soft-fail or runtime fallback.
//!
//! Placement: the hook runs at the pass-2 generated-body compile site, just
//! before `compile_function`, so a rejection rolls back atomically through the
//! slice-1 install transaction (the pass-2-reject path). It is gated on
//! AUTHENTICATED generated provenance via the issuer-recognition capability
//! (`GeneratedNodeIssuer::recognizes`, the `capture_plan/surface.rs` pattern),
//! never a name heuristic. A sync body has no suspension point and a program
//! with no `impl Drop` has no drop-obligated type, so both are structural
//! no-ops that keep non-async / non-Drop programs byte-identical.

use shape_ast::ast::FunctionDef;
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::GeneratedOrigin;

mod walk;

impl BytecodeCompiler {
    /// The D6 conservative async-drop-context install guard for a generated
    /// body (see module docs). `Ok(())` means "not this check's concern" (sync
    /// body, no `Drop` type, or no drop-obligated binding across a suspension);
    /// `Err` is the named installation rejection.
    ///
    /// `origin` is the freshly-issued expansion provenance of the body being
    /// installed; the gate authenticates it against this compiler instance's
    /// issuer capability before policing anything (never a name heuristic).
    pub(in crate::compiler) fn reject_generated_drop_obligated_across_suspension(
        &self,
        func_def: &FunctionDef,
        origin: &GeneratedOrigin,
    ) -> Result<()> {
        // Provenance authentication (surface.rs pattern): only an origin THIS
        // compiler instance issued is a generated body we own. A foreign or
        // serialized origin is not recognized and is left alone.
        let node_origin = origin.to_node_origin(&self.generated_node_issuer, &func_def.name);
        if !self.generated_node_issuer.recognizes(&node_origin) {
            return Ok(());
        }
        // Structural no-ops: an `await` outside an async function is already a
        // compile error, and a program with no `impl Drop` has no drop-obligated
        // type. Both keep non-async / non-Drop programs byte-identical.
        if !func_def.is_async || self.drop_type_info.is_empty() {
            return Ok(());
        }

        let mut scan = AsyncDropContextScan::new(self);
        scan.note_params(&func_def.params);
        scan.walk_statements(&func_def.body);

        match (scan.saw_suspension, scan.drop_obligated_type) {
            (true, Some(type_name)) => Err(self.async_drop_context_rejection(&type_name)),
            _ => Ok(()),
        }
    }

    /// The named (code-free this slice; slice 3 assigns `C0922`) rejection,
    /// naming BOTH facts and stating the conservatism so the wave40 relaxation
    /// is documented at the point of refusal.
    fn async_drop_context_rejection(&self, type_name: &str) -> ShapeError {
        ShapeError::SemanticError {
            message: format!(
                "generated body holds a drop-obligated value of type `{type_name}` across a \
                 suspension point; installation is rejected pending the AsyncDrop protocol \
                 (wave40). This is a CONSERVATIVE, fail-closed rejection: any drop-obligated \
                 local or parameter plus any suspension point (await / async scope / async let \
                 / join / for-await) in the same generated body is refused, without liveness \
                 precision — the precise across-suspension analysis is wave40's."
            ),
            location: None,
        }
    }
}

/// Single-pass read-only scan collecting the two D6 facts over a generated
/// body: whether it contains a suspension point, and (the type name of) a
/// drop-obligated local/parameter if one exists. Drop-obligation is resolved
/// through the SAME queries the emission drop-plan uses, so the two agree. The
/// traversal itself is the exhaustive [`walk`] child module.
struct AsyncDropContextScan<'c> {
    compiler: &'c BytecodeCompiler,
    saw_suspension: bool,
    drop_obligated_type: Option<String>,
}
