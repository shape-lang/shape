//! Tests for annotation wrapping patterns (memoization, retry, timeout-like patterns).
//!
//! Covers: after hook wrapping return values, caching patterns, retry patterns,
//! conditional result transformation, and composition of wrapping annotations.
//!
//! C3-S5c pin-rewrite wave 1: six pins rewritten IN PLACE onto the typed
//! surface (asserted outputs byte-identical). C3-S6 A-phase wave:
//! `stacked_after_hooks_transform_result_in_order` rewritten typed (the F2
//! onion preserves the asserted value 12). C3-S6 soundness fixlet
//! (supervisor-ordered): the AFTER-side return-kind gate landed (the S2c
//! analog at the specialization seam), so the A-phase-blocked F4 conversion
//! executed — `after_hook_wraps_result_in_string` is now the REJECTION pin
//! it was meant to be, with the measured heap-pointer-leak fixture as a
//! MUST-REJECT control, the sugar-path value-less-body backstop pin, and
//! the type-correct positive twin. Zero legacy spellings remain in this
//! file. Fixlet round 2 adds the F1 `?`-exit MUST-REJECT pin (the exact
//! traced shape — measured Err-carrier pointer leak on the round-1 gate),
//! the F2 closure-helper-internal-return green pin (fixed false positive),
//! and the body-level-return control. Fixlet round 4 adds the
//! interpolation-interior `?`-exit MUST-REJECT pin (the round-2 lens's
//! traced bypass — measured pointer-bits leak on the round-3 state) plus
//! its hoisted-and-handled green twin.

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

