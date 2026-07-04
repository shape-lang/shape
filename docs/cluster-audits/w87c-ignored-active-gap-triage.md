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

## W91B Addendum: Stale Expectation Cleanup

Date: 2026-07-03
Branch: `strict-flip-w91b-stale-ignored-cleanup`

W91B redrive reclassified the remaining four `stale_semantic_expectation`
ignores as `active_feature_gap` after supervisor focused execution proved the
tests still fail under current strict behavior. No production semantics changed.
The three `functions.rs` rows are also `deep-tests`-gated because they are
inside `#[cfg(all(test, feature = "deep-tests"))] mod tests`.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/functions.rs::test_out_param_extern_c_compiles` | `stale_semantic_expectation` | `active_feature_gap` | Real run fails: `Function 'duckdb_open' expects between 2 and 2 arguments, got 1`; extern-C out-param sugar needs caller-visible arity/stub alignment. |
| `crates/shape-vm/src/compiler/functions.rs::test_out_param_void_return_single_out` | `stale_semantic_expectation` | `active_feature_gap` | Real run fails: `Function 'duckdb_close' expects between 1 and 1 arguments, got 0`; single-out void sugar needs the same caller-visible arity/stub alignment. |
| `crates/shape-vm/src/compiler/functions.rs::test_intrinsic_builtin_blocked_from_user_code` | `stale_semantic_expectation` | `active_feature_gap` | Real run rejects before scope-gating with `(Vec<number>) -> number is not compatible with (Vec<int>) -> number`; internal-intrinsic diagnostics need ordering ahead of strict type solving. |
| `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_4e_matrix_vec_arithmetic_retargets_to_intrinsics` | `stale_semantic_expectation` | `active_feature_gap` | Real run fails: `Type 'Mat' does not implement trait 'Numeric'`; matrix/vector static retargeting is preempted by strict Numeric trait solving. |

The source-level count delta after W91B is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W88D | 16 | 4 | 5 |
| `shape-vm` after W91B | 20 | 0 | 5 |

## W92B Addendum: Extern Out-Param and Intrinsic Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w92b-extern-intrinsic-active-gaps`

W92B removed three `active_feature_gap` ignores after implementing static,
compile-time fixes in the compiler/type-analysis path. No native ABI runtime
bodies or JIT FFI were changed.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/functions.rs::test_out_param_extern_c_compiles` | `active_feature_gap` | active / passing | Type analysis now predeclares native-ABI `out` params with caller-visible non-`out` arity and the existing bytecode stub return shape (`run-p35738-i191407.service`). |
| `crates/shape-vm/src/compiler/functions.rs::test_out_param_void_return_single_out` | `active_feature_gap` | active / passing | Single native-ABI `out` plus `void` return now predeclares the direct out-value return shape (`run-p36283-i192099.service`). |
| `crates/shape-vm/src/compiler/functions.rs::test_intrinsic_builtin_blocked_from_user_code` | `active_feature_gap` | active / passing | Direct `__intrinsic_*` / `__json_*` builtin calls in ordinary user code are rejected before strict type solving, preserving the existing internal-intrinsic scope diagnostic (`run-p36564-i192388.service`). |

## W92C Addendum: Module-Qualified Type Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w92c-module-qualified-type-gaps`

W92C removed the two remaining module-qualified type `active_feature_gap`
ignores in `module_qualified_type_tests.rs`. Static source audit found W88D's
module-qualified struct-literal inference, inline-module analysis prepending,
and static UFCS lowering already cover these residual rows; W92C adds a
compile-shape proof for the trait-method call target so the dynamic
`CallMethod("greet")` fallback cannot silently return.

| Test | Before | After | Decision label |
|---|---|---|---|
| `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_impl_trait` | `active_feature_gap` | active | implemented static module-qualified trait-method receiver proof |
| `crates/shape-vm/src/lib_tests_parts/module_qualified_type_tests.rs::test_module_type_in_let_binding_annotation` | `active_feature_gap` | active | module-qualified let annotation uses strict solver nominal identity |

