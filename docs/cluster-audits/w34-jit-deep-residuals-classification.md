# Wave 34 JIT Deep Residuals Classification

Source worktree: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch`

Source HEAD: `09ddc04b` (`strict-flip-collection-dispatch`)

Worker worktree: `/home/dev/dev/shape-lang/shape-strict-flip-w34-jit-deep-residuals`

Worker branch: `strict-flip-w34-jit-deep-residuals`

## Reproduction

Command run before any edits:

```sh
env PATH=/nix/store/788mx070y81zjlg5ipcl0cra3afviw9k-gcc-wrapper-15.2.0/bin:/nix/store/0rn2xh3zciwdr8sjg79ir79dbr6lnmfb-gnumake-4.4.1/bin:/nix/store/018l43lbsqbrsasrfdcxxb5s4s161fkr-file-5.45/bin:$PATH MAKE=/nix/store/0rn2xh3zciwdr8sjg79ir79dbr6lnmfb-gnumake-4.4.1/bin/make CC=/nix/store/788mx070y81zjlg5ipcl0cra3afviw9k-gcc-wrapper-15.2.0/bin/cc CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/nix/store/788mx070y81zjlg5ipcl0cra3afviw9k-gcc-wrapper-15.2.0/bin/cc cargo test -p shape-jit --lib --features deep-tests --no-fail-fast
```

Result: `735 passed; 28 failed; 28 ignored`.

The saved reproduction log is `/tmp/w34_shape_jit_deep_tests.log`.

After the OOB store fix below, the same command was rerun and saved to
`/tmp/w34_shape_jit_deep_tests_after.log`.

Post-patch result: `736 passed; 27 failed; 28 ignored`. The removed failure is:

- `mir_compiler::v2_array_tests::v2_array_i64_out_of_bounds_store_raises_error`

## Classifications

| Test | Classification | Evidence / reason |
|---|---|---|
| `mir_compiler::closure_dispatch_regression_tests::option_return_conditional_number_none` | stale deep-test expectation under strict typing | Source returns `7.0` from `-> number?`; strict solver rejects scalar `number` as `Option<T>`. Rewrite with explicit `Some(7.0)` if this fixture remains. |
| `mir_compiler::closure_dispatch_regression_tests::option_return_conditional_number_some` | stale deep-test expectation under strict typing | Same implicit scalar-to-`Option` lift as the `none` variant. |
| `mir_compiler::integration_tests::aggregate_object_spread_simple_baseline` | missing language/compiler feature requiring implementation | Object spread creates user-facing schema `__merged_30_36.z` as `FieldType::Any`; strict schema proof must stamp `z: int` instead of relying on `Any`. |
| `mir_compiler::integration_tests::combined_ackermann_3_4` | missing language/compiler feature requiring implementation | Legacy recursive named-function path completes with `Null` instead of `125`; needs static call graph / recursion lowering, not runtime inference. |
| `mir_compiler::integration_tests::combined_variable_shadowing_in_function` | missing language/compiler feature requiring implementation | Legacy `function` call returns `Null` instead of `15`; same named-function return propagation lane. |
| `mir_compiler::integration_tests::function_calling_function` | missing language/compiler feature requiring implementation | Named function calling another named function returns `Null`. |
| `mir_compiler::integration_tests::function_early_return` | missing language/compiler feature requiring implementation | Untyped legacy function with early return returns `Null`. |
| `mir_compiler::integration_tests::function_early_return_zero_divisor` | missing language/compiler feature requiring implementation | Same early-return legacy function lane. |
| `mir_compiler::integration_tests::function_simple_call` | missing language/compiler feature requiring implementation | Basic legacy `function double(x)` call returns `Null`. |
| `mir_compiler::integration_tests::function_triple_chain` | missing language/compiler feature requiring implementation | Chained named calls return `Null`. |
| `mir_compiler::integration_tests::function_two_params` | missing language/compiler feature requiring implementation | Legacy two-arg named call returns `Null`. |
| `mir_compiler::integration_tests::function_with_local_variables` | missing language/compiler feature requiring implementation | Legacy named function local/return propagation returns `Null`. |
| `mir_compiler::integration_tests::jit_err_path_set_add_non_string_key_surfaces_clean_error` | stale deep-test expectation under strict typing | Test expects VM runtime key-type error from `Set.add(1)`, but strict compile-time proof rejects the source before reaching the VM error path (`Set<int>` field access / missing `Set.size` method, depending on solver path). Keep the no-crash intent by asserting a clean compile error or use a strict-legal runtime-error fixture. |
| `mir_compiler::integration_tests::mutual_recursion_even_odd` | missing language/compiler feature requiring implementation | Recursive legacy functions fail static call proof (`Float64` argument to `Int64` parameter). Implement recursive call-site proof or rewrite fixture to typed `fn` if legacy `function` is no longer in scope. |
| `mir_compiler::integration_tests::mutual_recursion_is_odd` | missing language/compiler feature requiring implementation | Mutual recursion returns `Null`; same named-function recursion lane. |
| `mir_compiler::integration_tests::parity_array_reduce` | stale deep-test expectation under strict typing | Source uses `arr.reduce(0, |acc, x| ...)`; current strict signature is `reduce(f, init)`, so callback must be first. |
| `mir_compiler::integration_tests::parity_higher_order_function` | missing language/compiler feature requiring implementation | Higher-order named function / closure dispatch returns `Null`. |
| `mir_compiler::integration_tests::parity_match_int` | missing language/compiler feature requiring implementation | Integer match expression executes to `Null` instead of selected arm value. |
| `mir_compiler::integration_tests::parity_null_coalescing_non_null` | stale deep-test expectation under strict typing | `let x: number? = 10.0` relies on implicit `Some`; strict source should use `Some(10.0)`. |
| `mir_compiler::integration_tests::parity_option_return` | stale deep-test expectation under strict typing | `return arr[i]` from `-> number?` relies on implicit `Some`; strict source should return `Some(arr[i])`. |
| `mir_compiler::integration_tests::parity_pipe_operator` | stale deep-test expectation under strict typing | Pipeline passes `Int64` literal `5` into an inferred `Float64` parameter. Use `5.0` or typed `int` functions. |
| `mir_compiler::integration_tests::phase_e_escaping_closure_still_correct` | missing language/compiler feature requiring implementation | Escaping closure returned from a function fails call-result proof (`int` vs proven `number`); needs closure return type proof/ABI work. |
| `mir_compiler::integration_tests::phase_e_higher_order_map_pipeline` | stale deep-test expectation under strict typing | Same stale `reduce(init, f)` argument order as `parity_array_reduce`; may expose further HOF work after fixture correction. |
| `mir_compiler::integration_tests::recursive_factorial` | missing language/compiler feature requiring implementation | Recursive legacy function returns `Null`. |
| `mir_compiler::integration_tests::recursive_factorial_10` | missing language/compiler feature requiring implementation | Same recursive named-function lane. |
| `mir_compiler::integration_tests::recursive_power` | missing language/compiler feature requiring implementation | Same recursive named-function lane. |
| `mir_compiler::short_circuit_regression_tests::or_short_circuit_does_not_invoke_rhs_function_call` | missing language/compiler feature requiring implementation | Function-call RHS under `||` fails strict type solving (`bool` vs `int`) inside an `fn main() -> int`; short-circuit lowering/type proof needs to preserve bool result before the enclosing int return. |
| `mir_compiler::v2_array_tests::v2_array_i64_out_of_bounds_store_raises_error` | real JIT/runtime correctness bug | JIT deopts for the known move-semantics surface, then interpreter fallback panics in `TypedArray::set` instead of returning `VMError::IndexOutOfBounds`. Fixed in this lane. |

## Patch Landed In This Lane

The OOB store panic was narrow and high priority. The fix stays at the VM typed-array opcode layer:

- `TypedArraySet*` scalar and char arms now check `index >= len` and return `VMError::IndexOutOfBounds`.
- Generic heap-element `TypedArraySet*` arms now check bounds before `get_unchecked`; on OOB they release the incoming element share.
- The hand-written `TypedArraySetString` arm now performs the same check and releases the newly materialized `StringObj` share on OOB.
- Existing `TypedArraySetCallable` already had this check.

No `TypedArray::set` API rewrite was done; its low-level unchecked/panicking contract remains internal, while user-facing opcode handlers now guard before calling it.

## Recommended Next Lanes

1. `w34-stale-jit-fixtures`
   - Owned files: `crates/shape-jit/src/mir_compiler/{closure_dispatch_regression_tests.rs,integration_tests.rs}`
   - Work: update or ignore stale strict-invalid fixtures only, with precise justification. Covers explicit `Some(...)`, `reduce(f, init)`, pipe numeric proof, and `Set.add(1)` compile-error expectation.

2. `w34-named-function-callgraph`
   - Owned files: `crates/shape-jit/src/mir_compiler/{terminators.rs,rvalues.rs,statements.rs,types.rs}` plus focused tests in `integration_tests.rs`
   - Work: legacy named-function calls returning `Null`, chained calls, locals, early returns, and recursion. No runtime inference; require compile-time call-site and return proof.

3. `w34-object-spread-schema`
   - Owned files: `crates/shape-vm/src/compiler/**/object*`, `crates/shape-runtime/src/type_schema/**`, and the single object-spread JIT fixture
   - Work: stamp spread-merged fields with proven field types instead of `FieldType::Any`.

4. `w34-match-and-short-circuit-proof`
   - Owned files: `crates/shape-vm/src/mir/lowering/expr.rs`, `crates/shape-jit/src/mir_compiler/terminators.rs`, `crates/shape-jit/src/mir_compiler/short_circuit_regression_tests.rs`
   - Work: integer match result propagation and function-call RHS short-circuit type proof.

5. `w34-escaping-closure-hof`
   - Owned files: `crates/shape-jit/src/mir_compiler/{closure*.rs,terminators.rs,types.rs}` and HOF-specific tests
   - Work: escaping closure return proof and higher-order closure dispatch after stale `reduce` fixtures are corrected.