// C3-S6 soundness fixlet: the F4 rejection-pin conversion the A-phase
// surfaced as BLOCKED, now executed. A type-changing `after(result)` body
// (string f-string on an int-returning target) previously specialized and
// RAN, printing the string's HEAP POINTER as the int result (A-phase
// measured `97366122325584`). The AFTER-side return-kind gate (the S2c
// analog at the specialization seam) makes it the established two-signature
// application-site rejection. A green run here would mean the gate is
// bypassed — the run-err expectation IS the leak refuter.
#[test]
fn after_hook_wraps_result_in_string() {
    ShapeTest::new(
        r#"
annotation stringify(prefix: string) {
  after(result) {
    f"{prefix}: {result}"
  }
}

@stringify("Result")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_err_contains("the template body returns `string`")
    .expect_run_err_contains("returns `int` (the target's declared result type)");
}

// The exact measured heap-pointer-leak fixture shape as a MUST-REJECT
// control: a bare string-returning `after` body on an int target (the
// A-phase measured the same pointer leak for `"xx"` — not f-string
// specific). With the gate present this fails at compile; a runtime
// heap-pointer print means the gate is bypassed and this expectation fails.
#[test]
fn after_hook_type_changing_bare_string_body_must_reject() {
    ShapeTest::new(
        r#"
annotation wrongify(tag: string) {
  after(result) {
    "xx"
  }
}

@wrongify("t")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_err_contains("the template body returns `string`")
    .expect_run_err_contains("cannot specialize for target `add`");
}

// The gate's value-less-completion backstop on the never-analyzer-visited
// sugar path (the API-path twin dies at definition with "must return a
// value" — pinned in `weave.rs`): an `after(result)` body with no
// value-producing exit must never leak unit bits as the int result.
#[test]
fn after_hook_value_less_body_must_reject() {
    ShapeTest::new(
        r#"
annotation swallow(tag: string) {
  after(result) {
    let x = result
  }
}

@swallow("t")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_err_contains("can complete without returning a value");
}

// C3-S6 fixlet round 2, F1 — the EXACT traced shape as a MUST-REJECT pin:
// a body-level `?` inside an `after` body (`let x = fallible()?; result`).
// `?` compiles to an unconditional early return of the propagated Err
// carrier (expressions/advanced.rs), bypassing the round-1 return-kind gate
// (the scan traversed `Expr::TryOperator` transparently) — MEASURED on the
// round-1 gate: this fixture ran, `add(3, 4)` escaped as `Err("boom")`, and
// the int consumer `add(3, 4) + 1` printed the Err carrier's pointer bits
// `102997035238305` (the heap-pointer-reinterpretation class). A green run
// here means the `?`-exit arm is bypassed.
#[test]
fn after_hook_try_operator_exit_must_reject() {
    ShapeTest::new(
        r#"
fn fallible(flag: int) -> Result<int, string> {
  if flag == 1 { Ok(5) } else { Err("boom") }
}

annotation tryhook(tag: string) {
  after(result) {
    let x = fallible(0)?
    result
  }
}

@tryhook("t")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_err_contains("the `?` operator cannot be used in an `after` template body");
}

// C3-S6 fixlet round 4 — MUST-REJECT (traced shape (a)): the round-2 lens's
// one remaining bypass. An f-string interpolation interior is re-parsed and
// compiled IN THE ENCLOSING FRAME (string_interpolation.rs:277-295) but is
// raw text to the specialization scans, so a `?` inside `f"{fallible(0)?}"`
// slipped past both the round-2 F1 arm and the return-kind gate's
// collections. MEASURED on the round-3 state (HEAD 7223fcfb): this fixture
// RAN and `add(3, 4) + 1` printed the Err carrier's pointer bits
// `95158939846801` (the heap-pointer-reinterpretation class). A green run
// here means the interior exit scan is bypassed.
#[test]
fn after_hook_fstring_interior_try_exit_must_reject() {
    ShapeTest::new(
        r#"
fn fallible(flag: int) -> Result<int, string> {
  if flag == 1 { Ok(5) } else { Err("boom") }
}

annotation tryfstr(tag: string) {
  after(result) {
    let x = f"{fallible(0)?}"
    result
  }
}

@tryfstr("t")
fn add(a: int, b: int) -> int { a + b }

let r = add(3, 4)
print(r + 1)
"#,
    )
    .expect_run_err_contains(
        "the `?` operator cannot be used inside an f-string interpolation in an `after` \
         template body",
    );
}

// The round-4 green twin (the rejection sentence's positive twin, verified
// green on the round-3 state before the fix landed): the fallible call
// hoisted OUT of the f-string, match-handled, and the f-string
// interpolating the plain handled local.
#[test]
fn after_hook_fstring_interpolating_handled_local_specializes_green() {
    ShapeTest::new(
        r#"
fn fallible(flag: int) -> Result<int, string> {
  if flag == 1 { Ok(5) } else { Err("boom") }
}

annotation tryfstr(tag: string) {
  after(result) {
    let v = match fallible(1) {
      Ok(x) => x,
      Err(e) => 0
    }
    print(f"[t] {v}")
    result
  }
}

@tryfstr("t")
fn add(a: int, b: int) -> int { a + b }

let r = add(3, 4)
print(r + 1)
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[t] 5")
    .expect_output_contains("8");
}

// C3-S6 fixlet round 2, F2 — the fixed false positive: a closure helper's
// own internal return (`return "s"` inside `|x: int| { ... }`) is a
// CLOSURE-frame exit, not a template-body exit. Before the body-level-only
// filter on the return collection, this type-correct after body (tail =
// `result`, proving the bound int) was rejected with "the template body
// returns `string`" (MEASURED round-2 probe). Now specializes green.
#[test]
fn after_hook_closure_helper_internal_return_specializes_green() {
    ShapeTest::new(
        r#"
annotation withhelper(tag: string) {
  after(result) {
    let helper = |x: int| { return "s" }
    let s = helper(1)
    print(f"[{tag}] {s}")
    result
  }
}

@withhelper("t")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[t] s")
    .expect_output_contains("7");
}

// Control for the F2 filter: a BODY-LEVEL return (statement form, non-tail)
// still enters the return-kind guard — a divergent body-level `return`
// rejects exactly as before the filter landed.
#[test]
fn after_hook_body_level_return_still_scans_and_rejects() {
    ShapeTest::new(
        r#"
annotation wrongret(tag: string) {
  after(result) {
    if result > 0 {
      return "xx"
    }
    result
  }
}

@wrongret("t")
fn add(a: int, b: int) -> int { a + b }

print(add(3, 4))
"#,
    )
    .expect_run_err_contains("the template body returns `string`")
    .expect_run_err_contains("returns `int` (the target's declared result type)");
}

// Positive twin of the rejection pins above: the SAME stringify shape with
// the bound `(R) -> R` honored (string body on a string-returning target)
// still specializes, weaves, and runs.
#[test]
fn after_hook_string_body_on_string_target_specializes_and_runs() {
    ShapeTest::new(
        r#"
annotation stringify(prefix: string) {
  after(result) {
    f"{prefix}: {result}"
  }
}

@stringify("Result")
fn shout(name: string) -> string { f"hi {name}" }

print(shout("Bob"))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("Result: hi Bob");
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
