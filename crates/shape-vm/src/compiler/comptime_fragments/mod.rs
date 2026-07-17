//! ADR-009 E2 #18 (slice 1) — the typed comptime-fragment sink for module
//! replacement.
//!
//! # `CheckedModule<Exports>` — the typed transport for `replace module`
//!
//! A `replace module (expr)` directive reaches the compiler by TWO complete,
//! independent paths that share only the surface spelling:
//!
//! - **legacy (U03)** — `expr` is a `string` of module source (or AST-JSON).
//!   `__emit_replace_module`'s string arm reparses it via
//!   `parse_module_items_payload` and pushes
//!   [`ComptimeDirective::ReplaceModule`](crate::compiler::comptime_builtins::ComptimeDirective::ReplaceModule).
//!   Consumed by the raw `*module_items = items` arm. UNCHANGED until the
//!   slice-5 deletion removes this whole path (the payload parser + the string
//!   producer arm + the directive variant) in ONE commit.
//! - **typed (this slice)** — `expr` is a typed `__ComptimeItemFragment` (e.g.
//!   from `item_fn(...)`). `__emit_replace_module`'s fragment arm converts it to
//!   AST items WITHOUT a source or JSON string ever existing, and pushes
//!   [`ComptimeDirective::ReplaceModuleChecked`](crate::compiler::comptime_builtins::ComptimeDirective::ReplaceModuleChecked).
//!   The module-target consumer routes it through
//!   [`build_checked_module`](crate::compiler::BytecodeCompiler::build_checked_module),
//!   which stamps each item's closures with generated provenance
//!   (`GeneratedNodeIssuer`) and reserves a hygienic export symbol (`SymbolId`)
//!   with exactly the per-item sequence the fresh-generated declaration-discovery
//!   pre-pass runs — producing this `CheckedModule`.
//!
//! # Two complete paths, not a bridge (E2-D8 staging discipline)
//!
//! The two paths never convert into one another: the typed arm never
//! materializes a string, and the legacy arm never reserves a hygienic symbol
//! or stamps provenance. They are two full transports carried by two directive
//! variants, staged side by side per the user-ratified E2-D8 ruling until the
//! slice-5 deletion removes the legacy one WHOLE. This is the ruled staging —
//! NOT the forbidden "keep the source-reparse arm for one case" walk-back
//! (CLAUDE.md §Forbidden rationalizations). The typed arm is self-sufficient
//! today, pinned end-to-end (`tests/annotations_comptime/directives.rs` typed
//! install+run, `bin/shape-cli/tests/cli/jit_c2_install_native.rs` VM+JIT), and
//! the deletion slice deletes the legacy arm without touching it.

use shape_ast::ast::Item;

use crate::compiler::comptime_builtins::expansion_provenance::SymbolId;

/// A comptime-generated module replacement, checked at construction: its items
/// carry generated closure provenance and each generated declaration owns a
/// reserved hygienic export identity (`Exports`). Built ONLY by
/// [`BytecodeCompiler::build_checked_module`](crate::compiler::BytecodeCompiler::build_checked_module)
/// from the typed `ReplaceModuleChecked` route — a source/JSON string never
/// participates.
pub(in crate::compiler) struct CheckedModule {
    /// The provenance-stamped replacement items, ready to become the module's
    /// body (the module-compile flow qualifies + registers them as usual).
    items: Vec<Item>,
    /// The reserved hygienic export identities, one per generated declaration.
    exports: Vec<SymbolId>,
}

impl CheckedModule {
    /// Carry already-stamped items and their reserved export symbols. The
    /// stamping + reservation is the builder's responsibility; this constructor
    /// only carries the result, so the type is never assembled from unstamped
    /// items by a caller that skipped the builder.
    pub(in crate::compiler) fn new(items: Vec<Item>, exports: Vec<SymbolId>) -> Self {
        Self { items, exports }
    }

    /// The provenance-stamped replacement items.
    pub(in crate::compiler) fn items(&self) -> &[Item] {
        &self.items
    }

    /// The reserved hygienic export identities (`Exports`).
    pub(in crate::compiler) fn exports(&self) -> &[SymbolId] {
        &self.exports
    }

    /// Consume into the replacement items for application to the module body.
    pub(in crate::compiler) fn into_items(self) -> Vec<Item> {
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_function_item() -> Item {
        shape_ast::parse_program("fn answer() -> int { 42 }")
            .expect("fixture parses")
            .items
            .into_iter()
            .next()
            .expect("fixture has one item")
    }

    #[test]
    fn into_items_round_trips_the_carried_items() {
        let checked = CheckedModule::new(vec![one_function_item()], Vec::new());
        assert_eq!(checked.items().len(), 1);
        let items = checked.into_items();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Item::Function(..)));
    }

    #[test]
    fn a_pure_constructor_carries_no_forged_exports() {
        // `SymbolId` cannot be minted outside `expansion_provenance`, so a
        // pure-constructor `CheckedModule` carries no exports. The behavioral
        // reservation (a real hygienic export per generated decl) is pinned at
        // the compiler tier by the module-target `replace module (item_fn(...))`
        // install pins; here we only pin that the accessor reflects the carried
        // set faithfully.
        let checked = CheckedModule::new(vec![one_function_item()], Vec::new());
        assert!(checked.exports().is_empty());
    }
}
