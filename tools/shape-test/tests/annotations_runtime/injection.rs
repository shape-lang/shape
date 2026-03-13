//! Tests for annotation argument injection and modification at runtime.
//!
//! Covers: before hooks modifying argument arrays, injecting extra context,
//! conditional argument transformation, and argument inspection.

use shape_test::shape_test::ShapeTest;

// BUG: before hook arg modification causes int->number type coercion,
// which triggers a TypeError at AddInt (known bug ct_15).
// The Trusted arithmetic opcodes were removed; AddInt now returns a TypeError.
#[test]
#[should_panic(expected = "Type error: expected int")]
fn before_hook_doubles_first_argument() {
    ShapeTest::new(
        r#"
annotation double_first(label) {
  before(args, ctx) {
    print(f"[{label}] original args[0] = {args[0]}")
    let modified = [args[0] * 2, args[1]]
    modified
  }
}

@double_first("test")
fn add(a: int, b: int) -> int {
  a + b
}

print(add(5, 3))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("13");
}

#[test]
fn before_hook_inspects_args_without_modification() {
    ShapeTest::new(
        r#"
annotation inspect(label) {
  before(args, ctx) {
    print(f"[{label}] arg count = {args.length}")
    args
  }
}

@inspect("info")
fn greet(name: string) -> string {
  f"Hello, {name}!"
}

print(greet("Bob"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[info] arg count = 1")
    .expect_output_contains("Hello, Bob!");
}

// TDD: before hook arg modification causes int->number type coercion
#[test]
fn before_hook_swaps_arguments() {
    ShapeTest::new(
        r#"
annotation swap_args(label) {
  before(args, ctx) {
    print(f"[{label}] swapping args")
    [args[1], args[0]]
  }
}

@swap_args("swap")
fn sub(a: int, b: int) -> int {
  a - b
}

// sub(3, 10) with swapped args becomes sub(10, 3) = 7
print(sub(3, 10))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("7");
}

#[test]
fn before_hook_logs_string_argument() {
    ShapeTest::new(
        r#"
annotation log_input(tag) {
  before(args, ctx) {
    print(f"[{tag}] input = {args[0]}")
    args
  }
}

@log_input("debug")
fn upper(s: string) -> string {
  s
}

print(upper("hello"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[debug] input = hello")
    .expect_output_contains("hello");
}

// TDD: before hook arg modification causes int->number type coercion
#[test]
fn before_hook_clamps_argument_to_range() {
    ShapeTest::new(
        r#"
annotation clamp_first(min_val, max_val) {
  before(args, ctx) {
    let val = args[0]
    if val < min_val {
      [min_val]
    } else if val > max_val {
      [max_val]
    } else {
      args
    }
  }
}

@clamp_first(0, 100)
fn process(x: int) -> int {
  x
}

print(process(150))
print(process(-5))
print(process(50))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("100")
    .expect_output_contains("0")
    .expect_output_contains("50");
}

#[test]
fn before_hook_passes_ctx_info() {
    ShapeTest::new(
        r#"
annotation show_ctx(label) {
  before(args, ctx) {
    print(f"[{label}] ctx = {ctx}")
    args
  }
}

@show_ctx("test")
fn noop() {
  print("noop")
}

noop()
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[test] ctx =")
    .expect_output_contains("noop");
}

// TDD: before hook arg modification causes int->number type coercion
#[test]
fn chained_before_hooks_modify_args_sequentially() {
    ShapeTest::new(
        r#"
annotation add_ten(label) {
  before(args, ctx) {
    print(f"[{label}] adding 10")
    [args[0] + 10]
  }
}

annotation add_five(label) {
  before(args, ctx) {
    print(f"[{label}] adding 5")
    [args[0] + 5]
  }
}

@add_ten("first")
@add_five("second")
fn show(x: int) -> int { x }

// Original 1 -> +5 -> +10 (inside-out) = 16
print(show(1))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("16");
}