## W92D Addendum: Monomorphization Active Gap Redrive

Date: 2026-07-03
Branch: `strict-flip-w92d-monomorphization-active-gaps`

W92D removed one `active_feature_gap` ignore after source audit showed the
test expectation had drifted from current strict dispatch. Nested-array
`flatten()` is now a native PHF `CallMethod` implementation with
receiver-derived result type propagation; it is not a generic stdlib
monomorphization and should not populate `MonomorphizationCache`.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/monomorphization/integration_tests.rs::test_nested_generic_call` | `active_feature_gap` | active / current semantics | Rewritten to assert native PHF `flatten()` compiles, does not create a generic specialization key, and evaluates to `10`. |

The cumulative supervisor source-level count delta after W92B + W92C + W92D is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W91B | 20 | 0 | 5 |
| `shape-vm` after W92B | 17 | 0 | 5 |
| `shape-vm` after W92B + W92C | 15 | 0 | 5 |
| `shape-vm` after W92B + W92C + W92D | 14 | 0 | 5 |

## W94B Addendum: Typed-Array Destructure Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w94b-vm-module-matrix-destructure-gaps`

W94B removes `test_block_expr_destructured_binding_still_runs` after
destructuring typed-array literals through the structural typed-array lowering
path. Earlier W94B partial edits to unrelated binary/operator paths were
rejected and not merged.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/functions.rs::test_block_expr_destructured_binding_still_runs` | `active_feature_gap` | active / verified | v2 typed-array destructuring lowers `let [a, b] = [1, 2]` without reverting to deleted carriers. |

Supervisor verification: `shape-vm --lib --features deep-tests
test_block_expr_destructured_binding_still_runs` passed 1/0 in
`run-p209422-i665530.service`.

The source-level count delta after W94B is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W92B + W92C + W92D | 14 | 0 | 5 |
| `shape-vm` after W94B | 13 | 0 | 5 |

## W94A Addendum: Const-Generic Call-Site Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w94a-vm-const-imported-module-gaps`

W94A removes the default-gated const-generic turbofish active gap by adding a
parser/AST carrier for explicit call-site const args and routing literal const
args through the existing static monomorphization path. Negative source tests
pin invalid paths: const args on non-const functions and non-literal const args
reject during compilation rather than falling through to runtime.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs::const_generic_repeat_n_3_end_to_end` | `active_feature_gap` | active / verified | `repeat::<3>(1)` parses by grammar design and specializes to `repeat::int_3`; invalid const-arg paths reject statically. |

The source-level count delta after W94A is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W94B | 13 | 0 | 5 |
| `shape-vm` after W94A | 12 | 0 | 5 |

Supervisor verification: the three W94A focused tests passed 3/0 in
`run-p214502-i671813.service`, and `shape-ast --lib` passed 512/0 in
`run-p216544-i673931.service`.

## W94C Addendum: JIT Captured-Cell Width Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w94c-jit-active-gaps`

W94C removes the two JIT closure-cell `active_feature_gap` ignores after a
source-level root-cause fix. `MirToIR::slot_kinds` still records the semantic
inner value kind used by `read_place` and arithmetic dispatch, but captured-cell
carrier slots now declare/default/store their Cranelift locals at `I64`
pointer width when the physical slot stores an `OwnedMutable`, closure-body
`Shared`, or outer `SharedCow` cell pointer. Lock-gated Shared reads still load
the legacy I64 payload bits, then coerce those bits back to the slot's semantic
kind before arithmetic consumes them. This prevents Bool captures from
truncating the cell pointer to `I8`, F64 shared locals from defining an `I64`
cell pointer into an `F64` local, and F64 Shared reads from missing the native
F64 binop path.

