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
//! - the closure-index cluster derived from `program.functions` position (see
//!   next section).
//! - `generated_symbols` reservations.
//!
//! ## Completing the truncation over the closure-index cluster
//!
//! Closures ARE `program.functions` entries, and a cluster of tables is keyed
//! by that function index — `closure_type_ids`, `closure_function_ids`,
//! `closure_capture_names`, `function_type_ids`, and `closure_capture_packs`
//! (`finalize_closure_function_layouts` keys packs by `pack.closure` and
//! iterates `closure_type_ids` by function index,
//! `compiler_impl_reference_model/closure_layouts.rs`). Because these are
//! DERIVED from the function-table position, truncating `program.functions`
//! leaves them dangling: a REUSED compiler re-registers a closure at the freed
//! index and then holds two cluster entries for that index, tripping the
//! "closure N has more than one ClosureTypeId entry" / "more than one capture
//! pack" uniqueness checks. (At base ff715e61 there was NO clearing — a failed
//! compile simply leaked its closure function entry, keeping the index space
//! monotonic across reuse; the leak is the very ghost C2 removes, so completing
//! the truncation is how reuse-consistency is preserved without the leak.) So
//! the truncation is completed over these function-index-keyed derivatives,
//! dropping entries at index >= watermark. The interned-identity registries
//! (`closure_registry`, `function_type_registry`) are keyed by `ClosureTypeId`,
//! never duplicate, and are looked up (never enumerated for uniqueness) by
//! finalize, so they need no rollback. `closure_capture_packs` is completed
//! batch-only (it is the cluster's query-metadata member — the LSP capture
//! query reads it; see below).
//!
//! ### Why these five tables are the complete set
//!
//! A table re-hides this bug iff it is APPEND-accumulated across compiles AND
//! keyed by function index. Every append of a function-index entry in the whole
//! compiler is the one closure-registration block (`expressions/closures.rs`,
//! the five `.push((func_idx as u16, …))` / `.push((…, func_idx as u16))` at
//! lines 3539/3542/3547/3563/3576) — exactly these five tables. The other
//! function-index-shaped fields are self-healing and correctly excluded:
//! `function_hashes_by_id` is index-ASSIGNED (overwrite, not append) and
//! reconciled to `program.functions.len()` at compile end
//! (`compiler_impl_reference_model.rs`), so a reused index overwrites rather
//! than duplicates; `program.closure_function_layouts` is REBUILT wholesale each
//! `finalize` (never accumulated); the two registries are `ClosureTypeId`-keyed,
//! not function-index-keyed. Every other `u16`-keyed compiler table is keyed by
//! a LOCAL slot or MODULE-BINDING index, not a function index. The per-function
//! capture EVIDENCE (`mutable_closure_captures`, `shared_closure_captures`,
//! `owned_mutable_closure_captures`, `pending_closure_capture_parameter_evidence`)
//! is already restored by the reference-flow transaction on the reject path
//! (the test's earlier asserts pin exactly that).
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
//! (`program.functions`, the name-keyed side tables, the fact bundle, and the
//! EXECUTABLE closure-index members `closure_type_ids` / `closure_function_ids`
//! / `closure_capture_names` / `function_type_ids`, which no query reads). The
//! mode gates the two tables the LSP DOES read — `generated_symbols` and
//! `closure_capture_packs` — so they survive for post-`Err` queryability (the
//! tolerance commit `4dce9471` relies on). Ordinary (batch/install) compilation
//! leaves the mode off and rolls back both.
//!
//! ## The query-mode asymmetry and why it is safe
//!
//! In query mode this retains `closure_capture_packs` while its executable
//! cluster siblings (`closure_type_ids` etc.) are truncated — a deliberate
//! asymmetry that would be a cluster desync on the BATCH reuse path but is safe
//! here. A generated-symbol/capture query runs on a FRESH compiler per request
//! (`generated_query_compiler`); it never reuses a compiler across compiles, so
//! the reuse pattern that turns dangling entries into duplicates never occurs.
//! `finalize_closure_function_layouts` — the only consumer that joins packs to
//! `closure_type_ids` / `program.functions` — runs during compilation, before
//! this rollback, and does not run again on the query path. The capture query
//! (`capture_plan/query.rs`) reads only `pack.origin` / `pack.descriptors`,
//! never `pack.closure` as an index into `program.functions` or the siblings,
//! so a retained pack whose function index was truncated still answers
//! correctly. The batch path rolls back all cluster members together and stays
//! fully consistent.
//!
//! ## Existing-body edits need no query-retain extension (C2 #13 slice 6)
//!
//! A `replace body` EDIT (slice 4) publishes its provenance and its
//! replacement's closure capture PACKS through the SAME two tables this retain
//! mode preserves — `generated_symbols` (the shadow + any generated
//! reservations) and `closure_capture_packs` (the replacement's declared-capture
//! packs) — so the shared LSP capture/symbol query OBSERVES an edited body's
//! captures on the recoverable-`Err` path with NO edit-specific retain logic. The
//! edit is not a second query surface: it lands in the C1 carrier and the C1
//! tables, and this mode gates exactly those.
//!
//! Named finding (E2 candidate, reported for the C2 close): an edited closure's
//! capture RESOLVES to a `[C0911]` MissingInferenceFact quarantine rather than an
//! exact semantic identity, because the structural inference facts that back
//! specialization identity are recorded by the inference engine at ANALYSIS time
//! and a `replace body` replacement is swapped at PASS-2 — so the analyzer never
//! sees the replacement's closure and no fact is published. This is orthogonal to
//! the retain mode (the pack IS retained) and to codegen (the `CaptureKind`
//! lowering is declared-mode-driven, unaffected); the fix is pre-analysis
//! materialization of directive-edited bodies (E2), not a bounded C2 patch.
//! Pinned by
//! `generated_captures::semantic_tests::replace_body_edit_capture_is_observed_but_specialization_quarantined`.
//!
//! # Caveat: rollback restores to COMPILE-START, not a partial-recovery point
//!
//! On `Err` this restores every rolled-back table to its value when the
//! transaction began — the whole compilation's start — NOT to some
//! partially-recovered state. Under `TypeDiagnosticMode::Strict` /
//! `CompileDiagnosticMode::FailFast` (the ship path) that is exactly right: a
//! failed compile ships nothing, so compile-start IS the correct end state. It
//! matters only under `RecoverAll`, where a caller might want the
//! partially-published facts of a recovered compile; there, an install rollback
//! discards them for the rolled-back tables. That mode is LSP-only today (the
//! query/diagnostics sessions), and those sessions read the reservation tables
//! the retain mode keeps — so the caveat is currently benign, but it is a real
//! constraint if `RecoverAll` ever feeds an executing program.

