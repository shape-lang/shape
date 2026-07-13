//! ADR-009 B4 (Stage 2, Dec 54) — VM+JIT end-to-end proofs for the uniform
//! nominal-application public surface: `type_constructor(C)`, `.apply(...)`,
//! `const_arg(N)`, `.refine(constructor)`, `.type_argument(I)`.
//!
//! Every POSITIVE program reduces a comptime-only application carrier to a
//! runtime-observable value THROUGH the B1 `reflect` surface (the only channel
//! by which an `AppliedType`'s type argument can be observed at runtime — the
//! carriers themselves are lift-walled). Each positive is proven on BOTH engines
//! via `expect_vm_and_jit_output` (the reflect.rs dual-run helper shape).
//! Negative programs assert the landed NAMED diagnostics end-to-end.
//!
//! Identity invariant note: the model layer already asserts, in both
//! directions, that `identity(apply(constructor(Head), [args]))` byte-equals the
//! A2 `identity(type_ref(Head<args>))` spelling
//! (`type_reflection/tests.rs`). This file proves the SAME identities flow
//! through the executable public surface — a `type_argument` recovered from an
//! application reflects to exactly the primitive that was applied.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

// ─────────────────────────────────────────────────────────────────────────
// Positive: the public surface executes end-to-end on BOTH engines.
// ─────────────────────────────────────────────────────────────────────────

/// (a) The headline proof: apply a 1-arity builtin head, recover its sole type
/// argument, and reflect it back to its exact primitive payload. The `int` that
/// went in through `apply(type_ref(int))` comes back out through
/// `.type_argument(0)` → `reflect` → `SignedInteger(W64)`.
#[test]
fn apply_option_then_type_argument_reflects_the_applied_primitive() {
    let source = r#"
let name = comptime {
  let applied = type_constructor(Option).apply(type_ref(int))
  match reflect(applied.type_argument(0)) {
    FrozenType::Primitive(p) => match p {
      FrozenPrimitive::SignedInteger(w) => match w {
        IntegerWidth::W64 => "int"
        _ => "signed:other"
      }
      _ => "primitive:other"
    }
    _ => "not-primitive"
  }
}

print(name)
"#;
    expect_vm_and_jit_output(source, "int");
}

/// (b) A 2-arity head (`Result<int, string>`): the SECOND type argument
/// (`type_argument(1)`) reflects to `String` — the ordered argument positions
/// are preserved through the applied descriptor, on both engines.
#[test]
fn apply_result_then_second_type_argument_reflects_string() {
    let source = r#"
let name = comptime {
  let applied = type_constructor(Result).apply(type_ref(int), type_ref(string))
  match reflect(applied.type_argument(1)) {
    FrozenType::Primitive(p) => match p {
      FrozenPrimitive::String => "string"
      _ => "primitive:other"
    }
    _ => "not-primitive"
  }
}

print(name)
"#;
    expect_vm_and_jit_output(source, "string");
}

/// (b') The FIRST argument of the same 2-arity application reflects to its own
/// primitive — proving `type_argument` indexes positionally, not by a single
/// fixed slot.
#[test]
fn apply_result_then_first_type_argument_reflects_int() {
    let source = r#"
let name = comptime {
  let applied = type_constructor(Result).apply(type_ref(int), type_ref(string))
  match reflect(applied.type_argument(0)) {
    FrozenType::Primitive(p) => match p {
      FrozenPrimitive::SignedInteger(w) => "int"
      _ => "primitive:other"
    }
    _ => "not-primitive"
  }
}

print(name)
"#;
    expect_vm_and_jit_output(source, "int");
}

/// (c) refine round-trip: an application refined against ITS OWN constructor
/// yields `Some(applied)`, and the recovered application's type argument
/// reflects back to the same primitive — a genuine round-trip through the
/// executable surface, on both engines.
#[test]
fn refine_against_own_constructor_round_trips_the_type_argument() {
    let source = r#"
let name = comptime {
  let applied = type_constructor(Option).apply(type_ref(int))
  match applied.refine(type_constructor(Option)) {
    Some(refined) => match reflect(refined.type_argument(0)) {
      FrozenType::Primitive(p) => match p {
        FrozenPrimitive::SignedInteger(w) => "int"
        _ => "primitive:other"
      }
      _ => "not-primitive"
    }
    None => "none"
  }
}

print(name)
"#;
    expect_vm_and_jit_output(source, "int");
}

