use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_number(source: &str, expected: f64) {
    ShapeTest::new(source).expect_number(expected);
    ShapeTest::new(source).with_jit().expect_number(expected);
}

#[test]
fn generated_function_rejects_implicit_closure_capture() {
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker() -> int { let value = 41; let worker = || value + 1; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'value'; generated captures must be explicit",
    );
}

#[test]
fn generated_function_rejects_parameter_capture() {
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker(base: int) -> int { let worker = || base + 1; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker(41))
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'base'; generated captures must be explicit",
    );
}

#[test]
fn generated_capture_diagnostic_is_deterministically_sorted() {
    let source = r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker() -> int { let z = 40; let a = 2; let worker = || z + a; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker())
"#;
    let expected =
        "generated closure implicitly captures 'a', 'z'; generated captures must be explicit";
    ShapeTest::new(source).expect_run_err_contains(expected);
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains(expected);
}

#[test]
fn generated_method_rejects_implicit_self_capture() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method read() -> int \{ let worker = || self.id; worker() \} \}")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 42 }
print(job.read())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'self'; generated captures must be explicit",
    );
}

#[test]
fn generated_function_allows_capture_free_closure() {
    let source = r#"
annotation generate_constant() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_constant() -> int { let worker = || 42; worker() }")
  }
}

@generate_constant()
type Job { id: int }

generated_constant()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

#[test]
fn generated_closure_parameters_are_not_captures() {
    let source = r#"
annotation generate_increment() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_increment() -> int { let increment = |value| value + 1; increment(41) }")
  }
}

@generate_increment()
type Job { id: int }

generated_increment()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

#[test]
fn generated_method_allows_capture_free_closure_in_both_tiers() {
    let source = r#"
annotation add_answer() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ let worker = || 42; worker() \} \}")
  }
}

@add_answer()
type Job { id: int }

let job = Job { id: 1 }
job.answer()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

