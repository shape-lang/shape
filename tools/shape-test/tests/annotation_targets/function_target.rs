//! Tests for annotations applied to function targets.
//!
//! Covers: annotations on top-level function declarations, on functions with
//! various signatures, on recursive functions, on void functions, and
//! on functions with the `targets: [function]` declaration.

use shape_test::shape_test::ShapeTest;

#[test]
fn annotation_on_simple_function() {
    ShapeTest::new(
        r#"
annotation log(tag) {
  before(args, ctx) {
    print(f"[{tag}] called")
    args
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
annotation fn_only(tag) {
  targets: [function]
  before(args, ctx) {
    print(f"[{tag}] before")
    args
  }
  after(args, result, ctx) {
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
annotation track(name) {
  after(args, result, ctx) {
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
annotation count_calls(tag) {
  before(args, ctx) {
    print(f"[{tag}] call with {args[0]}")
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

#[test]
fn annotation_on_multi_param_function() {
    ShapeTest::new(
        r#"
annotation trace(label) {
  before(args, ctx) {
    print(f"[{label}] args = {args}")
    args
  }
  after(args, result, ctx) {
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
annotation wrap(tag) {
  before(args, ctx) {
    print(f"[{tag}] start")
    args
  }
  after(args, result, ctx) {
    print(f"[{tag}] end")
    result
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

// TDD: annotations on async fn not yet supported
#[test]
fn annotation_on_async_function() {
    ShapeTest::new(
        r#"
annotation async_log(tag) {
  before(args, ctx) {
    print(f"[{tag}] async before")
    args
  }
  after(args, result, ctx) {
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
annotation first(n) {
  before(args, ctx) { print(f"first:{n}"); args }
  after(args, result, ctx) { print(f"first:{n} done"); result }
}

annotation second(n) {
  before(args, ctx) { print(f"second:{n}"); args }
  after(args, result, ctx) { print(f"second:{n} done"); result }
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
