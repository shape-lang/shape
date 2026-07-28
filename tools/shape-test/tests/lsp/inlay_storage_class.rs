//! LSP-I (R8 W11): inlay-hint cross-product coverage for ADR-006 §2
//! `BindingStorageClass` opt-in hints.
//!
//! Substance per supervisor 2026-05-26 Decision 2 ratify:
//!   (a) Single LSP setting `shape.inlayHints.bindingStorageClass.enable: bool`.
//!   (b) Default OFF — users opt in.
//!   (d) Post-#181 (ERGO-VAR-TRUTH): the hint is a PROJECTION of the
//!       compiler's own per-binding storage decision — real classes only
//!       (`Direct` / `UniqueHeap` / `SharedCow` / `Reference` /
//!       `LocalMutablePtr`), no `approx` qualifier. The retired heuristic's
//!       `SharedAtomic` / `SharedAtomicMut` spellings never existed in the
//!       compiler and are pinned here as never-rendered.
//!
//! `let` bindings stay opt-in behind the toggle; `var` is always hinted
//! (covered in shape-lsp's own inlay_hints tests).

use shape_lsp::inlay_hints::InlayHintConfig;
use shape_test::shape_test::ShapeTest;

/// Build an `InlayHintConfig` with the binding-storage-class hint toggled
/// either ON or OFF, with all other Shape-unique opt-in hints (chain hints)
/// explicitly OFF so they don't pollute the assertion surface.
fn cfg(storage_class_on: bool) -> InlayHintConfig {
    InlayHintConfig {
        show_type_hints: true,
        show_parameter_hints: false,
        show_variable_type_hints: false,
        show_return_type_hints: false,
        show_chain_hints: false,
        show_binding_kind_hints: storage_class_on,
    }
}

// ---------- Direct ----------

#[test]
fn inlay_storage_class_direct_on_renders_label() {
    // Primitive value type → Direct (ADR-006 §2 line 91).
    let code = "let x = 42\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[Direct]", &cfg(true));
}

#[test]
fn inlay_storage_class_direct_off_renders_nothing() {
    let code = "let x = 42\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[Direct]", &cfg(false));
}

// ---------- Usage decides, not type (post-#181) ----------
//
// The retired heuristic guessed by TYPE: a string was `[UniqueHeap approx]`,
// a closure `[SharedCow approx]`. The compiler's real planner decides by
// USAGE: an unaliased, unmutated binding is Direct regardless of its type.
// These pins are the observable proof the type-based heuristic is gone.

#[test]
fn inlay_storage_class_unaliased_string_is_direct_not_type_guessed() {
    let code = "let s = \"hello\"\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[Direct]", &cfg(true));
}

#[test]
fn inlay_storage_class_unaliased_string_off_renders_nothing() {
    let code = "let s = \"hello\"\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[Direct]", &cfg(false));
}

#[test]
fn inlay_storage_class_unaliased_closure_is_direct_not_type_guessed() {
    let code = "let f = |x| x + 1\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[Direct]", &cfg(true));
}

#[test]
fn inlay_storage_class_unaliased_closure_off_renders_nothing() {
    let code = "let f = |x| x + 1\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[Direct]", &cfg(false));
}

// ---------- Retired classes never render (post-#181 truth pin) ----------
//
// The pre-#181 heuristic could spell `SharedAtomic` / `SharedAtomicMut` and an
// `approx` qualifier; neither exists in the compiler's BindingStorageClass.
// Post-#181 the hint is a projection of the compiler's own decision, so these
// labels must never be rendered for any program, toggle on or off.

#[test]
fn inlay_storage_class_never_renders_retired_shared_atomic_labels() {
    let code = "let a = 42\nlet mut q = \"queue\"\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("SharedAtomic", &cfg(true));
}

#[test]
fn inlay_storage_class_never_renders_the_approx_qualifier() {
    let code = "let x = 42\nlet s = \"hello\"\nlet f = |x| x + 1\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("approx", &cfg(true));
}
