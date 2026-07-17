//! ADR-009 C2 #13 — the install transaction's displaced-entry undo journal.
//!
//! Removal-by-key is not restoration. The name-keyed side tables and the
//! `generated_symbols` reservation table are NOT append-only: an install can
//! OVERWRITE a shared key that a prelude/dependency (or an earlier successful
//! install on a reused compiler) put there BELOW the watermark
//! (`register_function` skips dedup for `::`/`.` names, and
//! `compile_with_graph_and_prelude` registers dependency symbols first). A
//! bounded `remove(key)` on rollback then deletes the shared key and corrupts
//! that pre-existing state (the review's H1/H2), and a wholesale reset destroys
//! below-watermark reservations.
//!
//! The fix is a journal: while a transaction is live, every keyed write the
//! install performs records its DISPLACED prior value (present -> the value,
//! absent -> `None`) here BEFORE the write; rollback replays in reverse,
//! restoring displaced values and removing keys that were fresh. This subsumes
//! H1 (name-keyed side tables), H2 (`generated_symbols`), M3
//! (`owned_mutable_locals` witness set) and M4 (`hoisted_fields`) in one
//! mechanism.
//!
//! Append-only tables keyed by `program.functions` INDEX (the function table
//! itself and the closure-index cluster) are NOT journaled — they cannot
//! displace a below-watermark entry, so a watermark truncate / index filter
//! (see [`rollback`](super::rollback)) is the right, cheaper restore.
//!
//! Two lists so the query-session retain mode is expressible: `executable`
//! always replays; `query_retained` (the `generated_symbols` reservations the
//! LSP symbol query reads) replays only in batch mode.

use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::{
    CanonicalHash, GeneratedOrigin, SourceAnchor, SymbolId, SymbolReservation,
};
use std::collections::HashMap;

/// One recorded restore action, replayed on rollback to undo one keyed write.
type Undo = Box<dyn FnOnce(&mut BytecodeCompiler)>;

/// The transaction's undo journal (see module docs).
#[derive(Default)]
pub(in crate::compiler) struct InstallJournal {
    executable: Vec<Undo>,
    query_retained: Vec<Undo>,
}

/// Restore one `HashMap<String, V>` entry to its recorded prior state.
fn restore_map<V>(table: &mut HashMap<String, V>, key: &str, prior: Option<V>) {
    match prior {
        Some(value) => {
            table.insert(key.to_string(), value);
        }
        None => {
            table.remove(key);
        }
    }
}

impl BytecodeCompiler {
    #[inline]
    fn install_journal_active(&self) -> bool {
        self.install_journal.is_some()
    }

    fn push_executable_undo(&mut self, undo: Undo) {
        if let Some(journal) = self.install_journal.as_mut() {
            journal.executable.push(undo);
        }
    }

    fn push_query_retained_undo(&mut self, undo: Undo) {
        if let Some(journal) = self.install_journal.as_mut() {
            journal.query_retained.push(undo);
        }
    }

    /// Record the pre-install values of every table `register_function` keys by
    /// function name, so a rollback restores a displaced prelude/dependency
    /// entry (H1) instead of deleting the shared key. Call once before the
    /// registration's inserts; the conditional return-type write is covered too
    /// (an absent prior restores to absent, a no-op).
    pub(in crate::compiler) fn journal_record_register_function(&mut self, name: &str) {
        if !self.install_journal_active() {
            return;
        }
        let key = name.to_string();
        let prior_def = self.function_defs.get(name).cloned();
        let prior_arity = self.function_arity_bounds.get(name).cloned();
        let prior_const = self.function_const_params.get(name).cloned();
        let prior_ct = self
            .type_tracker
            .get_function_return_concrete_type(name)
            .cloned();
        self.push_executable_undo(Box::new(move |c| {
            restore_map(&mut c.function_defs, &key, prior_def);
            restore_map(&mut c.function_arity_bounds, &key, prior_arity);
            restore_map(&mut c.function_const_params, &key, prior_const);
            match prior_ct {
                Some(ct) => c
                    .type_tracker
                    .register_function_return_concrete_type(&key, ct),
                None => c.type_tracker.remove_function_return_concrete_type(&key),
            }
        }));
    }

