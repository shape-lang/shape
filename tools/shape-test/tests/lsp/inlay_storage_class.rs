//! LSP-I (R8 W11): inlay-hint cross-product coverage for ADR-006 §2
//! `BindingStorageClass` opt-in hints.
//!
//! Substance per supervisor 2026-05-26 Decision 2 ratify:
//!   (a) Single LSP setting `shape.inlayHints.bindingStorageClass.enable: bool`.
//!   (b) Default OFF — users opt in.
//!   (d) Hint surface covers all five ADR-006 §2 categories:
//!       `Direct`, `UniqueHeap`, `SharedCow`, `SharedAtomic`, `SharedAtomicMut`.
//!
//! Cross-product = {ON, OFF} × {Direct, UniqueHeap, SharedCow,
//! SharedAtomic, SharedAtomicMut} = 10 tests.
//!
//! When ON, each test asserts the corresponding `[<class> approx]` (or
//! `[<class> mut approx]` for `_mut` variants) label is rendered.
//! When OFF, the same program is asserted to NOT render any such label.

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
    ShapeTest::new(code).expect_type_hint_label_with_config("[Direct approx]", &cfg(true));
}

#[test]
fn inlay_storage_class_direct_off_renders_nothing() {
    let code = "let x = 42\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[Direct approx]", &cfg(false));
}

// ---------- UniqueHeap ----------

#[test]
fn inlay_storage_class_unique_heap_on_renders_label() {
    // Heap type (string) with no closure capture / no concurrency primitive →
    // UniqueHeap (ADR-006 §2 line 92 + default).
    let code = "let s = \"hello\"\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[UniqueHeap approx]", &cfg(true));
}

#[test]
fn inlay_storage_class_unique_heap_off_renders_nothing() {
    let code = "let s = \"hello\"\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[UniqueHeap approx]", &cfg(false));
}

// ---------- SharedCow ----------

#[test]
fn inlay_storage_class_shared_cow_on_renders_label() {
    // Closure binding → SharedCow (ADR-006 §2 line 117 — escape via capture).
    let code = "let f = |x| x + 1\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[SharedCow approx]", &cfg(true));
}

#[test]
fn inlay_storage_class_shared_cow_off_renders_nothing() {
    let code = "let f = |x| x + 1\n";
    ShapeTest::new(code).expect_no_type_hint_label_with_config("[SharedCow approx]", &cfg(false));
}

// ---------- SharedAtomic ----------

#[test]
fn inlay_storage_class_shared_atomic_on_renders_label() {
    // Atomic constructor → SharedAtomic (ADR-006 §2 line 118 — cross-thread
    // read-shared).
    let code = "let a = Atomic.new(0)\n";
    ShapeTest::new(code).expect_type_hint_label_with_config("[SharedAtomic approx]", &cfg(true));
}

#[test]
fn inlay_storage_class_shared_atomic_off_renders_nothing() {
    let code = "let a = Atomic.new(0)\n";
    ShapeTest::new(code)
        .expect_no_type_hint_label_with_config("[SharedAtomic approx]", &cfg(false));
}

// ---------- SharedAtomicMut ----------

#[test]
fn inlay_storage_class_shared_atomic_mut_on_renders_label() {
    // Channel constructor + mutable binding → SharedAtomicMut (ADR-006 §2
    // line 119 + 143 — cross-task mutation).
    let code = "let mut q = Channel.new()\n";
    ShapeTest::new(code)
        .expect_type_hint_label_with_config("[SharedAtomicMut mut approx]", &cfg(true));
}

#[test]
fn inlay_storage_class_shared_atomic_mut_off_renders_nothing() {
    let code = "let mut q = Channel.new()\n";
    ShapeTest::new(code)
        .expect_no_type_hint_label_with_config("[SharedAtomicMut mut approx]", &cfg(false));
}
