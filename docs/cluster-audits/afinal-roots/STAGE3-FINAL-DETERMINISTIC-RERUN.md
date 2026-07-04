# A-final STAGE 3 — FINAL deterministic sf-NEW re-run (post STAGE-1 + STAGE-2)

**Date:** 2026-06-02
**Strict-flip:** worktree `shape-strict-flip-collection-dispatch` @ `d7361723`
(cumulative strict-flip; STAGE-1 ref-param caller→param type-flow `9411620f` +
STAGE-2 null_reassigned soundness-gate / 2 LSP inlay rebaselines `d7361723`
landed on top of `b526d407`). Binary `target/release/shape` **rebuilt at HEAD**
before this run (the 18:57 binary predated the 19:05 STAGE-2 commit; the
statements.rs / mod.rs / type_inference.rs edits were newer than it — rebuilt
so the run reflects HEAD source).
**Baseline (main):** `ad0005c7` — lib-identical to `705cd854` (the two
intervening commits are docs-only; `git diff 705cd854 ad0005c7` touches only
`docs/`). Binary `../shape/target/release/shape`.

## Method (DETERMINISTIC — per-binary `--test-threads=1`, NOT the flake-inflated parallel chunk count)

Per `project_bulk_test_hang.md` the parallel `-P4` module-chunked count is
flake-inflated. This run is single-threaded per test binary, identically on
both sides:

- Enumerate every `shape-test` integration binary via `cargo test -p shape-test
  --release --no-run --message-format=json` (deduped by executable path):
  **61 binaries on strict-flip, 60 on main**. The only name delta is
  `numeric_conversions` (strict-flip-only TDD suite; 104/0, contributes 0
  failures — does not perturb the diff).
- Run each binary as its own process with `--test-threads=1`
  (`error_handling` adds `--skip runtime_err_stack_overflow`, symmetric on both
  sides so it cancels).
- Extract `FAILED` test names (`^test <path> ... FAILED$`), prefix the binary
  name, sort -u.
- **sf-NEW = (FAIL strict-flip) ∖ (FAIL main)** via `comm -23` on the sorted
  name sets; **sf-FIXED = (FAIL main) ∖ (FAIL strict-flip)** via `comm -13`.
- **Masking guard:** every binary on BOTH sides was confirmed to emit a
  `test result:` summary line (0 missing on each side) — no crash/abort
  silently produced a phantom-0-failure binary that `grep FAILED` would miss.

## Result

| Quantity | Count |
|---|---:|
| strict-flip total FAILED (unique) | **1156** |
| main total FAILED (unique) | 1184 |
| common (both fail) | 1156 |
| **sf-NEW (fail strict-flip, pass main)** | **0** |
| sf-FIXED (fail main, pass strict-flip) | 28 |

Arithmetic is closed and consistent:
- `sf = common(1156) + sf-NEW(0) = 1156` ✓
- `main = common(1156) + sf-FIXED(28) = 1184` ✓

`common == sf-total` because sf-NEW = 0: **every test that fails on strict-flip
also fails on main** (the pre-existing shape-test residual clusters documented
in CLAUDE.md + the release-mode superset). They fail on BOTH binaries and
cancel in the diff; none is a strict-flip regression.

## sf-NEW classification — EMPTY SET

**sf-NEW = 0.** There are no remaining sf-only failures to classify into
{genuine-FP, soundness-regression, cosmetic-rebaselined, TP-correct-rejection}.

- **genuine-FP: 0** — strict-flip over-rejects no valid code that main accepts.
- **soundness-regression: 0** — strict-flip is not looser than main on any test
  in the corpus (the `int := None` directional gap that STAGE-2 closed is the
  only one the prior run found; `null_reassigned` now PASSES and its sibling
  `variable_starts_null_then_assigned` (`None := int`) still rejects — verified
  directly).
- **cosmetic-rebaselined: 0 remaining** — the 2 LSP inlay drifts were
  rebaselined / inlay-finalized in STAGE-2 (`inlay_hint_function_return_result_union_for_mixed_ok_values`
  → `Result<string>`; `inlay_hint_try_operator_unwraps_ok_constructor_inner_type`
  → `: int` via `infer_expr_finalized`). Both PASS now.
- **TP-correct-rejection: 0 in the diff** — the strict TP must-rejects are
  asserted by tests that PASS on strict-flip and are not in the sf-NEW set; they
  do not appear here because they are correctly-rejecting on the branch (and the
  numeric/no-truthiness/let-gen TP migrations were already landed pre-STAGE-3).

### The prior 8 sf-NEW — all cleared (directly re-verified at HEAD)