    /// Record the pre-install values of the `analyze_function_body` fact bundle
    /// (all seven maps, keyed by function name). Call once before the body
    /// analysis publishes; both the unconditional inserts and the conditional
    /// insert-or-remove tables are covered by restoring the recorded prior.
    pub(in crate::compiler) fn journal_record_analyze_function_body(&mut self, name: &str) {
        if !self.install_journal_active() {
            return;
        }
        let key = name.to_string();
        let prior_functions = self.mir_functions.get(name).cloned();
        let prior_borrows = self.mir_borrow_analyses.get(name).cloned();
        let prior_storage = self.mir_storage_plans.get(name).cloned();
        let prior_fields = self.mir_field_analyses.get(name).cloned();
        let prior_span = self.mir_span_to_point.get(name).cloned();
        let prior_borrow_summary = self.function_borrow_summaries.get(name).cloned();
        let prior_return_ref = self.function_return_reference_summaries.get(name).cloned();
        self.push_executable_undo(Box::new(move |c| {
            restore_map(&mut c.mir_functions, &key, prior_functions);
            restore_map(&mut c.mir_borrow_analyses, &key, prior_borrows);
            restore_map(&mut c.mir_storage_plans, &key, prior_storage);
            restore_map(&mut c.mir_field_analyses, &key, prior_fields);
            restore_map(&mut c.mir_span_to_point, &key, prior_span);
            restore_map(&mut c.function_borrow_summaries, &key, prior_borrow_summary);
            restore_map(
                &mut c.function_return_reference_summaries,
                &key,
                prior_return_ref,
            );
        }));
    }

    /// Record the pre-install value of the two field-hoisting tables for one
    /// BINDING name before either is (over)written, so a later same-named
    /// binding in a reused compiler does not read a ghost hoist (M4). Call
    /// before the insert. `hoisted_field_types` is co-keyed by the same name
    /// and rolled back with it (an unchanged table's restore is a no-op).
    pub(in crate::compiler) fn journal_record_hoisted_field(&mut self, var_name: &str) {
        if !self.install_journal_active() {
            return;
        }
        let key = var_name.to_string();
        let prior_fields = self.hoisted_fields.get(var_name).cloned();
        let prior_types = self.hoisted_field_types.get(var_name).cloned();
        self.push_executable_undo(Box::new(move |c| {
            restore_map(&mut c.hoisted_fields, &key, prior_fields);
            restore_map(&mut c.hoisted_field_types, &key, prior_types);
        }));
    }

    /// Record that an install may add `name` to the `owned_mutable_locals`
    /// witness set, so a rollback removes the ghost that would otherwise
    /// misclassify a later same-named binding (M3). Call before the insert.
    pub(in crate::compiler) fn journal_record_owned_mutable_local(&mut self, name: &str) {
        if !self.install_journal_active() {
            return;
        }
        let key = name.to_string();
        let was_present = self.owned_mutable_locals.contains(name);
        self.push_executable_undo(Box::new(move |c| {
            if was_present {
                c.owned_mutable_locals.insert(key);
            } else {
                c.owned_mutable_locals.remove(&key);
            }
        }));
    }

    /// Record a `Fresh` generated-symbol reservation so a BATCH rollback removes
    /// EXACTLY it — never a below-watermark reservation (H2). Goes on the
    /// query-retained list so the query-session retain mode keeps the
    /// reservation alive for the LSP symbol query. Call in the `Fresh` arm.
    pub(in crate::compiler) fn journal_generated_reservation(
        &mut self,
        decl_name: &str,
        id: SymbolId,
    ) {
        if !self.install_journal_active() {
            return;
        }
        let key = decl_name.to_string();
        self.push_query_retained_undo(Box::new(move |c| {
            c.generated_symbols.remove_reservation(&key, id);
        }));
    }

    /// Reserve a generated declaration AND journal a `Fresh` reservation in one
    /// place, so every generated-install reserve site is covered by the H2 fix.
    /// A batch rollback then removes exactly the reservations this install made,
    /// never a below-watermark one. Reissued reservations and reservation errors
    /// change nothing to undo.
    pub(in crate::compiler) fn reserve_generated_decl_journaled(
        &mut self,
        decl_name: &str,
        origin: GeneratedOrigin,
        content: CanonicalHash,
        generator_anchor: SourceAnchor,
    ) -> Result<SymbolReservation, String> {
        let reservation = self.generated_symbols.reserve_generated_decl(
            decl_name,
            origin,
            content,
            generator_anchor,
        )?;
        if let SymbolReservation::Fresh(id) = reservation {
            self.journal_generated_reservation(decl_name, id);
        }
        Ok(reservation)
    }

    /// Replay the executable journal in reverse (last write undone first, so the
    /// earliest displaced prior wins). Runs on every rollback.
    pub(in crate::compiler) fn replay_executable_journal(&mut self, journal: &mut InstallJournal) {
        while let Some(undo) = journal.executable.pop() {
            undo(self);
        }
    }

    /// Replay the query-retained journal in reverse. Runs on batch rollback only;
    /// the query-session retain mode leaves these reservations in place.
    pub(in crate::compiler) fn replay_query_retained_journal(
        &mut self,
        journal: &mut InstallJournal,
    ) {
        while let Some(undo) = journal.query_retained.pop() {
            undo(self);
        }
    }
}
