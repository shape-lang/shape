//! ADR-009 B6 (Stage 2, Dec 63) — the public `FrozenCallable` accessor surface.
//!
//! Built on the S1 compiler model (`reflect(type_ref(<callable>))` yields a
//! `FrozenType::Callable(c)` carrying the ordered `ParamDescriptor` array +
//! return identity, proven dual-engine in `reflect.rs`). This module proves the
//! ADR-named accessor SURFACE on top of that carrier:
//!
//!   * `callable.param(I)` — signature-indexed POSITIONAL access (a
//!     `ParamDescriptor` with the position's optionality + passing mode).
//!   * `callable.parameters` — the ordered per-position descriptor collection,
//!     iterated at comptime.
//!
//! and the rejection matrix R1 (string-keyed selection) + R2 (`Array<Any>`
//! params). R3 (descriptor issuance for an unresolved signature fires the
//! freeze diagnostic before any hook) is proven at the freeze-model tier in
//! `crates/shape-vm/.../type_reflection/tests.rs` (the freeze-boundary predicate
//! runs before the payload forms).
//!
//! Every POSITIVE program runs under the plain interpreter AND the JIT tier
//! (`expect_vm_and_jit_output`): the accessor reads the param types at comptime
//! on the comptime VM (comptime never tiers up), and the enclosing program then
//! lowers and runs identically on both engines — the JIT-tier-clean proof at
//! the comptime→runtime boundary.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// The passing-mode label helper: the same exhaustive `PassingMode` match every
/// positional proof runs through.
fn mode_label_of(param_expr: &str) -> String {
    format!(
        r#"match {param_expr}.mode {{
        PassingMode::Move => "move"
        PassingMode::SharedBorrow => "shared"
        PassingMode::ExclusiveBorrow => "exclusive"
      }}"#
    )
}

/// `callable.param(I)` is signature-indexed positional access: the returned
/// `ParamDescriptor` carries the position's passing mode. Read at comptime,
/// observed identically on VM and JIT.
#[test]
fn callable_param_index_selects_the_positional_descriptor_on_vm_and_jit() {
    let p0 = mode_label_of("c.param(0)");
    let p1 = mode_label_of("c.param(1)");
    let p2 = mode_label_of("c.param(2)");
    let source = format!(
        r#"
let modes = comptime {{
  match reflect(type_ref((int, &string, &mut int) -> bool)) {{
    FrozenType::Callable(c) => {{
      let a = {p0}
      let b = {p1}
      let d = {p2}
      a + ":" + b + ":" + d
    }}
    _ => "wrong"
  }}
}}

print(modes)
"#
    );
    expect_vm_and_jit_output(&source, "move:shared:exclusive");
}

/// `callable.param(I)` accepts a COMPUTED position index (it is genuine
/// positional access, not a literal-only sugar): `param(count - 1)` selects the
/// last parameter.
#[test]
fn callable_param_accepts_a_computed_position_index() {
    let last = mode_label_of("c.param(c.params.len() - 1)");
    let source = format!(
        r#"
let last_mode = comptime {{
  match reflect(type_ref((int, &mut string) -> bool)) {{
    FrozenType::Callable(c) => {last}
    _ => "wrong"
  }}
}}

print(last_mode)
"#
    );
    expect_vm_and_jit_output(&source, "exclusive");
}

/// `callable.parameters` is the ADR-named ordered per-position descriptor
/// collection — the same carrier as the value's `params`, reached through the
/// public accessor spelling. Reading its length proves the rename resolves.
#[test]
fn callable_parameters_accessor_resolves_to_the_ordered_collection() {
    let source = r#"
let arity = comptime {
  match reflect(type_ref((int, &string, &mut int) -> bool)) {
    FrozenType::Callable(c) => c.parameters.len()
    _ => -1
  }
}

print(arity)
"#;
    expect_vm_and_jit_output(source, "3");
}

/// Iterating `callable.parameters` at comptime yields the passing modes in
/// signature order. The enclosing program runs identically on VM and JIT.
#[test]
fn callable_parameters_iterates_the_ordered_descriptors_on_vm_and_jit() {
    let mode_label = mode_label_of("p");
    let source = format!(
        r#"
let modes = comptime {{
  match reflect(type_ref((int, &string, &mut int) -> bool)) {{
    FrozenType::Callable(c) => {{
      let mut acc = ""
      for p in c.parameters {{
        acc = acc + {mode_label} + ";"
      }}
      acc
    }}
    _ => "wrong"
  }}
}}

print(modes)
"#
    );
    expect_vm_and_jit_output(&source, "move;shared;exclusive;");
}

// ─────────────────────────────────────────────────────────────────────────
// Rejection matrix — named compile-time diagnostics, never a partial result.
// ─────────────────────────────────────────────────────────────────────────

/// R1 — `callable.param("name")` string-keyed selection is the named rejection:
/// parameters are position-indexed descriptors, never string keys.
#[test]
fn r1_string_keyed_param_selection_is_the_named_rejection() {
    let source = r#"
let bad = comptime {
  match reflect(type_ref((int, string) -> bool)) {
    FrozenType::Callable(c) => c.param("first").optional
    _ => false
  }
}

print(bad)
"#;
    ShapeTest::new(source)
        .expect_run_err_contains("not a string key");
}

/// R2 — a callable parameter modeled as the homogeneous top type `Array<Any>`
/// is the named Any-erasure rejection (parameters are heterogeneous
/// signature-indexed descriptors). Fires at the freeze boundary, before any
/// FrozenCallable is issued.
#[test]
fn r2_array_any_param_is_the_named_any_erasure_rejection() {
    let source = r#"
let bad = comptime {
  match reflect(type_ref((Array<Any>) -> bool)) {
    FrozenType::Callable(c) => c.params.len()
    _ => -1
  }
}

print(bad)
"#;
    ShapeTest::new(source)
        .expect_run_err_contains("heterogeneous signature-indexed descriptors");
}

/// R2 companion: lowercase `any` is the enabled Erased leaf, so a callable
/// parameter typed `any` is accepted (NOT the R2 rejection) — the callable
/// reflects with its one parameter.
#[test]
fn lowercase_any_param_is_accepted_as_the_erased_leaf() {
    let source = r#"
let arity = comptime {
  match reflect(type_ref((any) -> bool)) {
    FrozenType::Callable(c) => c.params.len()
    _ => -1
  }
}

print(arity)
"#;
    expect_vm_and_jit_output(source, "1");
}
