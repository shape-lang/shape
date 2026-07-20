//! ADR-009 E2 #18 — the typed comptime-fragment sink for module replacement.
//!
//! # `CheckedModule<Exports>` — the typed transport for `replace module`
//!
//! A `replace module (expr)` directive reaches the compiler through ONE typed
//! path: `expr` evaluates in the comptime VM to a typed generation carrier
//! (`item_fn(...)` -> `__CheckedItem`). `__emit_replace_module` converts it to
//! AST items WITHOUT a source or JSON string ever existing, and pushes
//! [`ComptimeDirective::ReplaceModuleChecked`](crate::compiler::comptime_builtins::ComptimeDirective::ReplaceModuleChecked).
//! The module-target consumer routes it through
//! [`build_checked_module`](crate::compiler::BytecodeCompiler::build_checked_module),
//! which stamps each item's closures with generated provenance
//! (`GeneratedNodeIssuer`) and reserves a hygienic export symbol (`SymbolId`)
//! with exactly the per-item sequence the fresh-generated declaration-discovery
//! pre-pass runs — producing this `CheckedModule`.
//!
//! # One path (slice-5 deletion complete)
//!
//! Slice 5 deleted the legacy source-string route WHOLE — the
//! `parse_module_items_payload` reparser, `__emit_replace_module`'s string arm,
//! and the `ComptimeDirective::ReplaceModule` variant. A source-string `replace
//! module` payload is now rejected at the builtin boundary with the named
//! [C0929] diagnostic. The typed path was pinned end-to-end before the deletion
//! (`tests/annotations_comptime/directives.rs` typed install+run,
//! `bin/shape-cli/tests/cli/jit_c2_install_native.rs` VM+JIT).

use shape_ast::ast::{FunctionDef, Item, Statement};

use crate::compiler::comptime_builtins::expansion_provenance::SymbolId;

/// ADR-009 E1 #17 (slice 1) — the public typed builder for a checked generated
/// body (`CheckedBodyBuilder` + `finish()`), discharging the C2-D1 amendment.
/// The construction chokepoint; the atomic install stays the C2
/// `crate::compiler::checked_body` seam (see the submodule's construction/install
/// split docs). API-foundation-only: the `CheckedBody` carrier + builder are
/// consumed by E1 slices 3/4/5, which add the re-export at their wiring site.
pub(in crate::compiler) mod checked_body;

/// A comptime-generated module replacement, checked at construction: its items
/// carry generated closure provenance and each generated declaration owns a
/// reserved hygienic export identity (`Exports`). Built ONLY by
/// [`BytecodeCompiler::build_checked_module`](crate::compiler::BytecodeCompiler::build_checked_module)
/// from the typed `ReplaceModuleChecked` route — a source/JSON string never
/// participates.
///
/// **`Exports` (slice 1):** the reserved hygienic export symbols, one per
/// generated declaration. Slice 1's only typed producer is `item_fn`, which
/// mints exactly one function, so a slice-1 `CheckedModule`'s `Exports` is the
/// single hygienic exported symbol of that one function.
///
/// **Single-item limitation (slice 1):** `items` is a `Vec<Item>` internally,
/// but the slice-1 typed producer (`item_fn`) yields exactly one function; the
/// multi-item module is a straight `Vec` extension once a multi-item producer
/// (`quote module { … }`) lands in a later slice — no shape change here.
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

/// A single comptime-generated declaration produced by `item_fn` (slice 2) —
/// the typed carrier that replaced the `__ComptimeItemFragment` sentinel map
/// (E2-D10; the sentinel schema + machinery were deleted in slice 5). It wraps a
/// fully-formed AST `Item` built directly at construction
/// (typed return + literal body, no sentinel `literal_kind`/parallel fields, no
/// source/JSON string). `item_fn` yields a `__CheckedItem` handle across the
/// comptime VM; this is the compiler-side item that handle resolves to.
///
/// Provenance-READY, not yet reserved: a comptime builtin has no `&mut` compiler
/// access, so the driver's shared check sequence
/// (`BytecodeCompiler::check_generated_function_item`) is what stamps the
/// closures and reserves the hygienic export `SymbolId` — at the extend-items /
/// replace-module consumer, where that access exists. `<Decl>` is the single
/// declaration carried; `item_fn` mints exactly one function, so a
/// multi-declaration item is a later slice (`quote item`), a `Vec` extension of
/// this carrier — no shape change here.
#[derive(Clone)]
pub(in crate::compiler) struct CheckedItem {
    item: Item,
}

impl CheckedItem {
    /// Carry a freshly-built, provenance-ready generated declaration. Never a
    /// sentinel map or a source/JSON string — the `item` is already an AST node.
    pub(in crate::compiler) fn new(item: Item) -> Self {
        Self { item }
    }

    /// The carried declaration.
    pub(in crate::compiler) fn item(&self) -> &Item {
        &self.item
    }

    /// Consume into the declaration for the consumer's check sequence.
    pub(in crate::compiler) fn into_item(self) -> Item {
        self.item
    }
}

