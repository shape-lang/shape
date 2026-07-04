# A-final ROOT-fix Implementation Plan

**Date:** 2026-06-01
**Baseline:** strict-flip worktree `shape-strict-flip-collection-dispatch`
@ `f01e83232933bac70b2103d5eed4706411ea9831` (let-gen landed → ROOT A cleared).
Binary: `target/release/shape`.
**Scope:** the 9 A-final roots (B–J) from
`docs/cluster-audits/v0.3.3-a-final-classification.md` §2. ROOT A is already
cleared on this branch (let-gen). This plan sequences the remaining roots.

All 9 roots were re-confirmed reproducing **verbatim** on the strict-flip binary
@ f01e8323 during planning (see per-root reproduction notes in each
`ROOT-*.md`). Every cited seam line was re-verified against the source on this
branch.

---

## 1. Verdict split

### FP_fix_checker (8 roots — need a checker/source edit)

| Root | One-line | File(s) touched | Seam |
|------|----------|-----------------|------|
| **B** | bare `HashMap()` constructor registered monomorphic → `<K,V>` never inferable | `crates/shape-runtime/src/type_system/environment/mod.rs` | `:927-932` (define_builtin → define_polymorphic) |
| **C** | `crypto::fn(...)` result mis-typed as the MODULE → member access rejected | `crates/shape-runtime/src/type_system/inference/expressions.rs` | `:182` `QualifiedFunctionCall` else-arm |
| **D** | checker `str_methods` seed omits `length` (PHF registry has both `len`+`length`) | `crates/shape-runtime/src/type_system/checking/method_table.rs` | `:378` add one line |
| **E** | `Result/Option` match-arm payload binder loses element type (int→number) | `crates/shape-runtime/src/type_system/inference/expressions.rs` | `:1529-1552` `PatternConstructorFields::Tuple` |
| **F** | width-cast `256 as i8` over-rejected (TypeAssertion lacks width carve-out) | `crates/shape-runtime/src/type_system/inference/expressions.rs` | `:364-368` `Expr::TypeAssertion` |
| **G** | single-element `Union([int])` not collapsed for `Comparable` | `crates/shape-runtime/src/type_system/constraints.rs` | `:771-783` `Comparable` arm |
| **H** | nested-fn generic annotation wrapped raw `Concrete(Generic)` not `Type::Generic` | `crates/shape-runtime/src/type_system/inference/expressions.rs` | `:876`+`:899` `Expr::FunctionExpr` |
| **J** | 3 independent sub-seams (J1 pub-const, J2 for-destructure, J3 unannotated-`+`) | J1: `inference/items.rs` / J2: `inference/expressions.rs` (+`shape-ast/.../patterns.rs` if helper) / J3: `inference/operators.rs` | J1 `:243-281` / J2 `:735` / J3 `:357-371` |

### TP_rebaseline_test (1 root — no checker change, edit the test)

| Root | One-line | File touched |
|------|----------|--------------|
| **I** | `int` genuinely has no `impl Display`; the rejection is correct strict behavior. Re-baseline the test to a must-reject. | `tools/shape-test/tests/generics/bounds.rs` (`where_clause_with_function_body`, `:133` assertion + `:124` comment) |

### needs_ruling (0 roots)

None. All 9 roots have a definitive verdict in their spec. (For context: the
broader A-final set has a 28-test LANG_TRUTHINESS cluster blocked on a user
truthiness ruling — but per `project_no_truthiness_coercion.md` the user already
ruled 2026-06-01 that strict Shape requires bool conditions, so that cluster is
NOT in this plan's needs_ruling and is being handled as TP-rebaselines elsewhere.
None of roots B–J is a truthiness case.)

---

## 2. File-territory analysis (conflict grouping)

The decisive shared territory is
`crates/shape-runtime/src/type_system/inference/expressions.rs` — **5 roots**
touch it (**C, E, F, H, J2**). Each touches a *distinct `match` arm* of the same
giant `infer_expr` function (and E touches a separate fn `bind_pattern_vars_typed`
in the same file), so they do not logically conflict, but they DO share the file
and will textually clobber each other's line offsets if applied in parallel
worktrees and merged. They must be done sequentially in ONE worktree.

Every other FP root touches a **disjoint** file:

| File | Root(s) |
|------|---------|
| `inference/expressions.rs` | **C, E, F, H, J2** (SHARED — sequential) |
| `environment/mod.rs` | B (disjoint) |
| `checking/method_table.rs` | D (disjoint) |
| `constraints.rs` | G (disjoint) |
| `inference/items.rs` | J1 (disjoint) |
| `inference/operators.rs` | J3 (disjoint) |
| `tools/shape-test/tests/generics/bounds.rs` | I (disjoint, TP-test only) |

