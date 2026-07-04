# W94B VM Module/Matrix/Destructure Gap Classification

Baseline: `f2dfb581`
Worktree: `shape-strict-flip-w94b-vm-module-matrix-destructure-gaps`
Date: 2026-07-03

Scope rule: no runtime inference/probing, no JIT/value/wire edits, no build/test lane.

## Closed by this patch

- `functions.rs::test_block_expr_destructured_binding_still_runs`
  - Classification: narrow compiler-lowering defect.
  - Fix: array destructuring now derives the v2 `TypedArrayKind` from the initializer's stamped `Vec<T>`/`Array<T>` type info, records that kind on the compiler temporary, and emits `TypedArrayGet*` instead of `GetProp` for statically proven typed-array destructure reads. Redrive correction: typed-array destructure indexes are emitted as integer constants, while the legacy `GetProp` path keeps number constants.
  - Ignore status: unignored.

## Ignored-test count delta

- `scripts/check-ignored-test-classification.py` is updated for exactly this one shape-vm unignore: `active_feature_gap` 14 -> 13 and deep-tests source-only 57 -> 56.
- No shape-jit expected counts are changed here. When stacked on a supervisor branch that already has W94C merged, apply this W94B shape-vm delta on top of W94C's shape-jit count changes.

## Not changed / Still ignored

- `operator_overload.rs::test_r5_4e_matrix_vec_arithmetic_retargets_to_intrinsics`
  - Classification: active feature gap.
  - Reason: the real gate reports `Type 'Mat' does not implement trait 'Numeric'`; the test remains ignored with the prior recognized `matrix/vector arithmetic retarget` reason. The unproven `Vec<T>`/`Array<T>` concat classifier change was removed from W94B.

- `operator_overload.rs::test_r5_3b_datetime_arithmetic_retargets_to_call_method`
  - Classification: active feature gap.
  - Reason: the real gate reports `(DateTime, Duration) -> DateTime is not compatible with (DateTime, duration) -> DateTime`; the test remains ignored with the prior recognized `temporal arithmetic retarget` reason.

- `functions.rs::test_compile_function_records_mir_reference_escape`
  - Classification: active feature gap.
  - Reason: not changed in this redrive; the prior recognized MIR reference-escape ignore reason is preserved to keep `stale_semantic_expectation` at 0.

- `extend_blocks.rs::test_extend_array_basic`
  - Classification: active feature gap outside a narrow static-only patch.
  - Reason: `extend Vec { method sum() { self[i] + ... } }` has no receiver-element type parameter, so `self[index]` inside the generic body has no strict scalar proof. A correct close needs receiver-specific generic specialization or an annotated `extend Vec<T>` body, not runtime inference.

- `extend_blocks.rs::test_extend_multiple_types`
  - Classification: active multi-extend resolver gap.
  - Reason: mixed Number/String/Vec extension dispatch depends on preserving all user extension registrations through UFCS and method fallback while also avoiding native array PHF precedence. That is broader than a one-site static metadata fix.

- `module_deep_tests.rs::test_module_exec_nested_module_function_resolution`
  - Classification: active module return-kind inference gap.
  - Reason: arithmetic over multiple qualified module calls requires return-kind propagation for nested module function references. Existing passing tests cover single nested calls; the failing shape is the combination in arithmetic.

- `module_deep_tests.rs::test_module_exec_module_function_recursion`
  - Classification: active module-scope name-resolution gap.
  - Reason: unqualified recursive calls inside module functions must resolve to the current module's function symbol. This is a module resolver rule, not a runtime fallback.

- `module_deep_tests.rs::test_module_exec_module_with_match_expression`
  - Classification: active module-scoped match lowering/inference gap.
  - Reason: the module helper's match expression currently loses the selected numeric arm shape. A correct close needs module-function match expression return inference, not runtime probing.

- `operator_overload.rs::test_r5_4e_mat_add_runtime_returns_correct_values`
  - Classification: runtime carrier gap, outside W94B production territory.
  - Reason: compile-time Mat+Mat retarget emission is static; this test checks the runtime carrier returned by `IntrinsicMatAdd` and nested indexing. Fixing it would require runtime/value/wire work prohibited for this worker.

## Verification Deferred

Per W94B dispatch instructions, no `cargo`, `nextest`, `just`, `rustc`, `miri`, `shape-test`, build command, or test binary was run. The supervisor owns the serialized verification lane.
