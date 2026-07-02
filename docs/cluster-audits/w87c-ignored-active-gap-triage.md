# W87C Ignored Active-Gap Triage

Date: 2026-07-02
Branch: `strict-flip-w87c-ignored-active-gap-triage`
Base: `2997b11b`

## Scope

This slice inventories the `active_feature_gap` and
`stale_semantic_expectation` ignores enforced by
`scripts/check-ignored-test-classification.py`.

No tests were unignored. The safe narrow action in this cheap-check-only slice
was to reclassify two stale rows as `deleted_v1_path` because they depend on
retired v1 execution paths rather than a current behavior expectation:

| Test | Before | After | Decision label |
|---|---|---|---|
| `crates/shape-vm/src/executor/tests/mod.rs::test_array_index_assignment_preserves_copy_on_write_aliasing` | `stale_semantic_expectation` | `deleted_v1_path` | deleted v1 path |
| `crates/shape-jit/src/worker.rs::test_backend_whole_function_invalid_id` | `stale_semantic_expectation` | `deleted_v1_path` | deleted v1 path |

The source-level count deltas are:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` before W87C | 18 | 5 | 4 |
| `shape-vm` after W87C | 18 | 4 | 5 |
| `shape-jit` before W87C | 5 | 1 | 20 |
| `shape-jit` after W87C | 5 | 0 | 21 |

## W88D Addendum: Module-Qualified Method Gap Closure

Date: 2026-07-02
Branch: `strict-flip-w88d-active-gap-module-qualified`

W88D removed two `active_feature_gap` ignores after implementing static
module-qualified method resolution and proving the lowering with focused
deep-tests:

| Test | Before | After | Decision label |
|---|---|---|---|
| `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_extend_method` | `active_feature_gap` | active / passing | implemented static module-qualified extend-method receiver proof |
| `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_struct_with_method_chaining` | `active_feature_gap` | active / passing | implemented static module-qualified method-chain return proof |

The source-level count delta after W88D is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W87C | 18 | 4 | 5 |
| `shape-vm` after W88D | 16 | 4 | 5 |

## Active Gap Inventory

These rows are precise missing-feature or implementation-gap issues. They
should not be unignored without implementing the named feature and running a
focused cargo test lane.

| Crate | Test | Gate | Decision label | Missing feature |
|---|---|---|---|---|
| `shape-vm` | `crates/shape-vm/src/compiler/functions.rs::test_block_expr_destructured_binding_still_runs` | default | missing language feature | v2 typed-array element handling for destructuring `let [a, b] = [1, 2]`. |
| `shape-vm` | `crates/shape-vm/src/compiler/functions.rs::test_compile_function_records_mir_reference_escape` | default | missing language feature | MIR reference-escape enforcement for local-return shapes. |
| `shape-vm` | `crates/shape-vm/src/compiler/monomorphization/integration_tests.rs::test_nested_generic_call` | default | missing language feature | Flatten monomorphization-cache population for nested generic calls. |
| `shape-vm` | `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs::const_generic_repeat_n_3_end_to_end` | default | missing language feature | Call-site turbofish grammar for const generics (`::<N>`). |
| `shape-vm` | `crates/shape-vm/src/executor/tests/extend_blocks.rs::test_extend_array_basic` | `deep-tests` | missing language feature | Typed receiver-specific method resolution for generic Vec extensions. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/extend_blocks.rs::test_extend_multiple_types` | `deep-tests` | missing language feature | Multi-extend resolver preserving String method registration when mixed with Number and Vec extensions. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_nested_module_function_resolution` | `deep-tests` | missing language feature | Nested qualified module-call return-kind inference. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_module_function_recursion` | `deep-tests` | missing language feature | Unqualified recursive calls inside modules resolving to the module function. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_module_with_match_expression` | `deep-tests` | missing language feature | Module-scoped match lowering preserving per-arm numeric shape. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_3b_datetime_arithmetic_retargets_to_call_method` | `deep-tests` | missing language feature | DateTime/Duration type-alias agreement under strict kind solving. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_4e_mat_add_runtime_returns_correct_values` | `deep-tests` | missing language feature | Matrix runtime carrier retarget so `Mat+Mat` returns a nested-indexable matrix carrier. |
| `shape-vm` | `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_comptime_set_return_expr_via_module_export` | default | missing language feature | Const-specialization for imported module functions. |
| `shape-vm` | `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_comptime_handler_can_call_comptime_helper_fn` | default | missing language feature | Const-specialization for imported module functions. |
| `shape-vm` | `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_typed_callable_field_propagates_table_schema_for_filter_chain` | default | missing language feature | Imported-module annotation `set return` schema propagation across the comptime boundary. |
| `shape-vm` | `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_impl_trait` | default | missing language feature | Qualified type trait-method resolution without treating the module as the receiver. |
| `shape-vm` | `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_type_in_let_binding_annotation` | default | missing language feature | Module-qualified type annotations agreeing with strict solver module/type identity. |
| `shape-jit` | `crates/shape-jit/src/compiler/c2_tests.rs::c2_owned_mut_bool_round_trip` | `deep-tests` | missing language feature | JIT capture-local declaration/param-store must preserve cell-pointer width for OwnedMutable captures. |
| `shape-jit` | `crates/shape-jit/src/compiler/c2_tests.rs::c2_shared_f64_round_trip` | `deep-tests` | missing language feature | JIT shared-local declaration must preserve `*const SharedCell` pointer width. |
| `shape-jit` | `crates/shape-jit/src/core.rs::test_simulation_kernel_compilation` | default | missing language feature | `build_kernel_ir` v2 runtime migration. |
| `shape-jit` | `crates/shape-jit/src/core.rs::test_kernel_mode_throughput` | default | missing language feature | `build_kernel_ir` v2 runtime migration. |
| `shape-jit` | `crates/shape-jit/src/core.rs::test_correlated_kernel_compilation` | default | missing language feature | `build_correlated_kernel_ir` v2 runtime migration. |

## Stale Expectation Inventory

These are not safe unignore candidates in a cheap-check-only slice. They need a
language/book or diagnostic-policy decision before someone either rewrites the
test or removes the stale expectation.

| Crate | Test | Gate | Decision label | Stale expectation |
|---|---|---|---|---|
| `shape-vm` | `crates/shape-vm/src/compiler/functions.rs::test_out_param_extern_c_compiles` | default | stale test expectation | Extern-C out-param sugar no longer matches current strict call arity checking. |
| `shape-vm` | `crates/shape-vm/src/compiler/functions.rs::test_out_param_void_return_single_out` | default | stale test expectation | Extern-C single-out/void sugar no longer matches current strict call arity checking. |
| `shape-vm` | `crates/shape-vm/src/compiler/functions.rs::test_intrinsic_builtin_blocked_from_user_code` | default | stale test expectation | Old intrinsic scope-gating diagnostic is now preempted by strict type solving. |
| `shape-vm` | `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_4e_matrix_vec_arithmetic_retargets_to_intrinsics` | `deep-tests` | stale test expectation | Mat operands no longer satisfy the old Numeric trait test shape. |

## Deleted Path Reclassifications

These were the smallest safe W87C edits. They remain ignored, but the checker
now classifies them under `deleted_v1_path` instead of stale semantics.

| Crate | Test | Decision label | Reason |
|---|---|---|---|
| `shape-vm` | `crates/shape-vm/src/executor/tests/mod.rs::test_array_index_assignment_preserves_copy_on_write_aliasing` | deleted v1 path | The test placeholder describes v1 `VMArray` alias-preservation behavior and still depends on deleted host-tier `ValueWord` helpers. A future test should be a v2 mutation/share test. |
| `shape-jit` | `crates/shape-jit/src/worker.rs::test_backend_whole_function_invalid_id` | deleted v1 path | The invalid-id request belongs to retired Tier 1 whole-function JIT (`compile_single_function`), while the current selective JIT path does not expose that request shape. |

## Process-Aborting Extern-C Tests

No process-aborting tests were unignored or changed. The three
`process_aborting_extern_c_todo` rows remain documented in
`docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md` and should
stay ignored until their `extern "C"` SURFACE functions return structured
errors instead of unwinding across the ABI boundary.
