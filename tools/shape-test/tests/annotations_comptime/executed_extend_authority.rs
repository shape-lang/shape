//! ADR-009 E3 (slice S1) — behavior-parity harness for the deletion of the
//! parallel static comptime-extend collector (legacy class U12,
//! `crates/shape-ast/src/transform/comptime_extends.rs`).
//!
//! The static collector was a NON-EVALUATING AST scan that fed the analysis
//! program in parallel with the EXECUTED declaration-discovery pre-pass
//! (`materialize_computed_comptime_extends`). The issue mandates behavior
//! parity BEFORE deletion (matrix rows D7/D8/D12 + R6 `ctx.target`): every
//! generated `extend`/method item a program observes must materialize through
//! the executed pre-pass alone. These tests exercise the executed authority
//! and stay green across the deletion — proving the static path was carrying
//! nothing the executed path does not.
//!
//! Reconstruction note (binding-artifact gap): the wave41
//! `docs/cluster-audits/wave41-comptime-untyped-paths.md` matrix is not
//! present in this worktree; the rows below are reconstructed from
//! `docs/design/typed-comptime/annotations-and-hooks.md` +
//! `expansion-and-tooling.md` and the annotations_comptime/comptime
//! regression suites. Re-validate against the wave41 doc if it is obtained.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// D7 — direct handler-AST directive `extend target { method }` materializes
/// through the executed pre-pass; the generated method is callable in BOTH
/// execution modes.
#[test]
fn d7_direct_extend_target_method_materializes_via_executed_prepass() {
    expect_vm_and_jit_output(
        r#"
annotation displayable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method display() -> string { f"({self.x}, {self.y})" }
    }
  }
}

@displayable()
type Point { x: int, y: int }

let p = Point { x: 3, y: 4 }
print(p.display())
"#,
        "(3, 4)",
    );
}

/// D8 — stacked annotations that each emit a direct `extend target { method }`
/// both materialize through the executed pre-pass (multi-generator, one
/// target).
#[test]
fn d8_stacked_annotations_both_extend_via_executed_prepass() {
    expect_vm_and_jit_output(
        r#"
annotation with_sum() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method sum() -> int { self.a + self.b }
    }
  }
}

annotation with_diff() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method diff() -> int { self.a - self.b }
    }
  }
}

@with_sum()
@with_diff()
type Numbers { a: int, b: int }

let n = Numbers { a: 10, b: 3 }
print(n.sum())
print(n.diff())
"#,
        "13\n7",
    );
}

/// D12 — the COMPUTED snippet directive `extend (f"extend {target.name} …")`
/// has NO literal `extend` statement in the handler body, so the deleted
/// static AST scan could never have found it: it materializes exclusively
/// through the executed pre-pass. This is the load-bearing parity row.
#[test]
fn d12_computed_snippet_extend_only_materializes_via_executed_prepass() {
    expect_vm_and_jit_output(
        r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Widget { id: int }

let w = Widget { id: 1 }
print(w.answer())
"#,
        "42",
    );
}

/// R6 — `ctx.target`/`target` resolution: the `target` head of a direct
/// `extend target { … }` resolves to the annotated type through the executed
/// pre-pass (target bound by position, not by any magic spelling that leaks
/// into a symbol table). Two independent annotated types must each receive
/// their own generated method.
#[test]
fn r6_target_resolves_to_annotated_type_per_application() {
    expect_vm_and_jit_output(
        r#"
annotation label() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method tag() -> string { f"{self.id}" }
    }
  }
}

@label()
type Alpha { id: int }

@label()
type Beta { id: int }

let a = Alpha { id: 1 }
let b = Beta { id: 2 }
print(a.tag())
print(b.tag())
"#,
        "1\n2",
    );
}

/// Executed-authority CORRECTNESS: a `false`-guarded `extend` inside a
/// handler body is NOT materialized. The deleted static AST scan recursed
/// into `if` branches unconditionally (it never evaluated the guard), so it
/// could observe false-guarded edits; the executed pre-pass evaluates the
/// guard and generates only the reachable directive. The real method is
/// callable; the guarded phantom method is not.
#[test]
fn false_guarded_extend_is_not_materialized_real_method_still_works() {
    expect_vm_and_jit_output(
        r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    if false {
      extend target {
        method ghost() -> int { 1 }
      }
    }
    extend target {
      method real() -> int { 2 }
    }
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
print(p.real())
"#,
        "2",
    );
}

/// ADR-009 E3 (S1 parity gap, #19) — a FUNCTION-target annotation
/// (`targets: [function]`) whose comptime handler `extend`s an EXPLICIT type
/// materializes the generated method through the SAME executed discovery
/// pre-pass as a type-target extend. The call site precedes the annotated
/// function in source, so the method is only callable if the executed
/// discovery pass (not a source-order pass-2) registered it. Callable in BOTH
/// execution modes. The type-target sibling is `d7` above.
#[test]
fn function_target_extend_explicit_type_materializes_via_executed_prepass() {
    expect_vm_and_jit_output(
        r#"
type Widget { id: int }

annotation add_label() {
  targets: [function]
  comptime post(target, ctx) {
    extend Widget {
      method label() -> string { f"widget-{self.id}" }
    }
  }
}

let w = Widget { id: 7 }
print(w.label())

@add_label()
fn register() -> int { 0 }
"#,
        "widget-7",
    );
}

/// Twin of the row above: the `false`-guarded phantom method must not be
/// callable — the executed authority never materialized it, so the call is
/// an error (the deleted static scan would have made the analyzer believe
/// `ghost` existed).
#[test]
fn false_guarded_extend_phantom_method_is_not_callable() {
    let source = r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    if false {
      extend target {
        method ghost() -> int { 1 }
      }
    }
    extend target {
      method real() -> int { 2 }
    }
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
print(p.ghost())
"#;
    ShapeTest::new(source).expect_run_err();
}