#[test]
fn ordinary_source_closures_keep_implicit_capture_in_both_tiers() {
    let source = r#"
fn answer() -> int {
  let value = 41
  let worker = || value + 1
  worker()
}

answer()
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
//   (c) a `replace body` expansion                   — compiles under the
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
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_nested() -> int { let outer = || { let v = 41; let inner = || v + 1; inner() }; outer() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_nested())
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
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_generic<T>(x: T) -> T { let value = x; let worker = || value; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_generic(41))
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
annotation stub_worker() {
  targets: [function]
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
annotation traced(tag) {
  before(args, ctx) {
    args
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
annotation wrap() {
  targets: [function]
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

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C1 (slice 3) — THE DECLARED CAPTURE CLAUSE.
//
// `|acc, item; move cfg, share total| …` — the clause DRIVES emission. Every
// row below is a named rejection asserted BY ITS EXACT TEXT, and every row is
// fired inside a generated `extend Type { method }` (the flagship), a
// monomorphized generated body, or a nested generated closure.
//
// The accept side lives at the compiler-unit level
// (`compiler/comptime_builtins/capture_plan.rs::declared_tests`) and the
// teardown side at `executor/tests/declared_capture_teardown.rs`, because both
// need to read the EMITTED artifact (`program.closure_function_layouts[fid]`) —
// a value-equality test passes vacuously when the declaration is discarded,
// which is precisely how the first C1 attempt got to green.
// ═══════════════════════════════════════════════════════════════════════════

/// [C0903] — the clause is a GENERATED-CODE-ONLY surface (posted rider 1).
/// Ordinary source keeps inference.
#[test]
fn c0903_capture_clause_in_ordinary_source_is_rejected() {
    ShapeTest::new(
        r#"
fn answer() -> int {
  let value = 41
  let worker = |; move value| value + 1
  worker()
}
print(answer())
"#,
    )
    .expect_run_err_contains(
        "[C0903] a capture clause is only valid in comptime-generated code; ordinary source \
         closures infer their captures — remove the `;` clause",
    );
}

/// The OTHER direction of rider 1: an ordinary source closure with NO clause
/// keeps inferring, and still runs. (Both directions, as required — a gate that
/// only rejects is indistinguishable from a gate that rejects everything.)
#[test]
fn ordinary_source_without_a_clause_still_infers() {
    expect_vm_and_jit_number(
        r#"
fn answer() -> int {
  let value = 41
  let worker = || value + 1
  worker()
}
answer()
"#,
        42.0,
    );
}

/// [C0901] — declared but never used. A stale declaration is how a generated
/// closure silently keeps a capture alive after the body that used it changed,
/// so it is an error rather than a warning.
#[test]
fn c0901_declared_but_unused_is_rejected_in_a_generated_extend_method() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let unused = 7
      let worker = |y: int; move unused| y + 1
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "[C0901] declared capture 'unused' is never used by the closure body; remove the \
         declaration",
    );
}

/// [C0902] — `&x` / `&mut x` are a TOTAL rejection: Shape has no region story
/// for a reference that escapes into a closure. The spelling parses so the
/// diagnostic can be a sentence rather than a syntax error.
#[test]
fn c0902_borrow_capture_is_rejected_in_a_generated_extend_method() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let base = 7
      let worker = |y: int; &base| y + base
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains("[C0902] ReferenceEscapeIntoClosure: declared capture '& base'");
}

/// [C0902] again, for `&mut` — and inside a NESTED generated closure, so the
/// matrix is not only exercised at the top level of a generated body.
#[test]
fn c0902_exclusive_borrow_is_rejected_in_a_nested_generated_closure() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let outer = |; | { let base = 7
      let inner = |; &mut base| base + 1
      inner() }
      outer() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains("[C0902] ReferenceEscapeIntoClosure: declared capture '&mut base'");
}

/// [C0904] — a declaration may not UN-SHARE. `move` over a `var` would give the
/// closure a private snapshot while a sibling keeps writing the shared cell.
#[test]
fn c0904_move_cannot_unshare_a_var_in_a_generated_extend_method() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { var total = 5
      let worker = |y: int; move total| y + total
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "[C0904] 'total' is a shared-ownership binding and cannot be un-shared by a declared \
         `move`; use `share total`",
    );
}

/// [C0905] — the declared name resolves to nothing. No `Immutable` fallback:
/// guessing is how a declaration gets silently downgraded.
#[test]
fn c0905_unresolvable_declared_capture_is_rejected() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let base = 7
      let worker = |y: int; move base, move ghost| y + base
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "[C0905] declared capture 'move ghost' does not resolve to a binding in the enclosing \
         scope",
    );
}

/// [C0908] (user ruling 2) — `share` over a plain local: there is nothing shared
/// to take a share OF. Fired inside a MONOMORPHIZED generated body, so the
/// declaration is proven to survive `substitute_function_def` and still be
/// authoritative in the specialization.
#[test]
fn c0908_share_on_a_plain_local_is_rejected_through_monomorphization() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read<T>(f: T) -> int { let base = 7
      let worker = |t: T; share base| base + 1
      worker(f) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(9))
"#,
    )
    .expect_run_err_contains("[C0908] 'base' is not a shared-ownership binding; use `move base`");
}

/// [C0907] — one binding, one declaration.
#[test]
fn c0907_duplicate_capture_declaration_is_rejected() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let base = 7
      let worker = |y: int; move base, move base| y + base
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "[C0907] duplicate capture declaration for 'base'; each captured binding may be declared \
         exactly once",
    );
}

/// The Wave-46 used-but-undeclared message, VERBATIM — a PARTIAL clause is not a
/// licence to capture the rest implicitly. Same sentence the no-clause path
/// raises, because it comes from the same producer.
#[test]
fn used_but_undeclared_raises_the_wave46_message_verbatim() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let base = 7
      let other = 5
      let worker = |y: int; move base| y + base + other
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'other'; generated captures must be explicit",
    );
}

