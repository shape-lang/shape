//! ADR-009 C2 #13 (slice 1) — the atomic install transaction for generated
//! bodies.
//!
//! # What this guards
//!
//! A comptime annotation handler can generate `extend Type { method ... }`
//! bodies (and generated free functions). Installing one is a two-phase
//! sequence that spans the whole compilation driver
//! ([`compile_in_place`](crate::compiler::BytecodeCompiler::compile_in_place)):
//! the whole-program pre-pass (`materialize_computed_comptime_extends`)
//! registers the generated signatures into the function table BEFORE the
//! shared analyzer runs, and pass-2 (`apply_comptime_extend`) compiles the
//! bodies AFTER it. A rejection can therefore surface in EITHER phase:
//!
//! - **analysis-time** — the shared analyzer (`analyze_program_full`, run in
//!   `FailFast`/`Strict`) infers the generated body and rejects it (an
//!   undefined callee, a type error). Pass-2 never runs; the pre-pass
//!   registration is the surviving ghost.
//! - **pass-2 body compile** — the body passes analysis but
//!   `analyze_function_body` / emission rejects it (a borrow/mutability error,
//!   an emission failure). Here the `analyze_function_body` fact bundle has
//!   already published before the `Err`.
//!
//! Both reject modes leave the pre-pass registration (and, for the pass-2 mode,
//! the fact bundle) behind unless something rolls them back. Before this slice
//! nothing did: a rejected install left a registered, zero-length ghost
//! function plus a live generated-symbol reservation. This transaction closes
//! that hole at the root — the D8 ruling — with no staging overlay: it records
//! a watermark before the install begins and, on any `Err`, restores exactly
//! the install's additions.
//!
//! # Rollback set
//!
//! Captured as a [watermark](InstallTransaction), never a table clear, so
//! UNRELATED already-installed state (prelude, imports, user functions, other
//! compilations) is left intact:
//!
//! - `program.functions` — truncated to the pre-install length. The truncated
//!   tail's names drive removal from every name-keyed side table
//!   `register_function` and `analyze_function_body` publish under a function
//!   name (see [`rollback`]). Truncation (not remove-by-name) is mandatory:
//!   removing a non-last entry would shift every later `FunctionId`, corrupting
//!   already-emitted `Operand::Function` operands in other bodies.
//! - the `analyze_function_body` fact bundle keyed by function name.
//! - `generated_symbols` reservations.
//!
//! ## Rollback-set correction: `closure_capture_packs` is NEVER rolled back
//!
//! `closure_capture_packs` was initially listed here but is DELIBERATELY not
//! rolled back. It is one member of a ~7-way parallel closure-registry cluster
//! (`closure_type_ids`, `closure_function_ids`, `closure_capture_names`,
//! `function_type_ids` are `Vec`s; `closure_registry`, `function_type_registry`
//! are registry structs). The cluster must stay mutually consistent: truncating
//! only the packs desynchronises it, so a REUSED compiler that re-registers a
//! closure index trips the `closure N has more than one ClosureTypeId entry`
//! uniqueness check (`compiler_impl_reference_model/closure_layouts.rs`). The
//! packs are per-function closure state populated by ALL closures (user and
//! generated), which the per-function reference-flow transaction intentionally
//! leaves as a consistent, harmless ghost — this install-level transaction must
//! not reach into it. Rolling back the whole cluster instead would couple into
//! the registry structs (no truncate API), so the packs are treated as query
//! metadata, not an executable publication, and are never rolled back. A
//! rejected generated closure never reaches the pack push anyway (the capture
//! rejection precedes the push), so no generated ghost pack is produced. (Slice
//! review: the claim "a ghost pack is unreachable from any executable or
//! observable surface post-reject" is the one to attack.)
//!
//! # Query-session retain mode
//!
//! [`retain_generated_reservations_for_query_session`] is a NAMED, explicit
//! mode — not a soft-fail. The LSP generated-symbol/capture query entries
//! (`compile_for_generated_symbol_queries` /
//! `compile_for_generated_capture_queries`) tolerate a recoverable compile
//! `Err` and then answer from the reservation tables: the symbol query reads
//! `generated_symbols`, the capture query reads `closure_capture_packs`
//! (`capture_plan/query.rs`). A query session never executes and never ships a
//! program, so it still rolls back every truly-executable publication
//! (`program.functions`, the name-keyed side tables, the fact bundle). The mode
//! gates exactly ONE table — `generated_symbols` — so it survives for post-`Err`
//! queryability (the tolerance commit `4dce9471` relies on); `closure_capture_packs`
//! survives everywhere by never being rolled back at all (above). Ordinary
//! (batch/install) compilation leaves the mode off and rolls back `generated_symbols`.

use crate::compiler::BytecodeCompiler;

mod rollback;

/// The baseline captured when a generated-body install begins.
///
/// Watermark semantics only: `program.functions` is append-only during an
/// install, so recording its length before the install begins lets rollback
/// truncate exactly the install's additions and keep every pre-existing entry.
/// `generated_symbols` is a name/id map that cannot be truncated; its watermark
/// is a length gate so an install that reserved nothing does not disturb an
/// already-settled table.
///
/// Committing is simply DROPPING this token on the success path — every
/// publication stays. The failure path passes it to
/// [`rollback_checked_body_install`](BytecodeCompiler::rollback_checked_body_install)
/// to restore.
pub(in crate::compiler) struct InstallTransaction {
    functions_watermark: usize,
    generated_symbols_watermark: usize,
}

impl BytecodeCompiler {
    /// Record the pre-install baseline. Called before the generated-body
    /// install begins (the whole-program comptime pre-pass); the returned token
    /// is committed (dropped) on success or rolled back on any `Err`.
    pub(in crate::compiler) fn begin_checked_body_install(&self) -> InstallTransaction {
        InstallTransaction {
            functions_watermark: self.program.functions.len(),
            generated_symbols_watermark: self.generated_symbols.len(),
        }
    }

    /// Failure path: restore the compiler to the pre-install baseline so no
    /// partial generated install is observable.
    ///
    /// Every truly-executable publication rolls back unconditionally. The
    /// `generated_symbols` reservation table rolls back too UNLESS the named
    /// query-session retain mode is set, in which case it survives for
    /// post-`Err` queryability. `closure_capture_packs` is never rolled back in
    /// either mode (see the module docs).
    pub(in crate::compiler) fn rollback_checked_body_install(
        &mut self,
        transaction: InstallTransaction,
    ) {
        self.rollback_executable_publications(&transaction);
        if !self.retain_generated_reservations_for_query_session {
            self.rollback_generated_symbol_reservations(&transaction);
        }
    }
}
