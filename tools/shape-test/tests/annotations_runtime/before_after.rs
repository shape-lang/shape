//! Tests for annotation before/after hook runtime behavior.
//!
//! Covers: before hooks firing before function body, after hooks firing after,
//! before+after chaining order, timing/logging annotation patterns,
//! and hook execution with various return values.
//!
//! C3-S5c pin-rewrite wave 1: the pins below (except
//! `ctx_target_calls_original_impl_from_after_hook`, E4-blocked per S2-F3)
//! are rewritten IN PLACE onto the typed surface — typed config params,
//! `before()` / `after()` observers, `before(args)` / `after(result)` hooks,
//! and the r2/r9-proven public-API spelling for the zero-param definition.
//! Asserted outputs are byte-identical to the legacy versions.
//!
//! C3-S6 A-phase status: `ctx_target_calls_original_impl_from_after_hook`
//! remains the ONLY legacy spelling in this file — E4-blocked per the
//! ratified S2-F3 disposition row (`ctx.target` has no typed spelling; the
//! surface @remote hard-depends on); it stays as retained legacy coverage
//! until the user ruling the F3 row mandates.

use shape_test::shape_test::ShapeTest;

#[test]
fn before_hook_fires_before_function_body() {
    ShapeTest::new(
        r#"
annotation log_entry(tag: string) {
  before() {
    print(f"[{tag}] entering")
  }
}

@log_entry("greet")
fn greet(name: string) -> string {
  print("inside greet")
  f"Hello, {name}!"
}

let result = greet("Alice")
print(result)
"#,
    )
    .expect_run_ok()
    .expect_output("[greet] entering\ninside greet\nHello, Alice!");
}

#[test]
fn after_hook_fires_after_function_body() {
    ShapeTest::new(
        r#"
annotation log_exit(tag: string) {
  after(result) {
    print(f"[{tag}] exiting with {result}")
    result
  }
}

@log_exit("compute")
fn compute(x: int) -> int {
  print("computing")
  x * x
}

let r = compute(7)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output("computing\n[compute] exiting with 49\n49");
}

#[test]
fn before_and_after_both_fire_in_order() {
    ShapeTest::new(
        r#"
annotation traced(label: string) {
  before() {
    print(f"[{label}] before")
  }
  after(result) {
    print(f"[{label}] after = {result}")
    result
  }
}

@traced("add_nums")
fn add_nums(a: int, b: int) -> int {
  a + b
}

print(add_nums(10, 20))
"#,
    )
    .expect_run_ok()
    .expect_output("[add_nums] before\n[add_nums] after = 30\n30");
}

#[test]
fn stacked_annotations_execute_outer_first() {
    // Inside-out wrapping: outer annotation's before fires first
    ShapeTest::new(
        r#"
annotation outer(tag: string) {
  before() {
    print(f"[outer:{tag}] before")
  }
  after(result) {
    print(f"[outer:{tag}] after")
    result
  }
}

annotation inner(tag: string) {
  before() {
    print(f"[inner:{tag}] before")
  }
  after(result) {
    print(f"[inner:{tag}] after")
    result
  }
}

@outer("A")
@inner("B")
fn identity(x: int) -> int { x }

print(identity(42))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("42");
}

#[test]
fn after_hook_receives_correct_result_value() {
    ShapeTest::new(
        r#"
annotation check_result(expected: int) {
  after(result) {
    if result == expected {
      print("result matches expected")
    } else {
      print(f"mismatch: got {result}, expected {expected}")
    }
    result
  }
}

@check_result(25)
fn square(x: int) -> int { x * x }

print(square(5))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("result matches expected")
    .expect_output_contains("25");
}

#[test]
fn after_hook_can_transform_result() {
    ShapeTest::new(
        r#"
annotation negate_result(label: string) {
  after(result) {
    print(f"[{label}] negating {result}")
    result * -1
  }
}

@negate_result("neg")
fn positive(x: int) -> int { x }

let r = positive(42)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[neg] negating 42")
    .expect_output_contains("-42");
}

#[test]
fn before_hook_with_empty_params() {
    // Zero-param definitions classify Legacy on the declarative surface
    // (S4 zero-param ruling), so this pin uses the r2/r9-proven public-API
    // spelling: a `comptime post` handler installing observer templates.
    ShapeTest::new(
        r#"
fn note_in() {
  print("simple_log: entering")
}

fn note_out() {
  print("simple_log: exiting")
}

annotation simple_log() on function {
  comptime post(target, ctx) {
    install(before_hook(note_in, []))
    install(after_hook(note_out, []))
  }
}

@simple_log()
fn hello() {
  print("hello world")
}

hello()
"#,
    )
    .expect_run_ok()
    .expect_output("simple_log: entering\nhello world\nsimple_log: exiting");
}

#[test]
fn same_annotation_reused_on_multiple_functions() {
    // Two applications with different config = a Dec-95 rule-6 split
    // (two distinct baked specializations of the same observer template).
    ShapeTest::new(
        r#"
annotation counter(name: string) {
  before() {
    print(f"calling {name}")
  }
}

@counter("add")
fn add(a: int, b: int) -> int { a + b }

@counter("mul")
fn mul(a: int, b: int) -> int { a * b }

print(add(2, 3))
print(mul(4, 5))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("calling add")
    .expect_output_contains("calling mul")
    .expect_output_contains("5")
    .expect_output_contains("20");
}

// DARK WINDOW (ADR-009 C3-G14 A′ / S2-F3, retired at the S6 completion):
// `ctx_target_calls_original_impl_from_after_hook` pinned §4.1.5 `ctx.target`
// (the original-impl function value delivered to a legacy runtime hook body —
// the surface the legacy `@remote` hard-depended on). The runtime-hook context
// family is E4's charter; the legacy surface carrying it is deleted, and
// `@remote` itself is dark until E4 re-implements it on the typed HookDecision
// protocol — see issue #68. The legacy fixture (`after(args, result, ctx)` +
// `ctx.target(3)`) can no longer compile, so the pin is retired rather than
// #[ignore]'d; E4's acceptance suite re-pins the capability on its typed form.
