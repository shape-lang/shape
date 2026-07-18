use shape_test::shape_test::ShapeTest;

use super::expect_vm_and_jit_number;

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
    extend target {
      method read(x: int) -> int { let unused = 7
        let worker = |y: int; move unused| y + 1
        worker(x) }
    }
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
    extend target {
      method read(x: int) -> int { let base = 7
        let worker = |y: int; &base| y + base
        worker(x) }
    }
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
    extend target {
      method read(x: int) -> int { let outer = |; | { let base = 7
        let inner = |; &mut base| base + 1
        inner() }
        outer() }
    }
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
    extend target {
      method read(x: int) -> int { var total = 5
        let worker = |y: int; move total| y + total
        worker(x) }
    }
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
    extend target {
      method read(x: int) -> int { let base = 7
        let worker = |y: int; move base, move ghost| y + base
        worker(x) }
    }
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
    extend target {
      method read<T>(f: T) -> int { let base = 7
        let worker = |t: T; share base| base + 1
        worker(f) }
    }
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
    extend target {
      method read(x: int) -> int { let base = 7
        let worker = |y: int; move base, move base| y + base
        worker(x) }
    }
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
    extend target {
      method read(x: int) -> int { let base = 7
        let other = 5
        let worker = |y: int; move base| y + base + other
        worker(x) }
    }
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
    extend target {
      method read(x: int) -> int { let base = 7
        let worker = |y: int; move base| { base = base + y
          base }
        worker(x) }
    }
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
print(job.read(2))
"#,
    )
    .expect_run_err_contains(
        "[B0005] cannot assign to immutable binding 'base' captured by closure",
    );
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
    extend target {
      method read() -> int { 1 }
    }
  }
}

@add_reader()
type Job { id: int }

print(1)
"#,
    )
    .expect_run_err();
}

/// The generated-`extend` acceptance proof for a declared `move`, exercised in
/// both tiers. Emitted layout/opcode authority remains pinned by compiler-unit
/// tests rather than inferred from result equality.
#[test]
fn declared_move_over_a_read_only_let_mut_runs_in_both_tiers() {
    expect_vm_and_jit_number(
        r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method scale(f: int) -> int { let mut hits = 3
        let worker = |x: int; move hits| x * hits
        worker(f) }
    }
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

/// High-level interpreter acceptance for the generated declared-`share` path.
/// Refcounted Shared JIT parity, zero-fallback, and lifecycle are now pinned by
/// the dedicated slice-4 fixtures; keeping this test interpreter-only avoids
/// duplicating those native-backend proofs.
#[test]
fn declared_share_over_a_var_runs_on_the_interpreter() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int { var total = 40
        let worker = |y: int; share total| y + total
        worker(x) }
    }
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
