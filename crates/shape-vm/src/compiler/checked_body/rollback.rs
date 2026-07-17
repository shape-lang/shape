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
    /// `hoisted_fields` is DELIBERATELY NOT rolled back here: it is keyed by
    /// binding (local variable) name, not function name, and is co-populated by
    /// the whole-program property-assignment pre-pass, so a function-name-keyed
    /// removal is unsound (a generated body's `let total` shares the key with an
    /// unrelated binding). Rolling it back precisely would couple to the
    /// binding/solver internals beyond this slice's bounded seam; it is left as
    /// a reported residual rather than force a collision-unsafe clear.
    pub(in crate::compiler) fn rollback_executable_publications(
        &mut self,
        transaction: &InstallTransaction,
    ) {
        let removed: Vec<String> = self.program.functions[transaction.functions_watermark..]
            .iter()
            .map(|function| function.name.clone())
            .collect();
        self.program
            .functions
            .truncate(transaction.functions_watermark);

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
    }

    /// Roll back the generated-query reservation tables. Runs ONLY in the
    /// batch/install path; the query-session retain mode skips this so the
    /// tables the LSP answers from survive a recoverable `Err`.
    ///
    /// `closure_capture_packs` (read by `generated_capture_query`) is a
    /// `Vec` truncated to its watermark.
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
    pub(in crate::compiler) fn rollback_generated_query_reservations(
        &mut self,
        transaction: &InstallTransaction,
    ) {
        self.closure_capture_packs
            .truncate(transaction.capture_packs_watermark);
        if self.generated_symbols.len() > transaction.generated_symbols_watermark {
            self.generated_symbols = GeneratedSymbolTable::new();
        }
    }
}