Note J2's *optional* helper route adds `crates/shape-ast/src/ast/patterns.rs`
(a `Pattern::get_identifiers()` method). That file is NOT touched by any other
root, so the helper route stays conflict-free; the local-`collect_pattern_names`
route keeps J2 entirely inside `expressions.rs`. Either route is conflict-clean.

### Within-file collision detail (the 5 expressions.rs arms)

All five edits are in separate, non-overlapping regions of `expressions.rs`:

- **C** — `Expr::QualifiedFunctionCall` else-arm, line **182** (1-line swap).
- **F** — `Expr::TypeAssertion`, insert between line **364** and **368**.
- **J2** — `Expr::For`, line **735** (replace the `as_simple_name` define).
- **H** — `Expr::FunctionExpr`, lines **876** + **899** (2 sites).
- **E** — `bind_pattern_vars_typed` → `PatternConstructorFields::Tuple`, lines
  **1529-1552** (separate function, deep in the file).

Because they are in ascending, disjoint line ranges, applying them in a single
worktree in **line order (C → F → J2 → H → E)** minimizes offset churn — earliest
line first, so each subsequent edit's anchor text is unmoved. (The `Edit` tool
anchors on text, not line numbers, so order is a convenience, not a correctness
requirement — but doing them in one worktree IS a correctness requirement to
avoid merge clobber.)

---

## 3. Implementation batches

### Batch 1 — parallel-safe (disjoint files, separate worktrees OK)

These 5 FP roots touch mutually-disjoint files and can each be implemented in its
own pinned worktree concurrently with no clobber risk:

| Worktree | Root | File |
|----------|------|------|
| `afinal-B` | **B** | `environment/mod.rs` |
| `afinal-D` | **D** | `checking/method_table.rs` |
| `afinal-G` | **G** | `constraints.rs` |
| `afinal-J1` | **J1** | `inference/items.rs` |
| `afinal-J3` | **J3** | `inference/operators.rs` |

(I — the TP-rebaseline — is also disjoint and parallel-safe; it edits only
`tools/shape-test/tests/generics/bounds.rs`. It can ride in Batch 1 as its own
worktree `afinal-I`, or be folded into any batch since it never touches a
compiler source file.)

### Batch 2 — sequential, ONE worktree `afinal-expr` (shared `expressions.rs`)

These 5 FP roots all edit `inference/expressions.rs` and MUST be done serially in
one worktree to avoid line-offset clobber. Apply in line order:

1. **C** — `:182` QualifiedFunctionCall else-arm
2. **F** — `:364-368` TypeAssertion width carve-out
3. **J2** — `:735` For-loop pattern bind walk
4. **H** — `:876`+`:899` FunctionExpr resolve_type_annotation
5. **E** — `:1529-1552` Tuple-payload builtin Result/Option binder

(Optionally J2's shared-helper variant also edits `shape-ast/.../patterns.rs`;
keep it local-helper to stay single-file, OR if the helper route is chosen the
patterns.rs add is still conflict-free with Batch 1.)

### Sequencing note