use crate::compiler::BytecodeCompiler;

/// Slice 2 — the §4.2 validation-battery manifest (which check runs where for
/// generated bodies, inside this transaction's span).
pub(in crate::compiler) mod battery;
/// Slice 2 — the D6 async-drop-context install rejection (battery row 10b), the
/// battery's one greenfield check. Wired at the generated-body compile site.
mod async_drop_context;
/// Slice 4 — the D7 edit-transaction shape guards (`[C0924]` split/two-identity,
/// `[C0925]` incomplete environment). Wired at the `replace body` commit seam as
/// defense-in-depth (the failure branches are structurally unreachable today).
mod edit_transaction_guards;
mod journal;
mod rollback;

pub(in crate::compiler) use journal::InstallJournal;

/// The baseline captured when a generated-body install begins.
///
/// Two rollback mechanisms cooperate. Append-only tables keyed by
/// `program.functions` INDEX (the function table and the closure-index cluster)
/// can never displace a below-watermark entry, so a length WATERMARK is
/// sufficient and cheapest — recorded here. Tables keyed by a NAME that an
/// install can OVERWRITE (side tables, the fact bundle, `generated_symbols`,
/// `owned_mutable_locals`, `hoisted_fields`) are restored by the displaced-entry
/// undo [`journal`](journal), populated at the write sites while the transaction
/// is live (`self.install_journal`).
///
/// Committing is simply DROPPING this token and clearing the journal on the
/// success path — every publication stays. The failure path passes it to
/// [`rollback_checked_body_install`](BytecodeCompiler::rollback_checked_body_install)
/// to restore.
pub(in crate::compiler) struct InstallTransaction {
    functions_watermark: usize,
}

impl BytecodeCompiler {
    /// Record the pre-install baseline and open the undo journal. Called before
    /// the generated-body install begins (the whole-program comptime pre-pass);
    /// the returned token is committed (dropped) on success or rolled back on
    /// any `Err`. Every keyed install write from now until commit/rollback
    /// records its displaced prior into `self.install_journal`.
    pub(in crate::compiler) fn begin_checked_body_install(&mut self) -> InstallTransaction {
        self.install_journal = Some(InstallJournal::default());
        InstallTransaction {
            functions_watermark: self.program.functions.len(),
        }
    }

    /// Failure path: restore the compiler to the pre-install baseline so no
    /// partial generated install is observable.
    ///
    /// The undo journal replays first, restoring every displaced name-keyed
    /// entry and removing fresh ones; then the index-keyed Vec tables truncate
    /// back to the watermark. The executable journal + the index tables restore
    /// in BOTH modes. The query-retained journal (`generated_symbols`
    /// reservations) and `closure_capture_packs` roll back too UNLESS the named
    /// query-session retain mode is set, in which case they survive for
    /// post-`Err` queryability (see the module docs).
    pub(in crate::compiler) fn rollback_checked_body_install(
        &mut self,
        transaction: InstallTransaction,
    ) {
        let retain = self.retain_generated_reservations_for_query_session;
        if let Some(mut journal) = self.install_journal.take() {
            self.replay_executable_journal(&mut journal);
            if !retain {
                self.replay_query_retained_journal(&mut journal);
            }
        }
        self.rollback_indexed_publications(&transaction);
        if !retain {
            self.rollback_capture_packs(&transaction);
        }
    }
}
