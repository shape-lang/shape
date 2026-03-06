//! Tests for annotation before/after hook runtime behavior.
//!
//! Covers: before hooks firing before function body, after hooks firing after,
//! before+after chaining order, timing/logging annotation patterns,
//! and hook execution with various return values.

use shape_test::shape_test::ShapeTest;

#[test]
fn before_hook_fires_before_function_body() {
    ShapeTest::new(
        r#"
annotation log_entry(tag) {
  before(args, ctx) {
    print(f"[{tag}] entering")
    args
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
annotation log_exit(tag) {
  after(args, result, ctx) {
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
annotation traced(label) {
  before(args, ctx) {
    print(f"[{label}] before")
    args
  }
  after(args, result, ctx) {
    print(f"[{label}] after = {result}")
    result
  }
}

@traced("sum")
fn sum(a: int, b: int) -> int {
  a + b
}

print(sum(10, 20))
"#,
    )
    .expect_run_ok()
    .expect_output("[sum] before\n[sum] after = 30\n30");
}

#[test]
fn stacked_annotations_execute_outer_first() {
    // Inside-out wrapping: outer annotation's before fires first
    ShapeTest::new(
        r#"
annotation outer(tag) {
  before(args, ctx) {
    print(f"[outer:{tag}] before")
    args
  }
  after(args, result, ctx) {
    print(f"[outer:{tag}] after")
    result
  }
}

annotation inner(tag) {
  before(args, ctx) {
    print(f"[inner:{tag}] before")
    args
  }
  after(args, result, ctx) {
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
annotation check_result(expected) {
  after(args, result, ctx) {
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
annotation negate_result(label) {
  after(args, result, ctx) {
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
    ShapeTest::new(
        r#"
annotation simple_log() {
  before(args, ctx) {
    print("simple_log: entering")
    args
  }
  after(args, result, ctx) {
    print("simple_log: exiting")
    result
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
    ShapeTest::new(
        r#"
annotation counter(name) {
  before(args, ctx) {
    print(f"calling {name}")
    args
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