Batch 1 and Batch 2 are independent of each other (disjoint files), so the two
batches can themselves run concurrently — Batch 1's 5 (+I) worktrees in parallel,
Batch 2 as one serial worktree, all at the same time. Merge all on completion.
Run the full strict-flip shape-test suite after the merge to confirm
fp_regression_total drops from 30 toward 0 and to catch the spec-predicted
"likely also clears" siblings (E's Group-E cluster, G's relational siblings,
H's nested-generic siblings).

---

## 4. Regression / soundness risk flags

Per-root assessment of whether the fix-direction risks a NEW regression or
re-opens a soundness hole. None is a forbidden-pattern (ValueWord / dynamic
dispatch / coercion-opcode) concern — all are pure type-checker inference/
constraint edits. The substantive risks:

- **ROOT B (LOW-MEDIUM, soundness-adjacent — WATCH).** The fix makes `HashMap()`
  polymorphic over fresh `K,V`. The spec is emphatic: do **NOT** also widen
  `expr_is_nonexpansive` (items.rs:1761) or relax `ensure_no_unresolved_generic_args`
  (mod.rs:1017) — both become no-ops once the constructor is polymorphic, and
  touching either risks the **let-gen value-restriction soundness binder**
  (§5 of the let-gen spec, ruled A-ENFORCED 2026-05-31; see
  `project_let_generalization.md`). The single-line constructor swap is the whole
  fix. RISK if an implementer over-reaches into the non-expansiveness gate.
  Verification: confirm `let mut m: HashMap<string,int> = HashMap()` now
  type-checks AND that an *unconstrained* `HashMap()` whose K,V never get pinned
  still surfaces an inference error (not silently generalized at a `let mut`),
  matching the value-restriction discipline.

- **ROOT E (LOW — bounded by explicit soundness probes).** The fix derives
  Ok/Some→args[0], Err→args[1] payload types for builtin `Result`/`Option`. Spec
  §"Why the fix is sound" already enumerates 3 must-still-reject / must-still-pass
  probes (`Ok(1.5)`→int rejects; `Ok int + Err number`→int rejects; `Ok(1.5)`
  →number passes). Verify all 3 post-fix. RISK only if the implementer also edits
  `numeric_result_type` (operators.rs) — the spec forbids it; the arithmetic
  defaulting is correct *given* a proper binder.

- **ROOT C (LOW).** Returns a fresh var instead of the bogus module-Reference.
  The solver tolerates HasField against an unresolved var (only checks once
  concrete — constraints.rs:768). RISK: a fresh var could in principle let a
  *genuinely* wrong member access through if the call result is never otherwise
  constrained — but the bytecode compiler still resolves the real signature, and
  the inference tier truthfully has no module-export signature, so "unknown here"
  is the correct tier-local statement. Do NOT touch the conservative HasField
  `_ =>` arm (constraints.rs:859) or the sibling enum-constructor arm
  (expressions.rs:163/187).

- **ROOT H (LOW — explicitly the producer-side fix).** Normalizes nested-fn
  annotations through `resolve_type_annotation` (matching infer_function). The
  spec explicitly forbids the tempting alternative — adding a
  `(Generic, Concrete(Generic))` cross-representation arm to the solver — as a
  **parallel-implementation patch** that would bless the dual representation
  (CLAUDE.md §Forbidden / parallel-implementation across carrier boundaries).
  RISK if an implementer "fixes the solver instead." Normalize at the source.
  Regression test `inference_tests.rs:407-422` already encodes the
  single-`Type::Generic` invariant — keep it green.

- **ROOT J3 (LOW-MEDIUM — must preserve the TP cases).** The Add-only
  both-operands-unresolved carve-out MUST keep rejecting the genuine-TP cases
  (spec scoping table: case D `c+1`/string rejects, case G `a*b`/string rejects)
  while flipping case F (`a+b`/strings accepts). RISK: too-broad a guard (e.g.
  applying it to `-`/`*`/`/`/`%`, or to a single-concrete-operand `+`) would
  silently swallow real numeric over-constraints. Verify the full D/E/F/G/H
  scoping table from the spec post-fix.

- **ROOT G (LOW).** Mirrors the already-blessed `ImplementsTrait` collapse
  (constraints.rs:991). Heterogeneous unions still fail (members differ). Add the
  two suggested regression tests (single-member collapses / heterogeneous still
  violates). Negligible risk.

- **ROOT D (NEGLIGIBLE).** One-line alias add; PHF registry already has both
  `len`+`length`. Pure desync close.

- **ROOT F (LOW).** Scoped to `IntWidth::from_name` (7 width names only); i64/int/
  number behavior unchanged. The compiler's `CastWidth` path is fully wired.
  Adjacent `5 as i64` / `5 as number` are explicitly OUT of scope (their own
  roots) — do NOT widen the carve-out to them.

- **ROOT J1 (LOW).** Note there are TWO `Item::Export` arms in items.rs (`:243`
  in `infer_item` and `:415` in the sibling predeclare path). The fix targets the
  `infer_item` arm (`:243`); confirm the predeclare arm at `:415` does not also
  need the `source_decl` handling for the binding to be in scope at reference
  time (the spec's repro clears with the `:243` edit, but a two-arm check is
  cheap insurance).

- **ROOT J2 (LOW).** Binds all pattern identifiers at element-type granularity
  (the existing as_simple_name behavior bound the whole element type too, so no
  precision regression). VM already implements for-destructure codegen.

- **ROOT I (NONE — test-only).** Re-baseline to `expect_run_err_contains(
  "does not implement trait 'Display'")`. The decisive seam (constraints.rs:997
  TraitBoundViolation) is CORRECT and must NOT be touched. Registering a phantom
  `impl Display for int` would be a checker weakening — refused. No other
  Display-bound test regresses (all siblings are parse-ok-only or use a user type
  with a real impl).

---

## 5. Post-merge verification checklist

1. `just check-clean` green on the merged worktree.
2. Re-run the 9 reproduction programs (`/tmp/afp_{b..j3}.shape`) — all should now
   compile + run.
3. Full strict-flip shape-test run; confirm:
   - the 13 directly-named tests (B×4, C×3, D×2, E×2, F×1, G×1, H×1, I×1, J×3)
     pass — note I passes by asserting the correct rejection.
   - fp_regression_total moves toward 0.
   - spec-predicted siblings clear (E: Group-E/ALLOWLIST:54 cluster; G: relational
     `< > <= >=` on `Union([T])`; H: nested-generic `Result/Option/Array/HashMap`
     returns).
4. Soundness probes from §4 (B value-restriction, E 3-probe set, J3 D/F/G scoping
   table) re-confirmed must-reject / must-pass as stated.