/// ADR-009 E2 #18 (slice 3) — the typed carrier for a `replace body` edit, used
/// to materialize the replacement PRE-ANALYSIS so the analyzer sees (and
/// publishes structural inference facts for) the replacement's closures.
///
/// # Why this exists — the C0911 quarantine and its flip
///
/// A `replace body` swaps a function's body at PASS-2, after the shared analyzer
/// has already run and handed its closure inference facts to the compiler
/// (immutably). So the analyzer never sees the replacement's closures and no
/// structural specialization fact is published for them — an edited closure's
/// capture then RESOLVES to a `[C0911]` MissingInferenceFact quarantine rather
/// than an exact identity (the C2 #13 named finding,
/// `checked_body/mod.rs` §"Existing-body edits"). E2 fixes this by
/// materializing the const-free function-target edit through the same
/// pre-analysis window the fresh-generated declaration pre-pass uses, so the
/// analyzer infers the STAMPED replacement closure and records the fact keyed by
/// its generated origin. The pre-pass and pass-2 both stamp with the SAME
/// `ExpansionSite`, so both produce the same content-derived closure-origin
/// identity (`GeneratedNodeIdentity`) — the fact the analyzer now publishes is
/// keyed identically to the capture descriptor pass-2 builds. That key equality
/// IS the flip.
///
/// # The typed route (slice-5 deletion complete)
///
/// This is the TYPED route, now the ONLY `replace body` transport. The legacy
/// source/JSON string transport that produced the `ReplaceBody` directive
/// (`parse_function_body_payload` / `__body_probe`, U03) was deleted WHOLE in
/// slice 5 (E2-D8); the block-form `replace body { ... }` stashes its statements
/// as a typed carrier and the expr form is rejected at compile with [C0928]. The
/// typed route never reparses a string and never materialized a pre-analysis
/// edit from source text.
///
/// # Analysis-only, journaled, atomic
///
/// The carrier drives an ANALYSIS-program edit only (body swap + the hygienic
/// `ctx.original` shadow), never a mutation of the shipped program — pass-2
/// still performs the authoritative install byte-unchanged. The one persistent
/// publication is the shadow's reserved hygienic identity (`shadow_export`),
/// journaled through the already-open C2 `InstallTransaction`
/// (`begin_checked_body_install` in `compile_in_place`), so a failing compile
/// rolls it back with the rest of the transaction — the "no half-materialized
/// edit" atomicity the rollback pin asserts.
#[derive(Clone)]
pub(in crate::compiler) struct CheckedReplaceBody {
    /// The edited function's name — the driver swaps THIS function's body in the
    /// analysis-program clone before `analyze_program_full`.
    target_name: String,
    /// The replacement body, `ctx.original`-rewritten and closure-stamped with
    /// generated provenance (same `ExpansionSite` as pass-2). Never a reparsed
    /// string.
    replacement_body: Vec<Statement>,
    /// The hygienic `ctx.original` shadow (the pre-annotation body under the
    /// unspellable shadow name) — prepended to the analysis program so a
    /// `ctx.original(...)` call in the replacement resolves at analysis time.
    /// Its closures are deliberately NOT generated-stamped (they retain user
    /// semantics; the capture gate follows the node stamp, not this reservation).
    shadow: FunctionDef,
    /// The shadow's reserved hygienic export identity, journaled through the open
    /// `InstallTransaction`. Rolls back on a failed install.
    shadow_export: SymbolId,
}

impl CheckedReplaceBody {
    /// Carry an already-stamped replacement body, its hygienic shadow, and the
    /// shadow's reserved identity. The rewrite + stamp + reservation is the
    /// builder's responsibility
    /// ([`BytecodeCompiler::build_checked_replace_body`](crate::compiler::BytecodeCompiler::build_checked_replace_body));
    /// this constructor only carries the result.
    pub(in crate::compiler) fn new(
        target_name: String,
        replacement_body: Vec<Statement>,
        shadow: FunctionDef,
        shadow_export: SymbolId,
    ) -> Self {
        Self {
            target_name,
            replacement_body,
            shadow,
            shadow_export,
        }
    }

    /// The edited function's name.
    pub(in crate::compiler) fn target_name(&self) -> &str {
        &self.target_name
    }

    /// The stamped, `ctx.original`-rewritten replacement body.
    pub(in crate::compiler) fn replacement_body(&self) -> &[Statement] {
        &self.replacement_body
    }

    /// The hygienic `ctx.original` shadow function.
    pub(in crate::compiler) fn shadow(&self) -> &FunctionDef {
        &self.shadow
    }

    /// The shadow's reserved hygienic export identity.
    pub(in crate::compiler) fn shadow_export(&self) -> SymbolId {
        self.shadow_export
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

    #[test]
    fn checked_item_carries_and_yields_its_declaration() {
        let checked = CheckedItem::new(one_function_item());
        assert!(matches!(checked.item(), Item::Function(..)));
        let item = checked.into_item();
        assert!(matches!(item, Item::Function(..)));
    }
}
