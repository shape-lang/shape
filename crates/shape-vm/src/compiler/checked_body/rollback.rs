//! Per-table restore logic for [`InstallTransaction`](super::InstallTransaction).

use super::InstallTransaction;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::GeneratedSymbolTable;

impl BytecodeCompiler {
    /// Roll back every truly-executable publication. Runs in BOTH the batch
    /// and the query-session paths — a query session never executes or ships,
    /// so it still discards these.
    ///
    /// `program.functions` is truncated to the watermark; the truncated tail's
    /// names are exactly the install's decl set (each `register_function`
    /// pushes one entry and keys every side table by that name), so they drive
    /// removal from the name-keyed tables. Removal is bounded to those names,
    /// leaving every pre-existing entry intact.
    ///
    /// # Completing the truncation over the closure-index cluster
    ///
    /// Closures ARE `program.functions` entries, and the closure-registry
    /// cluster is keyed by that function index:
    /// `finalize_closure_function_layouts` keys `packs_by_function` by
    /// `pack.closure` and iterates `closure_type_ids` by function index
    /// (`compiler_impl_reference_model/closure_layouts.rs`). Truncating the
    /// function table above FREES those indices, so any cluster entry left at a
    /// freed index is dangling — and a REUSED compiler that re-registers a
    /// closure at the reused index then holds two entries for the same index,
    /// tripping the "closure N has more than one ClosureTypeId entry" / "more
    /// than one capture pack" uniqueness checks. So the truncation is completed
    /// over its function-index-keyed derivatives here. The interned-identity
    /// registries (`closure_registry`, `function_type_registry`) are keyed by
    /// `ClosureTypeId`, never duplicate, and are looked up (never enumerated for
    /// uniqueness) by finalize, so they need no rollback. `closure_capture_packs`
    /// is completed too, but batch-only, in
    /// [`rollback_query_retained_reservations`] (the LSP capture query reads it).
    ///
    /// `hoisted_fields` is DELIBERATELY NOT rolled back: it is keyed by binding
    /// (local variable) name, not function name, and is co-populated by the
    /// whole-program property-assignment pre-pass, so a function-name-keyed
    /// removal is unsound (a generated body's `let total` shares the key with an
    /// unrelated binding). Rolling it back precisely would couple to the
    /// binding/solver internals beyond this slice's bounded seam; it is left as
    /// a reported residual rather than force a collision-unsafe clear.
    pub(in crate::compiler) fn rollback_executable_publications(
        &mut self,
        transaction: &InstallTransaction,
    ) {
        let watermark = transaction.functions_watermark;
        let removed: Vec<String> = self.program.functions[watermark..]
            .iter()
            .map(|function| function.name.clone())
            .collect();
        self.program.functions.truncate(watermark);

        for name in &removed {
            // Side tables `register_function` publishes.
            self.function_defs.remove(name);
            self.function_arity_bounds.remove(name);
            self.function_const_params.remove(name);
            self.type_tracker.remove_function_return_concrete_type(name);

            // The `analyze_function_body` fact bundle, keyed by function name.
            self.mir_functions.remove(name);
            self.mir_borrow_analyses.remove(name);
            self.mir_storage_plans.remove(name);
            self.mir_field_analyses.remove(name);
            self.mir_span_to_point.remove(name);
            self.function_borrow_summaries.remove(name);
            self.function_return_reference_summaries.remove(name);
        }

        // The function-index-keyed closure derivatives at the freed indices.
        self.closure_type_ids
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
        self.closure_function_ids
            .retain(|(_, function_index)| usize::from(*function_index) < watermark);
        self.closure_capture_names
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
        self.function_type_ids
            .retain(|(function_index, _)| usize::from(*function_index) < watermark);
    }

    /// Roll back the reservation tables the LSP queries read. Runs ONLY in the
    /// batch/install path; the query-session retain mode skips this so those
    /// tables survive a recoverable `Err` for post-`Err` queryability.
    ///
    /// `generated_symbols` (read by `generated_symbol_query`) is reset to empty
    /// — the same quarantine the annotation-declaration transaction's
    /// `poison_annotation_compiler` performs — but ONLY when this install
    /// actually grew the table past its watermark. The table is a name/id map,
    /// not an append-only vector, so it cannot be watermark-truncated; the reset
    /// is sound because it is initialized empty per compilation unit, so an
    /// install that grew it grew it FROM empty and the reset restores that
    /// baseline. Gating on the watermark keeps an early error that reserved
    /// nothing (e.g. a poisoned compiler rejecting at the usability gate) from
    /// mutating an already-settled table.
    ///
    /// `closure_capture_packs` (read by `generated_capture_query`) is completed
    /// here, batch-only: it is the query-metadata member of the closure-index
    /// cluster (the executable members roll back in both modes above). Dropping
    /// the entries at freed function indices keeps batch reuse consistent, while
    /// the query session retains them so the LSP capture query still answers.
    pub(in crate::compiler) fn rollback_query_retained_reservations(
        &mut self,
        transaction: &InstallTransaction,
    ) {
        if self.generated_symbols.len() > transaction.generated_symbols_watermark {
            self.generated_symbols = GeneratedSymbolTable::new();
        }
        let watermark = transaction.functions_watermark;
        self.closure_capture_packs
            .retain(|pack| usize::from(pack.closure) < watermark);
    }
}
