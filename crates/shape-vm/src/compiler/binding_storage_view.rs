//! ADR-017 §2 / R23 — the compiler's own per-binding storage decision,
//! published as a read-only projection surface.
//!
//! Tooling must not re-derive `BindingStorageClass`. Before this table
//! existed the LSP rendered a storage-class inlay hint from an LSP-side
//! heuristic over the declared type and `is_mut`, which could — and did —
//! disagree with the planner, and could name classes
//! (`SharedAtomic`, `SharedAtomicMut`) the compiler has no variant for. The
//! hint now READS this table, recorded by the compiler at the same
//! declaration sites whose codegen consults the decision, so a hint that
//! disagrees with the planner is a test failure rather than the reader's
//! problem.
//!
//! The table is render-only in the same sense as
//! `tools/shape-lsp/src/expansion_views.rs`: it is derived from compiler
//! state, is never read back by compilation, and grants no authority. It is
//! a `Vec` in declaration order — a decision surface that reached user-
//! visible output through unordered-container iteration would be
//! nondeterministic.

use shape_ast::ast::{Span, VarKind};

use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};

/// One declared binding's storage decision, as the compiler decided it.
///
/// Destructuring patterns contribute one entry per bound name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingStorageDecision {
    /// The bound name.
    pub name: String,
    /// The bound name's span in the declaring source. `Span::DUMMY` for
    /// compiler-generated declarations; consumers that map to source
    /// positions must skip those.
    pub name_span: Span,
    /// `let` / `var` / `const` as written.
    pub var_kind: VarKind,
    /// `let mut` as written.
    pub is_mut: bool,
    /// The ownership class the declaration form implies (`var` → `Flexible`).
    pub ownership_class: BindingOwnershipClass,
    /// The storage class codegen reads for this binding — the MIR storage
    /// plan's verdict when the binding is a MIR-planned function local, and
    /// the type tracker's binding semantics otherwise. That consult order is
    /// the compiler's own (`compiler/expressions/identifiers.rs`, the
    /// storage-plan-aware load decision).
    pub storage_class: BindingStorageClass,
    /// The enclosing function's semantic owner key; `None` at module scope.
    pub owner: Option<String>,
    /// The declaring slot, and whether it is a function local or a module
    /// binding. Together with `owner` this is the key a later storage-class
    /// promotion updates through.
    slot: u16,
    is_local: bool,
}

impl BindingStorageDecision {
    /// The slot this decision was recorded for.
    pub fn slot(&self) -> u16 {
        self.slot
    }

    /// True when the decision is for a function local rather than a module
    /// binding.
    pub fn is_local(&self) -> bool {
        self.is_local
    }
}

/// Every binding storage decision the compiler made, in declaration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingStorageTable {
    decisions: Vec<BindingStorageDecision>,
}

impl BindingStorageTable {
    /// Every recorded decision, in declaration order.
    pub fn decisions(&self) -> &[BindingStorageDecision] {
        &self.decisions
    }

    /// The decision recorded for the binding whose name span starts at
    /// `offset`. Declarations with a dummy span never match.
    pub fn at_name_offset(&self, offset: usize) -> Option<&BindingStorageDecision> {
        self.decisions
            .iter()
            .find(|decision| !decision.name_span.is_dummy() && decision.name_span.start == offset)
    }

    /// Record a decision, replacing any earlier entry for the same declaration
    /// site and name. Replacement keeps the original position so the table
    /// stays in declaration order across a re-compiled body (annotation
    /// re-emission, monomorphization).
    pub(crate) fn record(&mut self, decision: BindingStorageDecision) {
        let existing = self.decisions.iter_mut().find(|recorded| {
            recorded.name == decision.name
                && recorded.owner == decision.owner
                && recorded.name_span == decision.name_span
        });
        match existing {
            Some(slot) => *slot = decision,
            None => self.decisions.push(decision),
        }
    }

    /// Apply a later storage-class promotion to the innermost live decision
    /// recorded for `slot` in `owner`'s scope. The compiler promotes a
    /// `var`'s class after its declaration site when it observes an alias or
    /// a closure capture, so a def-site-only snapshot would publish a stale
    /// class for exactly the bindings this table exists to explain.
    pub(crate) fn apply_promotion(
        &mut self,
        owner: Option<&str>,
        slot: u16,
        is_local: bool,
        storage_class: BindingStorageClass,
    ) {
        let target = self.decisions.iter_mut().rev().find(|decision| {
            decision.slot == slot
                && decision.is_local == is_local
                && decision.owner.as_deref() == owner
        });
        if let Some(decision) = target {
            decision.storage_class = storage_class;
        }
    }

