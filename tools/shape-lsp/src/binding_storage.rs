//! ADR-017 §2 / R23 — the storage-class inlay hint's source of truth.
//!
//! The hint used to be rendered by an LSP-side heuristic over the
//! declaration's shape: `Channel(..)` on the right-hand side meant
//! `SharedAtomicMut`, a primitive type meant `Direct`, anything else meant
//! `UniqueHeap`. It carried an `[… approx]` qualifier because it was guessing,
//! and it named two classes — `SharedAtomic` and `SharedAtomicMut` — that
//! `BindingStorageClass` has no variant for. A `var` whose whole purpose is to
//! let the compiler pick storage was exactly the case the guess got wrong.
//!
//! This module reads the compiler's own verdict from
//! `BytecodeCompiler::binding_storage_query()` instead, following the query
//! sessions in `generated_symbols::compiler_queries`. There is no
//! LSP-side classification left to diverge: a binding the compiler did not
//! decide gets no hint.

use shape_ast::ast::{Item, Program};
use shape_vm::compiler::BindingStorageTable;

/// The compiler's per-binding storage decisions for `program`, or `None` when
/// this document carries no query authority.
///
/// A document with imports needs the module context the diagnostics path
/// builds, which inlay hints do not have; gating there keeps the hint from
/// reporting decisions made in a different module environment from the one the
/// user's diagnostics come from. A compile that fails outright decided
/// nothing worth showing.
pub fn binding_storage_decisions(program: &Program, text: &str) -> Option<BindingStorageTable> {
    if program
        .items
        .iter()
        .any(|item| matches!(item, Item::Import(..)))
    {
        return None;
    }
    let mut compiler = shape_vm::BytecodeCompiler::new();
    compiler.set_type_diagnostic_mode(shape_vm::compiler::TypeDiagnosticMode::RecoverAll);
    compiler.set_compile_diagnostic_mode(shape_vm::compiler::CompileDiagnosticMode::RecoverAll);
    compiler.set_source(text);
    compiler.compile_in_place(program).ok()?;
    Some(compiler.binding_storage_query().clone())
}

/// The tooltip shown for a storage-class inlay hint. Both the eager tooltip
/// (`inlay_hints.rs`) and the lazy `inlayHint/resolve` tooltip
/// (`server.rs`) render this, so the two can never drift into describing the
/// hint differently.
pub fn storage_class_tooltip(class: &str) -> String {
    format!(
        "BindingStorageClass::{class} — the storage class the compiler decided for this \
         binding (ADR-006 §2)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;
    use shape_vm::type_tracking::BindingStorageClass;

    fn decisions_for(src: &str) -> BindingStorageTable {
        let program = parse_program(src).expect("fixture parses");
        binding_storage_decisions(&program, src).expect("fixture compiles")
    }

    fn class_of(table: &BindingStorageTable, name: &str) -> Option<BindingStorageClass> {
        table
            .decisions()
            .iter()
            .find(|decision| decision.name == name)
            .map(|decision| decision.storage_class)
    }

    /// A `var` whose only interaction is `push` into the array it holds
    /// needs no shared storage.
    #[test]
    fn a_var_that_earns_nothing_is_direct() {
        let table = decisions_for(
            "fn f() -> Array<int> {\n    var xs = [1, 2, 3]\n    xs.push(4)\n    xs\n}\nlet r = f()\n",
        );
        assert_eq!(class_of(&table, "xs"), Some(BindingStorageClass::Direct));
    }

    /// A `var` that is read more than once, mutated, and returned earns
    /// copy-on-write through the planner's Rule 3/3b.
    #[test]
    fn an_aliased_and_mutated_var_is_shared_cow() {
        let table = decisions_for(
            "fn f() -> int {\n    var a = 0\n    a = a + 1\n    var b = a\n    b = b + 1\n    a + b\n}\nlet r = f()\n",
        );
        assert_eq!(class_of(&table, "a"), Some(BindingStorageClass::SharedCow));
    }

    /// A `var` mutated through a non-escaping closure takes the Phase D
    /// stack route rather than a heap cell.
    #[test]
    fn a_var_mutated_by_a_non_escaping_closure_is_local_mutable_ptr() {
        let table = decisions_for(
            "fn f() -> int {\n    var s = 0\n    let bump = || { s = s + 1 }\n    bump()\n    s\n}\nlet r = f()\n",
        );
        assert_eq!(
            class_of(&table, "s"),
            Some(BindingStorageClass::LocalMutablePtr)
        );
    }

    #[test]
    fn a_let_binding_is_direct() {
        let table = decisions_for("fn f() -> int {\n    let c = 1\n    c\n}\nlet r = f()\n");
        assert_eq!(class_of(&table, "c"), Some(BindingStorageClass::Direct));
    }

    #[test]
    fn a_document_with_imports_has_no_query_authority() {
        let src = "use std::core::math\nlet x = 1\n";
        let program = parse_program(src).expect("fixture parses");
        assert!(
            program
                .items
                .iter()
                .any(|item| matches!(item, Item::Import(..))),
            "the fixture must actually contain an import"
        );
        assert!(
            binding_storage_decisions(&program, src).is_none(),
            "an imported document must not answer from a different module environment"
        );
    }

    #[test]
    fn the_tooltip_names_the_class_and_never_calls_itself_an_approximation() {
        let tooltip = storage_class_tooltip("SharedCow");
        assert!(tooltip.contains("BindingStorageClass::SharedCow"));
        assert!(
            !tooltip.to_lowercase().contains("approx"),
            "the hint is the compiler's decision, not an approximation: {tooltip}"
        );
    }

    #[test]
    fn every_decided_class_has_a_name_the_compiler_owns() {
        // The hint renders `BindingStorageClass::name()`, so the LSP can
        // never show a class the compiler has no variant for — the specific
        // failure of the retired heuristic, which spelled `SharedAtomic` and
        // `SharedAtomicMut`.
        let table = decisions_for(
            "fn f() -> int {\n    var a = 0\n    a = a + 1\n    var b = a\n    b = b + 1\n    a + b\n}\nlet c = 1\nlet r = f()\n",
        );
        assert!(!table.decisions().is_empty(), "fixture decided something");
        for decision in table.decisions() {
            let rendered = decision.storage_class.name();
            assert!(
                [
                    "Deferred",
                    "Direct",
                    "UniqueHeap",
                    "SharedCow",
                    "Reference",
                    "LocalMutablePtr",
                ]
                .contains(&rendered),
                "{rendered} is not a BindingStorageClass variant"
            );
        }
    }
}