W95C supersedes the three kernel-mode rows. The kernel builders now have a
narrow v2-safe static lowering path for explicit integer-valued return-code
literals. This covers the existing smoke and throughput tests without
reintroducing the deleted general BytecodeToIR translator. Data-field reads,
state mutation, and arbitrary bytecode kernels remain unsupported and return
precise compile-time errors from `compiler/kernel_ir.rs`.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-jit/src/compiler/c2_tests.rs::c2_owned_mut_bool_round_trip` | `active_feature_gap` | active / verified | Captured-cell locals and entry param stores use cell-pointer storage width while preserving Bool inner-kind semantics for reads/writes. |
| `crates/shape-jit/src/compiler/c2_tests.rs::c2_shared_f64_round_trip` | `active_feature_gap` | active / verified | Outer `SharedCow` locals and closure-body `Shared` capture params use cell-pointer storage width while preserving F64 inner-kind semantics. |

The source-level count delta after W94C is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-jit` before W94C | 5 | 0 | 21 |
| `shape-jit` after W94C | 3 | 0 | 21 |

## W95C Addendum: JIT Kernel Return-Code Builder Closure

Date: 2026-07-03
Branch: `strict-flip-w95c-jit-kernel-builders`

W95C removes the remaining three `shape-jit` `active_feature_gap` ignores in
`core.rs` by replacing the v2 kernel-builder stubs with a small
`compiler/kernel_ir.rs` lowering module. The implementation is intentionally
bounded: it validates the single-series or correlated kernel config, then
lowers only an explicit `PushConst` integer-valued return code to the kernel
ABI's `i32` result. Unsupported bytecode shapes, missing constants, non-integer
constants, out-of-range return codes, and invalid config mappings fail at JIT
compile time with an explicit error. This is not a resurrection of the deleted
general BytecodeToIR path, and it does not claim field/state kernel coverage.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-jit/src/core.rs::test_simulation_kernel_compilation` | `active_feature_gap` | active / verified | Constant return-code kernel compiles and returns 0 through the simulation ABI. |
| `crates/shape-jit/src/core.rs::test_kernel_mode_throughput` | `active_feature_gap` | active / verified | Same static return-code kernel exercises the hot ABI call overhead. |
| `crates/shape-jit/src/core.rs::test_correlated_kernel_compilation` | `active_feature_gap` | active / verified | Constant return-code kernel compiles and returns 0 through the correlated ABI. |

The source-level count delta after W95C is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-jit` before W95C | 3 | 0 | 21 |
| `shape-jit` after W95C | 0 | 0 | 21 |

## W95A Addendum: MIR Local Reference Return Enforcement

Date: 2026-07-03
Branch: `strict-flip-w95a-mir-reference-escape`

W95A removes one `shape-vm` `active_feature_gap` ignore after implementing
static MIR enforcement for unannotated local-reference returns. The MIR borrow
solver now admits ReturnSlot reference-escape promotion only when the compiler
supplies a declared borrow-return contract (`-> &T` / `-> &mut T`); otherwise
a local-rooted reference flowing through aliases into SlotId(0) records B0003
`ReferenceEscape`. ModuleBindingStore promotion remains unchanged.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/compiler/functions.rs::test_compile_function_records_mir_reference_escape` | `active_feature_gap` | active / verified | Unannotated `let r = &x; let alias = r; return alias` is rejected through MIR borrow analysis with B0003, and the analysis records `BorrowErrorKind::ReferenceEscape`. Redrive tightened promotion emission so rejected param-rooted ReturnSlot escapes leave no storage-planning promotion trigger. |

The source-level count delta after W95A is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` before W95A | 12 | 0 | 5 |
| `shape-vm` after W95A | 11 | 0 | 5 |

## W95B Addendum: Extend Resolution Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w95b-extend-resolution`