    /// Drop every decision recorded for `owner`. Used when a function body's
    /// compilation is rolled back, so the table never publishes decisions for
    /// a body that was discarded.
    pub(crate) fn forget_owner(&mut self, owner: &str) {
        self.decisions
            .retain(|decision| decision.owner.as_deref() != Some(owner));
    }
}

/// Build a decision record. Kept here rather than at the recording site so
/// every field's meaning lives next to the type.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decision(
    name: String,
    name_span: Span,
    var_kind: VarKind,
    is_mut: bool,
    ownership_class: BindingOwnershipClass,
    storage_class: BindingStorageClass,
    owner: Option<String>,
    slot: u16,
    is_local: bool,
) -> BindingStorageDecision {
    BindingStorageDecision {
        name,
        name_span,
        var_kind,
        is_mut,
        ownership_class,
        storage_class,
        owner,
        slot,
        is_local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize) -> Span {
        Span {
            start,
            end: start + 1,
        }
    }

    fn entry(name: &str, start: usize, class: BindingStorageClass) -> BindingStorageDecision {
        decision(
            name.to_string(),
            span(start),
            VarKind::Var,
            false,
            BindingOwnershipClass::Flexible,
            class,
            Some("f".to_string()),
            1,
            true,
        )
    }

    #[test]
    fn decisions_keep_declaration_order() {
        let mut table = BindingStorageTable::default();
        table.record(entry("a", 10, BindingStorageClass::Direct));
        table.record(entry("b", 20, BindingStorageClass::Direct));
        table.record(entry("c", 30, BindingStorageClass::Direct));
        let names: Vec<&str> = table
            .decisions()
            .iter()
            .map(|decision| decision.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn re_recording_a_site_replaces_in_place_rather_than_appending() {
        let mut table = BindingStorageTable::default();
        table.record(entry("a", 10, BindingStorageClass::Direct));
        table.record(entry("b", 20, BindingStorageClass::Direct));
        table.record(entry("a", 10, BindingStorageClass::SharedCow));
        assert_eq!(table.decisions().len(), 2, "the re-record replaced");
        assert_eq!(table.decisions()[0].name, "a", "order is preserved");
        assert_eq!(
            table.decisions()[0].storage_class,
            BindingStorageClass::SharedCow
        );
    }

    #[test]
    fn a_promotion_updates_the_innermost_decision_for_the_slot() {
        let mut table = BindingStorageTable::default();
        // Two block scopes reusing slot 1 inside the same function.
        table.record(entry("outer", 10, BindingStorageClass::Direct));
        table.record(entry("inner", 20, BindingStorageClass::Direct));
        table.apply_promotion(Some("f"), 1, true, BindingStorageClass::SharedCow);
        assert_eq!(
            table.decisions()[0].storage_class,
            BindingStorageClass::Direct,
            "the shadowed outer binding is untouched"
        );
        assert_eq!(
            table.decisions()[1].storage_class,
            BindingStorageClass::SharedCow,
            "the innermost live binding takes the promotion"
        );
    }

    #[test]
    fn a_promotion_in_another_owner_scope_matches_nothing() {
        let mut table = BindingStorageTable::default();
        table.record(entry("a", 10, BindingStorageClass::Direct));
        table.apply_promotion(Some("g"), 1, true, BindingStorageClass::SharedCow);
        assert_eq!(
            table.decisions()[0].storage_class,
            BindingStorageClass::Direct
        );
    }

    #[test]
    fn forgetting_an_owner_drops_only_that_owners_decisions() {
        let mut table = BindingStorageTable::default();
        table.record(entry("a", 10, BindingStorageClass::Direct));
        let mut module_entry = entry("m", 30, BindingStorageClass::Direct);
        module_entry.owner = None;
        module_entry.is_local = false;
        table.record(module_entry);
        table.forget_owner("f");
        let names: Vec<&str> = table
            .decisions()
            .iter()
            .map(|decision| decision.name.as_str())
            .collect();
        assert_eq!(names, ["m"]);
    }

    #[test]
    fn a_dummy_span_declaration_is_never_addressable_by_offset() {
        let mut table = BindingStorageTable::default();
        let mut generated = entry("g", 0, BindingStorageClass::Direct);
        generated.name_span = Span::DUMMY;
        table.record(generated);
        assert!(table.at_name_offset(Span::DUMMY.start).is_none());
    }
}
