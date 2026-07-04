# A-final STAGE 3 — Deterministic sf-NEW re-run (post ROOT-A/B/C)

**Date:** 2026-06-02
**Strict-flip:** worktree `shape-strict-flip-collection-dispatch` @ `b526d407`
(cumulative strict-flip; ROOT-A closure-layout + ROOT-B/C Ok-Some-payload /
no-callsite-body-skip landed on top of `ecbdfc54`). Binary: `target/release/shape`.
**Baseline (main):** `ad0005c7` — binary-identical to `705cd854` for the lib
(the two intervening commits `7aa14768` + `ad0005c7` are docs-only; `git diff
705cd854 ad0005c7` touches only `docs/`). Binary: `../shape/target/release/shape`.

## Method (DETERMINISTIC — not the flake-inflated parallel chunk count)

Per `project_bulk_test_hang.md` the parallel `-P4` module-chunked count is
flake-inflated. This run is single-threaded per test binary:

- Build every `shape-test` integration binary in BOTH worktrees
  (`cargo test -p shape-test --release --no-run`).
- Run each binary as its own process with `--test-threads=1`
  (`error_handling` adds `--skip runtime_err_stack_overflow`).
- Record `FAILED` test names per binary.
- **sf-NEW = (FAIL on strict-flip) ∖ (FAIL on main)** via `comm -23` on the
  sorted name sets — same deterministic method on both sides, so the large
  common pre-existing-residual baseline cancels in the diff.

## Result

| Quantity | Count |
|---|---:|
| strict-flip total FAILED (unique) | 1164 |
| main total FAILED (unique) | 1184 |
| **sf-NEW (fail strict-flip, pass main)** | **8** |
| sf-FIXED (fail main, pass strict-flip) | 28 |

The ~1156 common failures are the pre-existing shape-test residual clusters
(documented in CLAUDE.md "Pre-existing shape-test failure clusters" + the
release-mode superset). They fail on BOTH binaries and cancel in the diff;
they are NOT strict-flip regressions.

**sf-NEW dropped from 78 (2026-06-01 classification) → 8.** The let-gen +
Ok/Some-payload-adoption + closure-layout fixes cleared the entire prior FP
set (Roots A–J) and the 30 FP_REGRESSIONs, and additionally fixed 28
main-failing tests (sf-FIXED below).

## The 8 sf-NEW, classified

### Genuine FP_REGRESSION (5) — one root: unannotated reference-param inference gap

Functions whose **reference** params (`&sum`, `&a`, `&b`) carry NO type
annotation and whose body binop has no literal to pair against
(`sum = sum + val`, `let tmp = a + b`). The param's concrete type is known
only at the call site (`add_to(&total, i)` with `total: int`;
`fib_step(&a, &b)` with caller `let mut a = 0`), but the global inference
tier does NOT flow the caller's ref-target type into the reference param, so
the body emits `Add` on `unknown` operands and strict-typing rejects:

```
Semantic error: Cannot infer types for binary operation `Add`:
operand types are `unknown` and `unknown|int`.
```

All 5 PASS on main, FAIL only on strict-flip → valid code over-rejected →
`spurious-reject of valid code` → v0.3.3-gating per the
no-known-incorrectness-ships rule.

1. `borrow_refs::borrow_rules::test_borrow_fibonacci_via_refs`
2. `borrow_refs::borrow_rules::test_borrow_for_in_with_ref_accumulator`
3. `borrow_refs::borrow_rules::test_borrow_loop_accumulator_sum_1_to_100`
4. `borrow_refs::borrow_scoping::borrow_for_loop_sum_1_to_100`
5. `borrow_refs::complex::complex_fibonacci_via_refs`

**Root (NEW — not in the prior A–J set).** Sibling of prior Root A (untyped-fn
let-generalization) and ROOT-A-closure (callsite-resolved closures), but lands
on **reference** params. Localized:

- `is_uninstantiated_implicit_generic` (`crates/shape-vm/src/compiler/functions.rs:247-252`)
  EXPLICITLY skips `param.is_reference || param.is_mut_reference`, so a ref
  param never participates in the deferral path — correct, because these fns
  are called via a single shared blob (NOT monomorphized per call site), so
  the body MUST run and produce the real numeric result (5050 / 60 / fib).
  Deferring the body would yield a dead/wrong blob. Deferral is the WRONG fix.
