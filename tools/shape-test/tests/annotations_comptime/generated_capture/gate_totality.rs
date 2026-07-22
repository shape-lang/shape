use shape_test::shape_test::ShapeTest;

use super::expect_vm_and_jit_number;

// E2 #18 5b-2 Option C: subject is the container-agnostic generated-closure
// capture gate (provenance-stamped node, not the container). The free-function
// container is retired with the U03 string route (see #18 close capability-gap
// entry: complex free-fn generation returns with quote-item, E-track); the
// method container carries the identical coverage.
#[test]
fn generated_function_allows_capture_free_closure() {
    let source = r#"
annotation generate_constant() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_constant() -> int { let worker = || 42; worker() }
    }
  }
}

@generate_constant()
type Job { id: int }

let job = Job { id: 1 }
job.generated_constant()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 C1 (slice 2) — GATE TOTALITY.
//
// The Wave-46 gate above used to fire on a NAME predicate
// (`generated_symbols.contains_name(current_function)`). It answered "is the
// closure's immediately-enclosing COMPILED FUNCTION a registered generated
// decl?", which is not the question. Three generated-code shapes answered "no"
// and compiled an implicit capture that the gate exists to reject:
//
//   (a) a closure NESTED inside a generated closure  — enclosing fn is
//       `__closure_N`, not a decl name;
//   (b) a MONOMORPHIZED generated body               — mangled specialization
//       name, not the decl name;
//   (c) a `replace body` expansion                    — compiles under the
//       USER's function name; ungated entirely.
//
// The predicate is now the node's own provenance (`Expr::FunctionExpr::
// generated_origin`), stamped where generated AST enters the program. These are
// TRUE-POSITIVE widenings: each program below compiled clean before slice 2.
// ─────────────────────────────────────────────────────────────────────────

/// (a) The capture is in a closure nested INSIDE a generated closure. The outer
/// closure captures nothing (`v` is bound in its own body), so the gate never
/// saw it: by the time the inner closure compiles, `current_function` is
/// `__closure_0`, which is not in the generated-symbol table.
#[test]
fn generated_nested_closure_rejects_implicit_capture() {
    // Option C (see file header): fn-container retired with U03; the method
    // container carries the nested-generated-closure gate coverage identically.
    ShapeTest::new(
        r#"
annotation generate_worker() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_nested() -> int { let outer = || { let v = 41; let inner = || v + 1; inner() }; outer() }
    }
  }
}

@generate_worker()
type Job { id: int }

let job = Job { id: 1 }
print(job.generated_nested())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'v'; generated captures must be explicit",
    );
}

/// (b) The capture is inside a GENERIC generated body, which reaches emission
/// through monomorphization (`substitute_function_def`). The stamp is forwarded
/// by the substitution rebuild — see `substitution.rs`'s
/// `Expr::FunctionExpr` arms, which name the field explicitly (no `..`), so
/// dropping it is a compile error.
#[test]
fn generated_generic_body_rejects_implicit_capture_through_monomorphization() {
    // Option C (see file header): fn-container retired with U03; the generic
    // METHOD container carries the monomorphized-generated-closure gate coverage
    // (generic methods parse in the direct extend form — grammar method_def
    // type_params?, verified against existing `method describe<N>` extends).
    ShapeTest::new(
        r#"
annotation generate_worker() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_generic<T>(x: T) -> T { let value = x; let worker = || value; worker() }
    }
  }
}

@generate_worker()
type Job { id: int }

let job = Job { id: 1 }
print(job.generated_generic(41))
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'value'; generated captures must be explicit",
    );
}

/// (c) The capture is inside a `replace body` expansion. The replacement body is
/// comptime-GENERATED but compiles under the USER's function name, so the name
/// predicate never fired: before slice 2 this program compiled and printed 42
/// with an undeclared implicit capture in generated code.
#[test]
fn replace_body_expansion_rejects_implicit_capture() {
    ShapeTest::new(
        r#"
annotation stub_worker() on function {
  comptime post(target, ctx) {
    replace body {
      let value = 41
      let worker = || value + 1
      return worker()
    }
  }
}

@stub_worker()
fn compute() -> int { 0 }

print(compute())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'value'; generated captures must be explicit",
    );
}

/// NEGATIVE CONTROL (G4). A `@before`/`@after` hook re-registers the USER's OWN
/// body under a hygienic name. That body is ORDINARY SOURCE — it keeps capture
/// inference, and the gate must stay silent. A predicate that fired on "the
/// enclosing function has a compiler-issued name" would reject this program;
/// node-borne provenance does not, because the user's closure was never stamped.
#[test]
fn annotation_hook_impl_body_keeps_implicit_capture() {
    let source = r#"
annotation traced(tag: string) {
  before() {
    print(f"[{tag}]")
  }
}

@traced("t")
fn compute() -> int {
  let value = 41
  let worker = || value + 1
  worker()
}

compute()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

/// The `replace body` SHADOW (the pre-annotation body reached through
/// `ctx.original`) is the USER's body under a hygienic name — ordinary source,
/// implicit capture allowed — while the REPLACEMENT is generated. One program
/// proves both halves: the shadow captures `base` implicitly and compiles; the
/// replacement declares no closure at all.
#[test]
fn ctx_original_shadow_body_keeps_implicit_capture() {
    let source = r#"
annotation wrap() on function {
  comptime post(target, ctx) {
    replace body {
      return ctx.original() + 1
    }
  }
}

@wrap()
fn compute() -> int {
  let base = 40
  let worker = || base + 1
  worker()
}

compute()
"#;
    expect_vm_and_jit_number(source, 42.0);
}