/// [B0005] VERBATIM — a declared `move` must NOT widen the immutability rule.
/// Writing to a captured `let` is still an error, declaration or no declaration.
#[test]
fn b0005_is_not_widened_by_a_declared_move() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let base = 7
      let worker = |y: int; move base| { base = base + y
        base }
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains("[B0005] cannot assign to immutable binding 'base' captured by closure");
}

/// [B0003] VERBATIM — a declared `move` must NOT widen the reference-escape
/// rule. At the position where the front-end arm actually fires (TOP-LEVEL code,
/// where `current_function.is_none()`), it still fires — and it fires on the
/// INFERRED path, which is the only path that can reach top level: a capture
/// clause is generated-code-only, and generated code always compiles inside a
/// function.
///
/// The declared path is therefore pinned against B0003 by a DIFFERENTIAL at the
/// unit level (`capture_plan.rs::declared_tests::
/// b0003_is_neither_widened_nor_narrowed_by_a_declaration`): a declared `move`
/// over a reference local behaves EXACTLY as inference does at the same
/// position, in both directions. This test holds the top-level arm itself.
#[test]
fn b0003_reference_escape_still_fires_at_top_level() {
    ShapeTest::new(
        r#"
let value = 7
let r = &value
let worker = |y: int| y + r
print(worker(2))
"#,
    )
    .expect_run_err_contains(
        "[B0003] reference 'r' cannot escape into a closure; capture a value instead",
    );
}

/// The forbidden STRING form (`capture("x")`) does not exist as a builtin and
/// does not resolve as a method — a capture is a binding reference, and the
/// grammar makes the string spelling unparseable (pinned in
/// `shape-ast::parser::tests::grammar_coverage::string_form_capture_does_not_parse`).
/// This is the REGISTRY half: no `ctx.capture(...)` capability exists to be
/// rediscovered later.
#[test]
fn no_ctx_capture_builtin_resolves() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    ctx.capture("base")
    extend ("extend Job { method read() -> int { 1 } }")
  }
}

@add_reader()
type Job { id: int }

print(1)
"#,
    )
    .expect_run_err();
}

/// THE ACCEPT, end to end, in both tiers: the flagship generated `extend`
/// method with a declared `move` over a READ-ONLY `let mut` compiles AND RUNS.
/// The layout/opcode proof is at the unit level; this proves the emitted
/// bytecode actually executes to the right answer, on both the interpreter and
/// the JIT.
#[test]
fn declared_move_over_a_read_only_let_mut_runs_in_both_tiers() {
    expect_vm_and_jit_number(
        r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method scale(f: int) -> int { let mut hits = 3
      let worker = |x: int; move hits| x * hits
      worker(f) } }")
  }
}

@add_scaler()
type Job { id: int }

let job = Job { id: 1 }
job.scale(14)
"#,
        42.0,
    );
}

/// The `share` accept, end to end — ON THE INTERPRETER ONLY, and the reason is
/// worth stating plainly rather than hiding behind a helper.
///
/// A `Shared` capture cannot reach the JIT on main at all: `jit_alloc_shared_cell`
/// (`shape-jit/src/ffi/object/closure.rs:519`) is a live `todo!()` awaiting the
/// ADR-006 §2.7.8/Q10 cell parallel-kind track. It ABORTS the process, and it
/// does so for an ORDINARY SOURCE `var`-capturing closure with no declaration
/// anywhere in sight — verified, not assumed. That is a concurrent lane's
/// territory, not this ticket's, and asserting `.with_jit()` here would be
/// asserting someone else's unfinished work.
///
/// The declared path's JIT proof is deliberately NOT attempted in this slice
/// (see the ticket's slice-4 scope): no capturing closure reaches native JIT at
/// HEAD, so a `[jit-fallback]`-free assertion is impossible to write honestly
/// today.
#[test]
fn declared_share_over_a_var_runs_on_the_interpreter() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { var total = 40
      let worker = |y: int; share total| y + total
      worker(x) } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read(2)
"#,
    )
    .expect_number(42.0);
}
