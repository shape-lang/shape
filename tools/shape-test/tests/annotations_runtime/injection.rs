//! Tests for annotation argument injection and modification at runtime.
//!
//! Covers: before hooks modifying argument arrays, injecting extra context,
//! conditional argument transformation, and argument inspection.
//!
//! C3-S5c pin-rewrite wave 1 rewrote the two READ-ONLY pins onto the typed
//! surface. The C3-S6 A-phase wave rewrote three of the args-MUTATION family
//! (`before_hook_doubles_first_argument`, `before_hook_clamps_argument_to_range`,
//! `chained_before_hooks_modify_args_sequentially`) onto typed per-slot
//! `args[i] = expr` mutation. The C3-S6 soundness fixlet (supervisor-ordered)
//! taught the S2c guard PROVABLE-INITIALIZER LOCALS, so
//! `before_hook_swaps_arguments` is now rewritten typed (the hoisted-local
//! exchange spelling the A-phase surfaced as guard-blocked). Remaining
//! legacy spelling (1): `before_hook_passes_ctx_info` — E4-blocked per the
//! ratified S2-F3 disposition row (the typed surface has no `ctx` by
//! design); stays as retained legacy coverage pending its ruling. Fixlet
//! round 2 adds the before-side F1 `?`-exit MUST-REJECT pin (measured
//! silent corruption of the woven call on the round-1 state).

use shape_test::shape_test::ShapeTest;

// Previously: before hook arg modification caused int->number type coercion.
// The int->number coercion bug has been fixed.
// C3-S6 A-phase typed rewrite: single-slot `args[0] = expr` mutation (the
// pseudo-tuple's legal write form, proven by the s4c sugar-mutation pin);
// the interior read hoists to a local before f-string interpolation (the F5
// non-scanned boundary). Asserted output unchanged.
#[test]
fn before_hook_doubles_first_argument() {
    ShapeTest::new(
        r#"
annotation double_first(label: string) {
  before(args) {
    let v = args[0]
    print(f"[{label}] original args[0] = {v}")
    args[0] = args[0] * 2
    args
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

// C3-S5c wave-1 typed rewrite: polymorphic `before(args)`; pseudo-tuple
// reads hoist to a local first (f-string interiors are a non-scanned
// boundary for the pseudo-tuple face).
#[test]
fn before_hook_inspects_args_without_modification() {
    ShapeTest::new(
        r#"
annotation inspect(label: string) {
  before(args) {
    let n = args.length
    print(f"[{label}] arg count = {n}")
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

// C3-S6 fixlet round 2, F1 (before side) — MUST-REJECT: a body-level `?`
// inside a `before` body early-returns the propagated Err carrier past the
// typed args-pack exit the weave consumes. MEASURED on the round-1 state:
// this fixture ran and the Err carrier, consumed as the args aggregate,
// silently corrupted the woven call (`add(3, 4) + 1` printed `1` instead of
// `8`). The same `?`-exit arm as the after side fires on the before-side
// specialization scan; a green run here means the arm is bypassed.
#[test]
fn before_hook_try_operator_exit_must_reject() {
    ShapeTest::new(
        r#"
fn fallible(flag: int) -> Result<int, string> {
  if flag == 1 { Ok(5) } else { Err("boom") }
}

annotation trybefore(tag: string) {
  before(args) {
    let x = fallible(0)?
    args
  }
}

@trybefore("t")
fn add(a: int, b: int) -> int { a + b }

let r = add(3, 4)
print(r + 1)
"#,
    )
    .expect_run_err_contains("the `?` operator cannot be used in a `before` template body");
}

// C3-S6 soundness fixlet: the A-phase finding-1 rewrite, executed. The S2c
// guard now admits PROVABLE-INITIALIZER LOCALS (a local whose initializer is
// provable joins the provable write-RHS set at its binding, transitively),
// so the exchange's hoisted-local spelling — the only temp-free-less shape
// an exchange has — is expressible on the typed surface. The refused
// arithmetic-swap rewrite-around stays refused (overflow-unsafe).
#[test]
fn before_hook_swaps_arguments() {
    ShapeTest::new(
        r#"
annotation swap_args(label: string) {
  before(args) {
    print(f"[{label}] swapping args")
    let t = args[0]
    args[0] = args[1]
    args[1] = t
    args
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
    .expect_output_contains("[swap] swapping args")
    .expect_output_contains("7");
}

// C3-S5c wave-1 typed rewrite: read-only `args[0]` hoisted to a local
// before f-string interpolation (the pseudo-tuple face's F5 boundary).
#[test]
fn before_hook_logs_string_argument() {
    ShapeTest::new(
        r#"
annotation log_input(tag: string) {
  before(args) {
    let v = args[0]
    print(f"[{tag}] input = {v}")
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

// C3-S6 A-phase typed rewrite: CONDITIONAL per-slot mutation — `args[0]`
// written inside `if`/`else if` branches, then the unconditional `args`
// tail. This pin is the coverage anchor for the branch-conditional-write
// corner (previously unpinned).
#[test]
fn before_hook_clamps_argument_to_range() {
    ShapeTest::new(
        r#"
annotation clamp_first(min_val: int, max_val: int) {
  before(args) {
    let val = args[0]
    if val < min_val {
      args[0] = min_val
    } else if val > max_val {
      args[0] = max_val
    }
    args
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

// C3-S6 A-phase typed rewrite: STACKED mutating befores (proven by the s4c
// stacked-sugar pin). ORDER DISCLOSURE: the typed before-chain runs in
// APPLICATION order (first-applied outermost, S2-F2 onion) where the legacy
// weave ran nearest-first — the fixture's additions are commutative, so the
// asserted value 16 is identical; only the UNASSERTED print interleaving
// flips ("[first]" now precedes "[second]").
#[test]
fn chained_before_hooks_modify_args_sequentially() {
    ShapeTest::new(
        r#"
annotation add_ten(label: string) {
  before(args) {
    print(f"[{label}] adding 10")
    args[0] = args[0] + 10
    args
  }
}

annotation add_five(label: string) {
  before(args) {
    print(f"[{label}] adding 5")
    args[0] = args[0] + 5
    args
  }
}

@add_ten("first")
@add_five("second")
fn show(x: int) -> int { x }

// 1 -> +10 -> +5 (application order) = 16
print(show(1))
"#,
    )
    .expect_run_ok()
    .expect_output_contains("16");
}
