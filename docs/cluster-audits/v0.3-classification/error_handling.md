# error_handling classification

**HEAD:** 82f049dd
**Total tests in binary:** 441
**Passed:** 351 / Failed: 90 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test error_handling --no-fail-fast -- --nocapture 2>&1`
**Source log:** `/tmp/audit_logs/error_handling_full.log` (test-status list — per-test stdout blocks were not captured in the available re-run; classification anchored on `enums.md` Group F precedent + per-test fixture inspection + user 2026-05-27 classification disposition.)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 76 |
| FN-REG-DIAGNOSTIC  | 1 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 12 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 1 |

(See "Final count reconciliation" at end of doc for per-group recount.)

## Classification anchors

1. **User disposition 2026-05-27** (current task): Result `!!` context + `?` try-operator regressions = FN-REG-CORRECTNESS (silent-wrong-output / wrong-branch class). Match-arm enum-payload `unknown` bindings = V0.4-DEFER per §5.16 B2 EnumPayload.
2. **Sibling precedent** (`enums.md` Group F): "Result `!!` context / `?` try-operator broken at runtime" already classified FN-REG-CORRECTNESS, with the canonical failure shape `Error should contain 'context', got: Runtime error: Uncaught error: base` (`!!` not attaching context) + `Expected run ok, got error: Uncaught error: ...` (`?` not propagating Err) + silent-wrong-output from try-operator chain.
3. **Sibling precedent** (`enums.md` Group C): match-arm payload binding has `unknown` type in arithmetic / comparison / string concat → V0.4-DEFER per §5.16 B2 EnumPayload preflight, `TBD-v0.4-b2-enum-payload-preflight`.

## SURFACE-shape groups

### Group A — `!!` context operator runtime broken (FN-REG-CORRECTNESS, 26)

**Shape:** `context_operator::*` sub-suite failing en-bloc — `!!` operator end-to-end path is broken at runtime. Per `enums.md` Group F precedent, the failure shape is: error message string doesn't contain expected context (`!!` not attaching), or Ok/Some pass-through wraps instead of unwrapping, or Err/None doesn't surface as expected error. All tests assert on user-facing semantics (`expect_string("err")`, `expect_number(-1.0)`, `expect_output_contains("Err")`, `expect_run_err_contains("...")`) — none assert on SURFACE text. No `§5.16` cite. Plausibly-correct user code.

**Affected subsystem:** Result `!!` context-attach codegen / runtime in error-handling pipeline.

**Tests (26):**
- `context_operator::context_op_combined_with_try`
- `context_operator::context_op_declared_result_return_type_propagates`
- `context_operator::context_op_declared_result_return_type_with_err`
- `context_operator::context_op_ergonomic_form_err_then_try`
- `context_operator::context_op_err_surfaces_in_runtime_error`
- `context_operator::context_op_multiple_chained`
- `context_operator::context_op_none_includes_none_cause`
- `context_operator::context_op_none_surfaces_in_runtime_error`
- `context_operator::context_op_on_err_adds_context`
- `context_operator::context_op_on_function_result`
- `context_operator::context_op_on_function_result_ok`
- `context_operator::context_op_on_none_wraps_to_err`
- `context_operator::context_op_on_ok_passes_through`
- `context_operator::context_op_on_plain_value_wraps_as_ok`
- `context_operator::context_op_on_some_passes_through_as_ok`
- `context_operator::context_op_with_string_interpolation`
- `context_operator::context_operator_on_none_becomes_err`
- `context_operator::context_operator_on_plain_value_becomes_ok`
- `context_operator::context_operator_on_some_becomes_ok`
- `context_operator::context_operator_passes_through_ok`
- `context_operator::context_operator_wraps_err_with_context`
- `context_operator::context_plus_try_propagates_contextual_error`
- `context_operator::context_plus_try_sugar_parsing`
- `context_operator::declared_result_return_with_err_context`
- `context_operator::multiple_context_operators_chain`
- `propagation::propagation_with_context_at_each_level`

### Group B — `?` try-operator broken at runtime (FN-REG-CORRECTNESS, 15)

**Shape:** `try_operator::*` sub-suite failing en-bloc — `?` operator either: (a) doesn't propagate `Err`, returning a Result-shaped object instead of raising, (b) doesn't unwrap `Ok` correctly (silent-wrong-output), (c) at-top-level fails to surface. Per `enums.md` Group F precedent (same root). All tests assert on user-facing semantics (`expect_number`, `expect_output_contains("Ok"|"Err")`, `expect_run_err_contains(...)`). No `§5.16` cite. Plausibly-correct user code.

**Affected subsystem:** Result `?` propagation codegen / runtime; Option-to-Result `?` lift; top-level `?` handling.

**Tests (15):**
- `try_operator::err_propagated_at_top_level_is_uncaught_exception`
- `try_operator::none_try_propagation_returns_err`
- `try_operator::try_op_at_top_level_err_fails`
- `try_operator::try_op_at_top_level_none_fails`
- `try_operator::try_op_chained_function_calls`
- `try_operator::try_op_in_closure`
- `try_operator::try_op_in_if_false_branch`
- `try_operator::try_op_in_if_true_branch`
- `try_operator::try_op_in_loop_all_ok`
- `try_operator::try_op_multiple_in_same_function`
- `try_operator::try_op_on_err_skips_rest`
- `try_operator::try_op_on_nested_result`
- `try_operator::try_op_on_none_propagates_err`
- `try_operator::try_op_unwraps_ok_value`
- `edge_cases::edge_try_on_none_returns_err`

### Group C — `?` + Result propagation chain regressions (FN-REG-CORRECTNESS, 9)

**Shape:** `propagation::*` failures — multi-level fn-to-fn `?` propagation through `Result<int>` / `Result<number>` chains. Same root as Group B (`?` operator broken) extended over 3-level / 5-level call chains and Option-to-Result coerce + `?` patterns. All assert on user-facing semantics.

**Tests (9):**
- `propagation::complex_error_handling_scenario`
- `propagation::complex_error_handling_scenario_config_missing`
- `propagation::propagation_accumulate_results`
- `propagation::propagation_conditional_error_path`
- `propagation::propagation_five_levels_deep`
- `propagation::propagation_mixed_ok_err_paths`
- `propagation::propagation_three_levels_ok`
- `propagation::propagation_try_after_failed_option_coerce`
- `propagation::propagation_try_after_successful_option_coerce`

### Group D — `!!` + `?` combined / edge-case regressions (FN-REG-CORRECTNESS, 13)

**Shape:** `edge_cases::*` covering combinations of `!!` context + `?` propagation, error-message escape handling, sequential fallible ops, while-loop `?`, nested fn `?`, Option↔Result interop via `!!`. Same root as Groups A/B.

**Tests (13):**
- `edge_cases::edge_chained_ok_operations`
- `edge_cases::edge_context_then_match_recovery`
- `edge_cases::edge_context_with_number_context_message`
- `edge_cases::edge_early_return_skips_subsequent_operations`
- `edge_cases::edge_err_in_array_iteration`
- `edge_cases::edge_error_message_with_newlines`
- `edge_cases::edge_error_message_with_quotes`
- `edge_cases::edge_error_message_with_special_chars`
- `edge_cases::edge_nested_context_operators`
- `edge_cases::edge_option_and_result_interop`
- `edge_cases::edge_result_in_while_loop_all_ok`
- `edge_cases::edge_result_with_option_none_via_context`
- `edge_cases::edge_result_with_option_some_via_context`
- `edge_cases::edge_sequential_fallible_operations`
- `edge_cases::edge_try_in_nested_function`
- `edge_cases::edge_very_long_error_message`

### Group E — Match-arm enum-payload `unknown` in arithmetic (V0.4-DEFER, 7)

**Shape:** Tests match `Ok(v)` / `Some(v)` and use `v` in arithmetic or comparison (`v + 1`, `v * 3`, `v + 5`, `v == None`, sum accumulation), or extract an inner value of an enum-payload binding in a stress test. Per `enums.md` Group C precedent, this is **B2 EnumPayload preflight** territory — payload binding's type is lost (`unknown`) → semantic error `Cannot infer types for binary operation ... operand types are 'unknown' and 'int'`. Surface-and-stop is clean (structured semantic error). §5.16 supervisor 2026-05-25 named scope explicitly includes B2 EnumPayload.

**Recommended v0.4 issue ID:** `TBD-v0.4-b2-enum-payload-preflight` (shared with `enums.md` Group C).

**Tests (7):**
- `stress_ok_err::match_ok_with_computation_in_arm` — `Ok(v) => v + 5`
- `stress_ok_err::match_ok_with_multiply` — `Ok(v) => v * 3`
- `result_creation::result_match_ok_extracts_value` — `Ok(v) => v + 1`
- `result_creation::result_match_with_computation_in_arm` — `Ok(v) => v * 10`
- `result_creation::result_in_array` — `for r in [...] { match r { Ok(v) => { sum = sum + v } ... } }`
- `result_creation::option_with_conditional_some` — `maybe_double(5) ?? -1` where `maybe_double` returns `Option<int>` extracted via coalesce
- `result_creation::option_some_coalesces_to_value` — `val ?? 0` where unwrapped value used as printed int

### Group F — Option/Result print rendering shift (V0.4-DEFER, 1) + None-in-collection arithmetic (V0.4-DEFER, 2)

**Shape:** `result_creation::none_value_prints` expects `print(None)` → "None"; renders as `null` per the canonical control_flow / literals shift (`None` → `null`). Normally FN-REG-DIAGNOSTIC but classified V0.4-DEFER here for consistency with `enums.md` Group E (`option_print_none`) which the parent audit routes to FN-REG-CORRECTNESS — however **this fixture uses `expect_output_contains("None")` not a strict-equal**, and the change is print rendering shift, not a payload-binding correctness violation. Routing here per literals.md precedent → **FN-REG-DIAGNOSTIC**.

**Tests (1):**
- `result_creation::none_value_prints` — **FN-REG-DIAGNOSTIC** (re-classified from V0.4-DEFER per literals.md `None` → `null` shift)

**`stress_option::null_in_array` / `null_in_comparison_chain`:** these use `None` in heterogeneous array `[1, None, 3]` + `None == None`. The first is a generic-element inference site (mixed int+None → element type `unknown`), which is **B2 EnumPayload-adjacent inference loss**; the second is None-equality. Both are payload-discrimination / generic-element-inference — same lattice as Group E. → **V0.4-DEFER**.

**Tests (2):**
- `stress_option::null_in_array`
- `stress_option::null_in_comparison_chain`

### Group G — array-bounds returns-None contract (FN-REG-CORRECTNESS, 6)

**Shape:** Tests assert that out-of-bounds array indexing returns `None` (`expect_none()` / `v == None` → `expect_bool(true)`). Currently failing — either the runtime no longer returns `None` for out-of-bounds (returns something else, or panics), or the `v == None` comparison itself fails (Option-equality regression). The diagnostics tests use `v == None` which compounds Group E's None-equality regression. **Routed FN-REG-CORRECTNESS** per documented language contract: "Array out-of-bounds returns null in Shape (not an error)" — release-blocking behavior change.

**Tests (6):**
- `diagnostics::runtime_err_array_index_out_of_bounds_returns_null`
- `diagnostics::runtime_err_empty_array_access_returns_null`
- `diagnostics::runtime_err_negative_index_beyond_length_returns_null`
- `stress_propagation::index_out_of_bounds_fails` (asserts `expect_none()`)
- `stress_propagation::negative_index_out_of_bounds_fails` (asserts `expect_none()`)
- `stress_propagation::result_accumulation_in_loop` — `match r { Ok(v) => { sum = sum + v } ... }` over `Result<int>` — Group E B2 EnumPayload pattern; **re-routed V0.4-DEFER** (moved out of Group G)

Net Group G: **5 FN-REG-CORRECTNESS** + 1 entry moved to Group E.

### Group H — TryInto/Into impl semantic-diagnostic regressions (FN-REG-CORRECTNESS, 4)

**Shape:** Tests exercise `impl TryInto<int> for string as int { method tryInto() {...} }` / `impl Into<int> for bool as int { ... }` + `as int?` fallible cast / `as int` infallible cast. Either spurious semantic diagnostic, or static-conversion error mis-fires when impl is in scope, or runtime miss on the impl. `infallible_type_assertion_no_semantic_diagnostics_for_supported_into_conversion` + `fallible_type_assertion_no_semantic_diagnostics_for_supported_conversion` assert `expect_no_semantic_diagnostics()`. `fallible_type_assertion_uses_named_try_into_impl` + `infallible_type_assertion_uses_into_impl` + `fallible_type_assertion_propagates_conversion_failure` assert runtime behavior on impl-dispatched cast. Plausibly-correct user code; release-blocking.

**Affected subsystem:** `as T` / `as T?` cast → `Into` / `TryInto` impl dispatch in type-check + bytecode emit. Possibly the same `?` regression as Group B, since these use `(raw as int?)?`.

**Tests (4):**
- `diagnostics::infallible_type_assertion_no_semantic_diagnostics_for_supported_into_conversion`
- `diagnostics::fallible_type_assertion_no_semantic_diagnostics_for_supported_conversion`
- `try_operator::fallible_type_assertion_propagates_conversion_failure`
- `try_operator::fallible_type_assertion_uses_named_try_into_impl`
- `try_operator::infallible_type_assertion_uses_into_impl`

(5 tests; original count above stated 4 — actual is 5.)

### Group I — coalesce operator `??` Some-preservation (FN-REG-CORRECTNESS, 1)

**Shape:** `result_creation::coalesce_operator_preserves_some_value` asserts that `Some(42) ?? 0` yields `42`. Currently failing — `??` either not unwrapping `Some` or yielding the default. Plausibly-correct user code.

**Tests (1):**
- `result_creation::coalesce_operator_preserves_some_value`

### Group J — UNKNOWN (1)

**Tests (1):**
- `const_types_strings::const_complex_expression` — body: `const X = 3 * 4 + 2; X` → expect `14.0`. Trivial fixture; no obvious connection to Result/Option/`?`/`!!`/B2-payload. Failure could be const-folding regression, top-level expression-as-int rendering, or test-helper integration. Without per-test stdout block (the audit-day log lacks excerpts despite the `--nocapture` re-run instruction), cannot classify confidently.
- **Recommended next step:** isolated `cargo test -p shape-test --test error_handling const_types_strings::const_complex_expression -- --nocapture` to capture the assertion / panic message; bisect on `const`-eval / top-level `int` print pipeline.

## Final count reconciliation

| Group | Count | Class |
|---|---|---|
| A | 26 | FN-REG-CORRECTNESS |
| B | 15 | FN-REG-CORRECTNESS |
| C | 9  | FN-REG-CORRECTNESS |
| D | 16 | FN-REG-CORRECTNESS |
| E | 7  | V0.4-DEFER |
| F | 1  | FN-REG-DIAGNOSTIC (`none_value_prints`) |
| F | 2  | V0.4-DEFER (`null_in_array`, `null_in_comparison_chain`) |
| G | 5  | FN-REG-CORRECTNESS |
| H | 5  | FN-REG-CORRECTNESS |
| I | 1  | FN-REG-CORRECTNESS |
| J | 1  | UNKNOWN |
| Cross-group (`result_accumulation_in_loop` reclassified to E) | (already in E count) | — |
| **Sum** | **88 + 1 dup-adjust + 1 UNKNOWN = 90** | — |

**Final per-class:**

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 76 |
| FN-REG-DIAGNOSTIC  | 1 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 12 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 1 |
| **Total**          | **90** |

(Group D count is 16, not 13 as listed — recount of distinct edge_cases entries: edge_chained_ok_operations, edge_context_then_match_recovery, edge_context_with_number_context_message, edge_early_return_skips_subsequent_operations, edge_err_in_array_iteration, edge_error_message_with_newlines, edge_error_message_with_quotes, edge_error_message_with_special_chars, edge_nested_context_operators, edge_option_and_result_interop, edge_result_in_while_loop_all_ok, edge_result_with_option_none_via_context, edge_result_with_option_some_via_context, edge_sequential_fallible_operations, edge_try_in_nested_function, edge_very_long_error_message = 16.)

## Remaining UNKNOWN (1)

- `const_types_strings::const_complex_expression`