| Prior sf-NEW | Prior class | Now | Cleared by |
|---|---|---|---|
| `borrow_refs::borrow_rules::test_borrow_fibonacci_via_refs` | genuine-FP | PASS | STAGE-1 ref-param caller→param type-flow |
| `borrow_refs::borrow_rules::test_borrow_for_in_with_ref_accumulator` | genuine-FP | PASS | STAGE-1 |
| `borrow_refs::borrow_rules::test_borrow_loop_accumulator_sum_1_to_100` | genuine-FP | PASS | STAGE-1 (2nd `apply_callsite_unions` pass) |
| `borrow_refs::borrow_scoping::borrow_for_loop_sum_1_to_100` | genuine-FP | PASS | STAGE-1 |
| `borrow_refs::complex::complex_fibonacci_via_refs` | genuine-FP | PASS | STAGE-1 |
| `error_handling::stress_option::null_reassigned` | soundness-regression | PASS | STAGE-2 `infer_assignment` ground-deferred-int-seed-on-non-numeric-RHS |
| `lsp::presentation::inlay_hint_function_return_result_union_for_mixed_ok_values` | cosmetic | PASS | STAGE-2 rebaseline (trailing-expr return) |
| `lsp::presentation::inlay_hint_try_operator_unwraps_ok_constructor_inner_type` | cosmetic | PASS | STAGE-2 `infer_expr_finalized` |

STAGE-1 touches the GLOBAL inference engine
(`type_system/inference/access.rs` + `mod.rs`). The deterministic re-run is the
blast-radius proof that this inference-tier change introduced **no new
common-residual regression**: had it broken any previously-common-passing test,
that test would surface as a NEW sf-only failure (sf-NEW > 0). sf-NEW = 0 and
the strict-flip total dropped 1164 → 1156 (exactly the 8 prior sf-only
failures), so the change is purely subtractive on the failure set.

## sf-FIXED (28) — strict-flip wins vs main (let-gen / payload-adoption / ref-flow)

Identical to the prior run's win set:

```
closures_hof::stress_hof::test_named_recursive_fn
complex_integration::multi_function::test_complex_calculator_four_ops
complex_integration::multi_function::test_complex_recursive_gcd
complex_integration::multi_function::test_complex_recursive_power
complex_integration::real_world::test_program_retry_logic
complex_integration::real_world::test_program_validator
enums::matching_patterns::constructor_ok_err_function_err_path
enums::option::option_from_function_return_none
enums::option::option_from_function_return_some
enums::option::test_complex_accumulate_with_option
enums::option::test_option_in_conditional
enums::result::result_from_function_err_path
enums::result::result_from_function_ok_path
enums::result::test_result_as_function_return_err
enums::result::test_result_as_function_return_ok
literals::stress_booleans_none::test_empty_string_is_truthy
operators::special::error_context_operator
pattern_matching::stress_advanced::t115_match_recursive_function
query_language::advanced::query_join_with_filter
query_language::clauses::query_let_clause_basic
query_language::clauses::query_let_clause_with_where
query_language::clauses::query_multiple_let_clauses
security_permissions::compile_time::process_spawn_denied_with_pure_permissions
stdlib_math::statistical::manual_variance_calculation
stdlib_math::trig::sin_squared_plus_cos_squared
variables_bindings::stress_let_basic::test_width_i8_overflow_compile_error
variables_bindings::stress_let_basic::test_width_u16_overflow_compile_error
variables_bindings::stress_let_basic::test_width_u8_negative_compile_error
```

## Gates (all green at `d7361723`, binary rebuilt at HEAD)

- `numeric_conversions`: **104 passed / 0 failed** (`--test-threads=1`).
- smoke s1–s5: **5/5 VM == JIT** — `(4950,0) (30,0) (x,0) (2,0) (x,0)` on both
  modes. s4 (`Set()`) and s5 (`Array<dyn HasX>` concrete→dyn coercion) both
  resolve — the prior SMOKE-s4-s5 FP roots are fixed in-branch; the stale
  README expected-values (`5d842283` baseline showing s5 SURFACE) are
  superseded.
- `just check-clean`: **EXIT 0**.
- `scripts/check-no-dynamic.sh`: **EXIT 0**.

## Bottom line — READY TO FLIP

- **sf-NEW (deterministic) = 0.** 0 genuine-FP, 0 soundness-regression, 0
  remaining cosmetic, 0 TP-in-diff. Nothing left to classify.
- strict-flip total FAILED = **1156 ≤ 1164**; the STAGE-1 inference change
  introduced **no new common-residual regression** (count fell by exactly the 8
  prior sf-only failures).
- All four gates green. The `ReliableOnly → Strict` default flip is already
  in-branch (`compiler_impl_initialization.rs:125`).
- **`ready_to_flip = true`** — branch ready to merge at the v0.3.3 tag.

No FP is masked as a cast-TP; no regression is absorbed under "pre-existing"
(the 1156 common failures fail identically on main and are the documented
pre-existing residual, verified non-masked via the per-binary summary-line
guard). CLAUDE.md grain respected throughout STAGE-1/2: no int-VALUE→number
widening (literals-only adoption), no dynamic fallback, no parallel-impl
defection, no fabricated kinds, no new conversion opcode.
