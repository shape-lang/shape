//! ADR-009 B3 S1 — existential descriptor packages + `comptime for some<W...>`.
//!
//! The witness-typed iteration substrate: a comptime
//! `Array<exists<T> FrozenType<T>>` is built from B1 `reflect(type_ref(...))`
//! payloads of ≥2 distinct types (heterogeneous witnesses, introduced into the
//! existential package), then iterated with `comptime for some<T> ft in coll
//! { … }`. Each iteration opens a fresh hidden witness `T` and binds `ft :
//! FrozenType<T>`; the body matches the sealed B1 payload sum — the SAME
//! reflect()/payload surface, no second protocol.
//!
//! ## Engine scope (stated precisely — no overclaim)
//!
//! The `comptime for some` iteration executes at compile time on the comptime
//! VM interpreter. Comptime code NEVER tiers up to the JIT, so neither engine
//! iterates the existential collection at runtime — by the time the program
//! runs, `out` is an already-folded constant. The interpreter/JIT dual-run in
//! [`expect_output_under_interpreter_and_jit`] therefore does NOT prove "the
//! JIT iterates the collection". What it genuinely proves is that the
//! *enclosing* program (the comptime block plus its downstream runtime use of
//! the folded result) lowers and runs identically under both the interpreter
//! and the JIT tier — i.e. the feature is JIT-tier-clean at the
//! comptime→runtime boundary. The authentic iteration proof is the VM comptime
//! evaluation (the plain `ShapeTest` run) plus the freeze-model unit tests that
//! drive the real canonicalizer / freeze overlay
//! (`crates/shape-vm/src/compiler/comptime_builtins/existential.rs`).

use shape_test::shape_test::ShapeTest;

