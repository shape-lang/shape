# error_handling classification

**HEAD:** 82f049dd
**Total tests in binary:** 441
**Passed:** ~351 / Failed: 90 / Ignored: 0 (truncated count — see Note)
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test error_handling --no-fail-fast 2>&1`

> **Audit note:** The provided log at `/tmp/audit_logs/error_handling.log` is **truncated at line 452** — it contains the full test-name list (with FAILED markers) but does not include the per-test failure-detail blocks (`---- name stdout ----` + panic excerpts). Per audit discipline ("every classification decision MUST be backed by an actual test-output excerpt"), tests for which I cannot quote a failure excerpt route to UNKNOWN. The strong test-name patterns suggest the rough distribution noted in the summary below, but the per-test classification requires a re-run with full output.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 90 |

## Per-test classification

All 90 failing tests are routed to **UNKNOWN** because the audit log lacks per-test failure-detail blocks. Listing by sub-suite to make follow-up re-classification mechanical once the full log is available.

### context_operator::* (26 tests)

- `context_op_combined_with_try`, `context_op_declared_result_return_type_propagates`, `context_op_declared_result_return_type_with_err`, `context_operator_on_none_becomes_err`, `context_operator_on_plain_value_becomes_ok`, `context_operator_on_some_becomes_ok`, `context_operator_passes_through_ok`, `context_operator_wraps_err_with_context`, `context_op_ergonomic_form_err_then_try`, `context_op_err_surfaces_in_runtime_error`, `context_op_multiple_chained`, `context_op_none_includes_none_cause`, `context_op_none_surfaces_in_runtime_error`, `context_op_on_err_adds_context`, `context_op_on_function_result`, `context_op_on_function_result_ok`, `context_op_on_none_wraps_to_err`, `context_op_on_ok_passes_through`, `context_op_on_plain_value_wraps_as_ok`, `context_op_on_some_passes_through_as_ok`, `context_op_with_string_interpolation`, `context_plus_try_propagates_contextual_error`, `context_plus_try_sugar_parsing`, `declared_result_return_with_err_context`, `multiple_context_operators_chain`

Class: **UNKNOWN** — blocked on missing failure excerpts. Whole sub-suite failing en-bloc strongly suggests a single shared regression in the `!!` (error-context) operator end-to-end path. Next step: re-run with `--nocapture` + capture failure-detail blocks; expect this to collapse into 1–2 FN-REG-CORRECTNESS entries.

### diagnostics::* (5 tests)

- `fallible_type_assertion_no_semantic_diagnostics_for_supported_conversion`, `infallible_type_assertion_no_semantic_diagnostics_for_supported_into_conversion`, `runtime_err_array_index_out_of_bounds_returns_null`, `runtime_err_empty_array_access_returns_null`, `runtime_err_negative_index_beyond_length_returns_null`

Class: **UNKNOWN** — blocked on missing failure excerpts. Test-name shape suggests: (a) `*as_int` / `try_into` diagnostic-shape changes (FN-REG-DIAGNOSTIC candidates) + (b) `arr[i]` out-of-bounds behavior change (FN-REG-CORRECTNESS or DIAGNOSTIC depending on whether the test expected `null` and now sees a thrown error or vice-versa). Need failure excerpts.

### edge_cases::* (18 tests)

- `edge_chained_ok_operations`, `edge_context_then_match_recovery`, `edge_context_with_number_context_message`, `edge_early_return_skips_subsequent_operations`, `edge_err_in_array_iteration`, `edge_error_message_with_newlines`, `edge_error_message_with_quotes`, `edge_error_message_with_special_chars`, `edge_nested_context_operators`, `edge_ok_with_complex_value`, `edge_option_and_result_interop`, `edge_result_in_while_loop_all_ok`, `edge_result_with_option_none_via_context`, `edge_result_with_option_some_via_context`, `edge_sequential_fallible_operations`, `edge_try_in_nested_function`, `edge_try_on_none_returns_err`, `edge_very_long_error_message`

Class: **UNKNOWN** — Most appear to exercise `!!` (context) + `?` (try) interop; large overlap with the context_operator sub-suite. Same root cause likely.

### propagation::* (9 tests)

- `complex_error_handling_scenario`, `complex_error_handling_scenario_config_missing`, `propagation_accumulate_results`, `propagation_conditional_error_path`, `propagation_five_levels_deep`, `propagation_mixed_ok_err_paths`, `propagation_three_levels_ok`, `propagation_try_after_failed_option_coerce`, `propagation_try_after_successful_option_coerce`, `propagation_with_context_at_each_level`

Class: **UNKNOWN** — `?` propagation through Result chains and Option↔Result coerce. Probably FN-REG-CORRECTNESS on the try-operator path.

### result_creation::* (7 tests)

- `coalesce_operator_preserves_some_value`, `none_value_prints`, `option_some_coalesces_to_value`, `option_with_conditional_some`, `result_in_array`, `result_match_ok_extracts_value`, `result_match_with_computation_in_arm`

Class: **UNKNOWN** — `none_value_prints` is likely the same `None` → `null` diagnostic-rendering shift seen in literals/control_flow (FN-REG-DIAGNOSTIC); others likely Result/Option match-arm value-extraction regressions (FN-REG-CORRECTNESS). Cannot confirm without excerpts.

### stress_ok_err::* (2 tests)

- `match_ok_with_computation_in_arm`, `match_ok_with_multiply`

Class: **UNKNOWN** — Pattern-matching arm with computation. Probably the same `unknown × int` strict-typing inference regression seen elsewhere.

### stress_option::* (2 tests)

- `null_in_array`, `null_in_comparison_chain`

Class: **UNKNOWN**

### stress_propagation::* (3 tests)

- `index_out_of_bounds_fails`, `negative_index_out_of_bounds_fails`, `result_accumulation_in_loop`

Class: **UNKNOWN** — array out-of-bounds semantics + Result accumulation in a loop. Likely 2 root causes.

### try_operator::* (18 tests)

- `err_propagated_at_top_level_is_uncaught_exception`, `fallible_type_assertion_propagates_conversion_failure`, `fallible_type_assertion_uses_named_try_into_impl`, `infallible_type_assertion_uses_into_impl`, `none_try_propagation_returns_err`, `try_op_at_top_level_err_fails`, `try_op_at_top_level_none_fails`, `try_op_chained_function_calls`, `try_op_in_closure`, `try_op_in_if_false_branch`, `try_op_in_if_true_branch`, `try_op_in_loop_all_ok`, `try_op_multiple_in_same_function`, `try_op_on_err_skips_rest`, `try_op_on_nested_result`, `try_op_on_none_propagates_err`, `try_op_unwraps_ok_value`

Class: **UNKNOWN** — sub-suite failing en-bloc strongly suggests a single regression in the `?` (try) operator. Likely FN-REG-CORRECTNESS once excerpts are available.

### const_types_strings::const_complex_expression (1 test)

Class: **UNKNOWN**
