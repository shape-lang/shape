//! Tests for annotations applied to function targets.
//!
//! Covers: annotations on top-level function declarations, on functions with
//! various signatures, on recursive functions, on void functions, and
//! on functions with the `targets: [function]` declaration.
//!
//! C3-S5c pin-rewrite wave 1 rewrote every pin except
//! `annotation_on_multi_param_function`; the C3-S6 A-phase wave rewrote that
//! one too (the F5 whole-args `{args}` rendering is replaced by per-element
//! reads; the asserted line is unchanged). No legacy spellings remain in
//! this file.

use shape_test::shape_test::ShapeTest;

#[test]
fn annotation_on_simple_function() {
    ShapeTest::new(
        r#"
annotation log(tag: string) {
  before() {
    print(f"[{tag}] called")
  }
}

@log("fn")
fn greet() {
  print("hello")
}

greet()
"#,
    )
    .expect_run_ok()
    .expect_output("[fn] called\nhello");
}

#[test]
fn annotation_with_targets_function_on_function() {
    ShapeTest::new(
        r#"
annotation fn_only(tag: string) on function {
  before() {
    print(f"[{tag}] before")
  }
  after(result) {
    print(f"[{tag}] after")
    result
  }
}

@fn_only("test")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[test] before")
    .expect_output_contains("[test] after")
    .expect_output_contains("7");
}

#[test]
fn annotation_on_function_with_return_value() {
    ShapeTest::new(
        r#"
annotation track(name: string) {
  after(result) {
    print(f"[{name}] returned {result}")
    result
  }
}

@track("square")
fn square(x: int) -> int { x * x }

let r = square(6)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output("[square] returned 36\n36");
}

#[test]
fn annotation_on_recursive_function() {
    ShapeTest::new(
        r#"
annotation count_calls(tag: string) {
  before(args) {
    let v = args[0]
    print(f"[{tag}] call with {v}")
    args
  }
}

@count_calls("fact")
fn factorial(n: int) -> int {
  if n <= 1 { 1 } else { n * factorial(n - 1) }
}

print(factorial(4))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[fact] call with 4")
    .expect_output_contains("24");
}

// C3-S6 A-phase typed rewrite. F5 DISCLOSURE: whole-args rendering
// (`f"{args}"`) has no typed spelling (bare `args` in value position is a
// named rejection); the UNASSERTED args line becomes per-element
// interpolation via hoisted reads. The asserted "[math] result =" line is
// unchanged.
#[test]
fn annotation_on_multi_param_function() {
    ShapeTest::new(
        r#"
annotation trace(label: string) {
  before(args) {
    let a0 = args[0]
    let a1 = args[1]
    let a2 = args[2]
    print(f"[{label}] args = {a0}, {a1}, {a2}")
    args
  }
  after(result) {
    print(f"[{label}] result = {result}")
    result
  }
}

@trace("math")
fn weighted_sum(a: int, b: int, w: int) -> int {
  a * w + b * (100 - w)
}

print(weighted_sum(10, 5, 60))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[math] result =");
}

#[test]
fn annotation_on_void_function() {
    ShapeTest::new(
        r#"
annotation wrap(tag: string) {
  before() {
    print(f"[{tag}] start")
  }
  after() {
    print(f"[{tag}] end")
  }
}

@wrap("side")
fn log_msg(msg: string) {
  print(f"LOG: {msg}")
}

log_msg("test")
"#,
    )
    .expect_run_ok()
    .expect_output("[side] start\nLOG: test\n[side] end");
}

// Definition-only pin: the typed-config weave accepts the async target
// (the target is never called; async hook EXECUTION coverage is a JIT
// named-expected-fallback per the S0 fence, S7 territory).
#[test]
fn annotation_on_async_function() {
    ShapeTest::new(
        r#"
annotation async_log(tag: string) {
  before() {
    print(f"[{tag}] async before")
  }
  after(result) {
    print(f"[{tag}] async after")
    result
  }
}

@async_log("async")
async fn fetch(url: string) -> string {
  "response"
}

print("defined async fn")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("defined async fn");
}

#[test]
fn multiple_annotations_on_same_function() {
    ShapeTest::new(
        r#"
annotation first(n: string) {
  before() { print(f"first:{n}") }
  after(result) { print(f"first:{n} done"); result }
}

annotation second(n: string) {
  before() { print(f"second:{n}") }
  after(result) { print(f"second:{n} done"); result }
}

@first("A")
@second("B")
fn value() -> int { 42 }

print(value())
"#,
    )
    .expect_run_ok()
    .expect_output_contains("42");
}