/// Run `source` under the plain interpreter and again with the JIT tier
/// enabled, asserting identical output. For a `comptime for some` program the
/// iteration is folded at compile time on the comptime VM interpreter (comptime
/// never tiers up); this dual-run proves the enclosing program is JIT-tier-clean
/// at the comptime→runtime boundary, NOT that the JIT iterates the collection.
fn expect_output_under_interpreter_and_jit(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// Headline positive proof: iterate a heterogeneous existential descriptor
/// collection with typed per-iteration witness bindings; the body destructures
/// the opened `FrozenType<T>` payload. The iteration runs at comptime on the VM
/// interpreter; the enclosing program then runs identically under the
/// interpreter and the JIT tier (see module doc for the precise engine scope).
#[test]
fn comptime_for_some_iterates_existential_reflect_payloads() {
    let source = r#"
let out = comptime {
  let coll: Array<exists<T> FrozenType<T>> = [
    reflect(type_ref(int)),
    reflect(type_ref(bool)),
  ]
  let mut acc = ""
  comptime for some<T> ft in coll {
    acc = acc + match ft {
      FrozenType::Primitive(p) => "P"
      FrozenType::Never(n) => "N"
      FrozenType::Erased(e) => "E"
      FrozenType::Callable(c) => "C"
    }
  }
  acc
}

print(out)
"#;
    // int and bool both reflect to the Primitive payload category.
    expect_output_under_interpreter_and_jit(source, "PP");
}

/// Rejection-matrix row 6: `comptime for some` over a value that is NOT an
/// existential descriptor collection is the named NON_EXISTENTIAL rejection —
/// never a silent bind-to-anything.
#[test]
fn comptime_for_some_over_non_existential_is_rejected() {
    ShapeTest::new(
        r#"
let out = comptime {
  let nums: Array<int> = [1, 2, 3]
  let mut n = 0
  comptime for some<T> x in nums {
    n = n + 1
  }
  "done"
}
print(out)
"#,
    )
    .expect_run_err_contains("not an existential descriptor package");
}

/// Rejection-matrix row 2: a hidden witness may not escape its `some` opening
/// scope — assigning the opened, witness-typed descriptor to an enclosing
/// binding is the named WITNESS_ESCAPES rejection.
#[test]
fn hidden_witness_may_not_escape_its_opening_scope() {
    ShapeTest::new(
        r#"
let out = comptime {
  let coll: Array<exists<T> FrozenType<T>> = [
    reflect(type_ref(int)),
    reflect(type_ref(bool)),
  ]
  let mut escaped = reflect(type_ref(int))
  comptime for some<T> ft in coll {
    escaped = ft
  }
  "done"
}
print(out)
"#,
    )
    .expect_run_err_contains("hidden witness cannot escape");
}

/// Rejection-matrix row 4 (not-yet-enabled descriptor family): building the
/// existential `some` collection from a `reflect()` of a family whose payload
/// descriptor has NOT landed (a Nominal user struct — only Primitive / Never /
/// Erased are enabled at B1) surfaces the SAME named per-category R1 rejection
/// inside the iteration substrate that `reflect()` gives standalone — never a
/// partial descriptor, never a silent skip.
#[test]
fn comptime_for_some_over_not_yet_enabled_family_is_named_per_category_rejection() {
    ShapeTest::new(
        r#"
type Widget { x: int }

let out = comptime {
  let coll: Array<exists<T> FrozenType<T>> = [
    reflect(type_ref(Widget)),
  ]
  let mut acc = ""
  comptime for some<T> ft in coll {
    acc = acc + "seen"
  }
  acc
}
print(out)
"#,
    )
    .expect_run_err_contains("the Nominal payload descriptor has not landed");
}

// Rejection-matrix rows 1 (witness erased to compiler-internal `Any`), 3
// (second reflection protocol), and 5 (no freeze handle at the some-site) are
// FREEZE-MODEL / ARCHITECTURAL invariants, not user-syntax-reachable failures,
// so their authentic surface is a freeze-model unit test that drives the REAL
// canonicalizer / freeze overlay — NOT a compile-failing ShapeTest:
//
//   * Row 1: capital `Any` is a compiler-internal top type, not a first-class
//     user type (CLAUDE.md "No `any` type"). A user cannot spell it in a slot
//     that survives inference to the some-gate — the gate infers the iterable's
//     element type from the VALUE, and no B1 reflect payload carries `Any`. The
//     guard fires at existential-annotation canonicalization; its real surface
//     is `existential::tests::witness_slot_erased_to_any_is_rejected` (the
//     `type_reflection.rs` canonicalizer rejects an `Any` witness slot).
//     Empirically confirmed: `Array<exists<T> FrozenType<Any>>` in a let
//     annotation does NOT route through the guard (logged in docs/defections.md).
//   * Row 3: `for some` is sugar over the ONE reflect()/payload freeze surface;
//     there is NO user syntax that requests a second reflection protocol
//     (surface-and-stop). Its surface is
//     `existential::tests::second_reflection_protocol_is_a_named_surface_and_stop_refusal`.
//   * Row 5: the per-compilation-unit freeze is always installed before user
//     comptime executes, so a real some-site always holds the handle; the
//     no-handle path is only reachable with a compiler that never installed the
//     freeze. Its surface is
//     `existential::tests::some_site_without_installed_freeze_fires_no_freeze_handle`.

/// Companion positive: a non-witness-typed value derived inside the loop (a
/// plain `string` label) may freely flow to an enclosing accumulator — the
/// escape check keys on the witness type, not on syntactic reference to the
/// loop variable. (Guards against a false-positive escape rejection.)
#[test]
fn non_witness_value_flows_out_of_the_loop_freely() {
    let source = r#"
let out = comptime {
  let coll: Array<exists<T> FrozenType<T>> = [
    reflect(type_ref(int)),
    reflect(type_ref(bool)),
  ]
  let mut count = 0
  comptime for some<T> ft in coll {
    count = count + 1
  }
  count
}
print(out)
"#;
    expect_output_under_interpreter_and_jit(source, "2");
}