W95B removes two `shape-vm` `active_feature_gap` ignores after implementing
static extend-block proofs for bare `Vec` receiver specialization, mixed
builtin receiver registrations, and chained Number extend calls without
runtime `CallMethod("multiply")` fallback.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/executor/tests/extend_blocks.rs::test_extend_array_basic` | `active_feature_gap` | active / verified | Bare `extend Vec` methods specialize against the call-site receiver element type. |
| `crates/shape-vm/src/executor/tests/extend_blocks.rs::test_extend_multiple_types` | `active_feature_gap` | active / verified | Builtin receiver extension registration preserves Number, String, and Vec methods in the same program. |

The source-level count delta after W95B is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W95A | 11 | 0 | 5 |
| `shape-vm` after W95B | 9 | 0 | 5 |

## W96C Addendum: Imported Comptime Gap Closure

Date: 2026-07-03
Branch: `strict-flip-w96c-imported-comptime`

W96C removes three default-gated imported-module `active_feature_gap` ignores.
Imported const-specialized clones now keep annotations intact so comptime
`set return (expr)` handlers run on the call-site clone, and post-comptime
return metadata is synchronized to the structural side table.

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_comptime_set_return_expr_via_module_export` | `active_feature_gap` | active / verified | Imported const specialization compiles with the module-export-backed `set return (expr)` annotation intact. |
| `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_comptime_handler_can_call_comptime_helper_fn` | `active_feature_gap` | active / verified | Imported annotation handlers can call comptime helper functions during const-specialized compilation. |
| `crates/shape-vm/src/lib_tests_parts/extension_integration_tests.rs::test_imported_module_typed_callable_field_propagates_table_schema_for_filter_chain` | `active_feature_gap` | active / verified | Imported `set return` schema metadata propagates `Table<T>` into the downstream filter closure. |

The source-level count delta after W96C is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W95B | 9 | 0 | 5 |
| `shape-vm` after W96C | 6 | 0 | 5 |

W96A/W96B follow-up: the final six `shape-vm` active gaps were implemented and
unignored after focused supervisor verification:

| Test | Before | After | Disposition |
|---|---|---|---|
| `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_nested_module_function_resolution` | `active_feature_gap` | active / verified | Qualified nested module calls now project structural return kinds without runtime inference. |
| `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_module_function_recursion` | `active_feature_gap` | active / verified | Module-local bare recursive calls are statically qualified unless shadowed. |
| `crates/shape-vm/src/executor/tests/module_deep_tests.rs::test_module_exec_module_with_match_expression` | `active_feature_gap` | active / verified | Module-local match lowering preserves statically inferred return shape. |
| `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_3b_datetime_arithmetic_retargets_to_call_method` | `active_feature_gap` | active / verified | DateTime/Duration aliases now agree before strict operator retargeting. |
| `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_4e_matrix_vec_arithmetic_retargets_to_intrinsics` | `active_feature_gap` | active / verified | Matrix/vector arithmetic retargets through compile-time type proof. |
| `crates/shape-vm/src/executor/tests/operator_overload.rs::test_r5_4e_mat_add_runtime_returns_correct_values` | `active_feature_gap` | active / verified | `Mat+Mat` returns the schema-backed matrix carrier expected by nested indexing. |

The source-level count delta after W96A/W96B is:

| Crate | Active gap | Stale expectation | Deleted v1 path |
|---|---:|---:|---:|
| `shape-vm` after W96C | 6 | 0 | 5 |
| `shape-vm` after W96A/W96B | 0 | 0 | 5 |

## Active Gap Inventory

No current `active_feature_gap` ignores remain after W96A/W96B. Future active
gap rows should be treated as new drift and should not be ignored without a
precise missing feature, a supervisor-owned focused cargo lane, and an updated
classification entry.

## Stale Expectation Inventory

No current `stale_semantic_expectation` ignores remain after W91B. Future stale
rows should be treated as new drift and either rewritten to current semantics,
deleted if they describe retired paths, or reclassified as active feature gaps
with a concrete missing feature.

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