- The ref-param type-stamp at `functions.rs:1597-1671` recovers a type from
  (a) `inferred_param_type_hints` (global inference) and (b)
  `infer_param_type_from_body` literal-pairing. For `sum = sum + val` there is
  NO literal, and the global inference leaves the param a `Type::Variable`
  (so `inferred_type_to_hint_name` at `compiler_impl_reference_model.rs:1135`
  returns `None`). Confirmed: annotating the body (`let tmp: int = a + b`) or
  the param (`&sum: int`) makes all repros pass.
- **Correct fix (deferred — inference-tier change, gate-risk):** the global
  type-inference engine (`shape-runtime/src/type_system/inference/`) must
  unify a reference param's type with its caller-side ref-target type
  (`add_to(&total, …)` ⇒ `&total: int` ⇒ param `sum: int`). This is the
  caller→ref-param type-flow gap, the reference-param analogue of the
  callsite-resolution that ROOT-A landed for closures. NOT attempted in
  STAGE 3 (deterministic-verify + gates stage): an inference-engine change
  risks the 1156-test common baseline and the gate set, and belongs in a
  dedicated fix-stage with blast-radius verification.

### NOT FP — soundness regression toward permissiveness (1)

`error_handling::stress_option::null_reassigned` — program
`let mut x = 42` / `x = None` / `x`; the test asserts
`.expect_run_err_contains("is not compatible with int")` (it EXPECTS a strict
rejection). On main the harness rejects `x = None` (test passes). On
strict-flip the reassignment is NOT rejected at the assignment statement — `x`
silently becomes `Null` and the test gets `Some(String("Null"))` instead of an
error → test FAILS.

This is the OPPOSITE of an over-rejection: strict-flip became too LOOSE for
`int := None`. The sibling `variable_starts_null_then_assigned`
(`let mut x = None; x = 10`) still correctly rejects on both binaries, so the
gap is directional (`int := None` slips; `None := int` caught). Likely a
side-effect of the ROOT-B/C Ok/Some/None bare-payload adoption making `None`
adopt the LHS target type too eagerly on reassignment. Bounded: a downstream
typed use (`x + 1`) IS still caught on both binaries; only the no-further-use
reassignment slips. Gating per "regressions are not an option"; root = the
reassignment type-check in the None-payload-adoption path.

### NOT FP — LSP inlay-hint label drift (2), cosmetic, arguably more-correct

Both are `expect_type_hint_label` assertions (LSP hint cosmetics), NOT code
rejections — no valid code is rejected.

6. `lsp::presentation::inlay_hint_function_return_result_union_for_mixed_ok_values`
   — `fn test() { Ok(1)\n Ok("str") }`; expected `-> Result<int | string>`,
   strict-flip produces `-> Result<string>`. Only the TRAILING expression is
   the return value, so `Result<string>` is arguably the MORE-correct label;
   the old union-of-both-Ok behavior is what changed.
7. `lsp::presentation::inlay_hint_try_operator_unwraps_ok_constructor_inner_type`
   — `fn test() { let r = Ok(1)? }`; expected `: int`, strict-flip produces
   `-> Result<()>` + `: T` (unresolved Err type-var on the `Ok` with the new
   payload-adoption path). Hint-resolution drift on the `?`-unwrap inner type.

Action for both: re-baseline the asserted label to the new (correct/intended)
value, OR fix the inlay-hint type resolution for the `Ok`-trailing-return and
`Ok(_)?`-unwrap cases. No gate impact (LSP cosmetic).

## sf-FIXED (28) — strict-flip fixes vs main (the let-gen / payload-adoption wins)

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

## Gates (all green at b526d407)

- `numeric_conversions`: **104 passed / 0 failed** (single-threaded).
- smoke s1–s5: **5/5 VM == JIT** (F' release-binary harness; s5 now
  `(x, 0)` on both modes — the prior W16.2-B SURFACE is resolved on this branch).
- `just check-clean`: **EXIT 0**.
- `scripts/check-no-dynamic.sh`: **EXIT 0**.

## Bottom line

- **sf-NEW (deterministic) = 8**, of which **5 are genuine FP_REGRESSIONs**
  collapsing to **ONE root** (unannotated reference-param caller→param type
  flow), 1 is a soundness regression (missed `int := None` rejection), and 2
  are cosmetic LSP inlay-hint label drifts.
- The genuine-FP count fell from 30 → **5**, all one root. NO FP is masked as
  a cast-TP; the 5 are unambiguous valid-code over-rejections (all pass on main).
- Clearing the remaining 5 FPs needs the inference-tier caller→ref-param
  type-flow fix (the reference-param analogue of ROOT-A closure
  callsite-resolution) — deferred to a dedicated fix-stage with blast-radius
  verification, NOT done in this deterministic-verify/gates STAGE 3.