/// (d) refine non-match: an `Option` application refined against a `Result`
/// constructor is `None` (head mismatch), never a partial answer, on both
/// engines.
#[test]
fn refine_against_a_different_constructor_is_none() {
    let source = r#"
let label = comptime {
  let applied = type_constructor(Option).apply(type_ref(int))
  match applied.refine(type_constructor(Result)) {
    Some(refined) => "some"
    None => "none"
  }
}

print(label)
"#;
    expect_vm_and_jit_output(source, "none");
}

// ─────────────────────────────────────────────────────────────────────────
// Negative: the landed NAMED rejection matrix, end-to-end.
// ─────────────────────────────────────────────────────────────────────────

/// Wrong arity: a 1-arity head applied with 2 arguments is the named
/// arity rejection (`canonical_apply`).
#[test]
fn apply_with_wrong_arity_is_the_named_arity_rejection() {
    ShapeTest::new(
        r#"
let bad = comptime {
  type_constructor(Option).apply(type_ref(int), type_ref(string))
}
"#,
    )
    .expect_run_err_contains("expects 1 type argument(s), but 2 were provided");
}

/// Wrong kind (const_arg into a Type slot): a checked const argument supplied
/// where the parameter is a type parameter is the named kind rejection
/// (`canonical_apply`). This is how `const_arg` is proven deterministically —
/// no const-parameter head is reachable in the builtin freeze, so a positive
/// const application would be fragile; the wrong-kind wall is exact.
#[test]
fn apply_const_arg_into_a_type_slot_is_the_named_kind_rejection() {
    ShapeTest::new(
        r#"
let bad = comptime {
  type_constructor(Option).apply(const_arg(5))
}
"#,
    )
    .expect_run_err_contains("has the wrong kind");
}

/// R5 — non-nominal head: `type_constructor(int)` over a builtin scalar is the
/// named non-nominal rejection (the runtime carrier builder re-validates the
/// head category).
#[test]
fn type_constructor_over_a_non_nominal_head_is_the_named_r5_rejection() {
    ShapeTest::new(
        r#"
let bad = comptime {
  type_constructor(int)
}
"#,
    )
    .expect_run_err_contains("only nominal type constructors accept type arguments");
}

/// R6 — unknown/unfrozen head: a name that froze to no nominal identity flows
/// the INVALID sentinel (never a name string) and is the named unknown-head
/// rejection.
#[test]
fn type_constructor_over_an_unfrozen_head_is_the_named_r6_rejection() {
    ShapeTest::new(
        r#"
let bad = comptime {
  type_constructor(NotAFrozenType)
}
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

/// type_argument out of range: index past the argument count is the named
/// out-of-range rejection.
#[test]
fn type_argument_out_of_range_is_the_named_rejection() {
    ShapeTest::new(
        r#"
let bad = comptime {
  let applied = type_constructor(Option).apply(type_ref(int))
  applied.type_argument(3)
}
"#,
    )
    .expect_run_err_contains("out of range");
}

// ─────────────────────────────────────────────────────────────────────────
// Lift wall: the application carriers are comptime-only — binding one and
// returning it into runtime code is the named lift rejection.
// ─────────────────────────────────────────────────────────────────────────

/// A `TypeConstructorRef` bound out of comptime into runtime code is the named
/// lift rejection.
#[test]
fn type_constructor_ref_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let leaked = comptime { type_constructor(Option) }
print(leaked)
"#,
    )
    .expect_run_err_contains(
        "TypeConstructorRef is a comptime-only compiler capability and cannot enter runtime code",
    );
}

/// An `AppliedType` bound out of comptime into runtime code is the named lift
/// rejection.
#[test]
fn applied_type_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let leaked = comptime { type_constructor(Option).apply(type_ref(int)) }
print(leaked)
"#,
    )
    .expect_run_err_contains(
        "AppliedType is comptime-only reflection data and cannot enter runtime code",
    );
}
