//! Tests for annotation wrapping patterns (memoization, retry, timeout-like patterns).
//!
//! Covers: after hook wrapping return values, caching patterns, retry patterns,
//! conditional result transformation, and composition of wrapping annotations.
//!
//! C3-S5c pin-rewrite wave 1: six pins rewritten IN PLACE onto the typed
//! surface (asserted outputs byte-identical). C3-S6 A-phase wave:
//! `stacked_after_hooks_transform_result_in_order` rewritten typed (the F2
//! onion preserves the asserted value 12). Remaining legacy spelling (1):
//! `after_hook_wraps_result_in_string` — the planned F4 rejection-pin
//! conversion is BLOCKED on the surfaced after-return soundness gap (the
//! polymorphic `after(result)` sugar path does not check the body's actual
//! return type against the bound `(R) -> R`; a type-changing body leaks a
//! heap pointer as the scalar result). Stays as retained legacy coverage
//! pending supervisor disposition.

use shape_test::shape_test::ShapeTest;

#[test]
fn after_hook_doubles_numeric_result() {
    ShapeTest::new(
        r#"
annotation double_result(label: string) {
  after(result) {
    print(f"[{label}] doubling {result}")
    result * 2
  }
}

@double_result("x2")
fn compute(x: int) -> int { x + 10 }

let r = compute(5)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[x2] doubling 15")
    .expect_output_contains("30");
}

// C3-S6 A-phase PROBE FINDING (2026-07-21, surfaced — NOT converted): the
// planned F4 rejection-pin conversion (typed `stringify(prefix: string)` +
// `after(result) { f"{prefix}: {result}" }` on an int target, expecting the
// per-specialization body-vs-Sig rejection) does NOT reject at HEAD — the
// polymorphic `after(result)` sugar path never checks the body's ACTUAL
// return type against the bound `(R) -> R`, the woven program runs, and the
// string's HEAP POINTER prints as the int result (measured: `97366122325584`;
// a bare `"xx"` return leaks the same way). SOUNDNESS HOLE surfaced for
// supervisor disposition; the F4 dated withdrawal cannot execute until the
// after-return gate exists, so this pin stays on the legacy spelling as
// retained coverage.
#[test]
fn after_hook_wraps_result_in_string() {
    ShapeTest::new(
        r#"
annotation stringify(prefix) {
  after(args, result, ctx) {
    f"{prefix}: {result}"
  }
}

@stringify("Result")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("Result: 7");
}

// C3-S6 A-phase typed rewrite. The typed after-chain applies in REVERSE
// application order (the S2-F2 onion: nearest-the-fn annotation innermost),
// which reproduces the legacy inner-first order exactly — the asserted
// value 12 is preserved by design (the F2 ledger).
#[test]
fn stacked_after_hooks_transform_result_in_order() {
    // Inner after fires first, outer after fires second
    ShapeTest::new(
        r#"
annotation add_one(label: string) {
  after(result) {
    print(f"[{label}] {result} + 1")
    result + 1
  }
}

annotation times_two(label: string) {
  after(result) {
    print(f"[{label}] {result} * 2")
    result * 2
  }
}

@times_two("outer")
@add_one("inner")
fn base(x: int) -> int { x }

// base(5) = 5 -> inner: 5+1=6 -> outer: 6*2=12
let r = base(5)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("12");
}

#[test]
fn after_hook_conditionally_transforms_result() {
    ShapeTest::new(
        r#"
annotation cap_at(max_val: int) {
  after(result) {
    if result > max_val {
      print(f"capped {result} to {max_val}")
      max_val
    } else {
      result
    }
  }
}

@cap_at(100)
fn square(x: int) -> int { x * x }

print(square(5))
print(square(20))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("25")
    .expect_output_contains("capped 400 to 100");
}

#[test]
fn after_hook_returns_original_on_passthrough() {
    ShapeTest::new(
        r#"
annotation passthrough(tag: string) {
  before() {
    print(f"[{tag}] before")
  }
  after(result) {
    print(f"[{tag}] after, result unchanged")
    result
  }
}

@passthrough("noop")
fn identity(x: int) -> int { x }

let r = identity(99)
print(r)
"#,
    )
    .expect_run_ok()
    .expect_output("[noop] before\n[noop] after, result unchanged\n99");
}

#[test]
fn annotation_wrapping_void_function() {
    ShapeTest::new(
        r#"
annotation wrap_void(tag: string) {
  before() {
    print(f"[{tag}] before void")
  }
  after() {
    print(f"[{tag}] after void")
  }
}

@wrap_void("side-effect")
fn log_message(msg: string) {
  print(f"LOG: {msg}")
}

log_message("test message")
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[side-effect] before void")
    .expect_output_contains("LOG: test message")
    .expect_output_contains("[side-effect] after void");
}

#[test]
fn annotation_with_string_result_transformation() {
    ShapeTest::new(
        r#"
annotation prefix_result(prefix: string) {
  after(result) {
    f"{prefix}_{result}"
  }
}

@prefix_result("v1")
fn get_name() -> string { "release" }

print(get_name())
"#,
    )
    .expect_run_ok()
    .expect_output_contains("v1_release");
}

// TDD: annotation-based memoization requires mutable state capture (closures in annotations).
// C3-S5c wave-1: hooks fire per call on the recursive target; the contains-
// assertions hold; `args[0]` hoists to a local (the F5 f-string boundary).
#[test]
fn annotation_memoize_pattern_basic() {
    ShapeTest::new(
        r#"
annotation memoize(label: string) {
  before(args) {
    let v = args[0]
    print(f"[{label}] computing for {v}")
    args
  }
  after(result) {
    print(f"[{label}] result = {result}")
    result
  }
}

@memoize("fib")
fn fib(n: int) -> int {
  if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}

print(fib(5))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("5");
}
