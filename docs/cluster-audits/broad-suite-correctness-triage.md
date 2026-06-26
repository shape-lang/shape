# Broad-suite correctness triage

Date: 2026-06-26
Worktree: `/home/dev/dev/shape-lang/shape-broad-suite-triage`
Branch: `strict-flip-broad-suite-triage`
Current HEAD: `a1ff3be027a80ec9f59fcb7f27c39e28867cabb5`

## Scope

This cluster is doc/script-only. I did not modify `crates/**`, `tools/**`, or
`tests/**`.

The local coordination-base comparison was run against
`/home/dev/dev/shape-lang/shape-l5-coordination-base`, whose HEAD at triage
time was `19d8ae359e8b0bcf27a9b58f566d2f43db5158e5` (`Create L5
coordination base snapshot`). That fresh local comparison reproduced the key
ratchet property (`current_only=0`) but produced `base_only=568`, not the
supervisor checkpoint's remembered `base_only=584`. The artifact-backed result
below is therefore the authoritative result for the base checkout present on
disk during this triage.

## Commands

Current worktree:

```sh
mkdir -p /tmp/shape-broad-suite-triage/current /tmp/shape-broad-suite-triage/base
direnv exec /home/dev/dev/shape-lang/shape-broad-suite-triage cargo test -p shape-runtime --lib --no-fail-fast > /tmp/shape-broad-suite-triage/current/shape-runtime-lib.log 2>&1
direnv exec /home/dev/dev/shape-lang/shape-broad-suite-triage cargo test -p shape-test --test book_policy --no-fail-fast > /tmp/shape-broad-suite-triage/current/shape-test-book-policy.log 2>&1
direnv exec /home/dev/dev/shape-lang/shape-broad-suite-triage cargo test -p shape-vm --lib --no-fail-fast > /tmp/shape-broad-suite-triage/current/shape-vm-lib.log 2>&1
scripts/compare_cargo_failures.sh --log runtime=/tmp/shape-broad-suite-triage/current/shape-runtime-lib.log --log vm=/tmp/shape-broad-suite-triage/current/shape-vm-lib.log --log book_policy=/tmp/shape-broad-suite-triage/current/shape-test-book-policy.log --write-dir /tmp/shape-broad-suite-triage/current/failure-sets > /tmp/shape-broad-suite-triage/current/parse-current.txt
```

Coordination base, with target output isolated to `/tmp`:

```sh
direnv exec /home/dev/dev/shape-lang/shape-l5-coordination-base env CARGO_TARGET_DIR=/tmp/shape-broad-suite-triage/base-target cargo test -p shape-runtime --lib --no-fail-fast > /tmp/shape-broad-suite-triage/base/shape-runtime-lib.log 2>&1
direnv exec /home/dev/dev/shape-lang/shape-l5-coordination-base env CARGO_TARGET_DIR=/tmp/shape-broad-suite-triage/base-target cargo test -p shape-vm --lib --no-fail-fast > /tmp/shape-broad-suite-triage/base/shape-vm-lib.log 2>&1
scripts/compare_cargo_failures.sh --log current_runtime=/tmp/shape-broad-suite-triage/current/shape-runtime-lib.log --log base_runtime=/tmp/shape-broad-suite-triage/base/shape-runtime-lib.log --compare current_runtime:base_runtime --write-dir /tmp/shape-broad-suite-triage/base/failure-sets > /tmp/shape-broad-suite-triage/base/parse-runtime-compare.txt
scripts/compare_cargo_failures.sh --log current_vm=/tmp/shape-broad-suite-triage/current/shape-vm-lib.log --log base_vm=/tmp/shape-broad-suite-triage/base/shape-vm-lib.log --compare current_vm:base_vm --write-dir /tmp/shape-broad-suite-triage/base/failure-sets > /tmp/shape-broad-suite-triage/base/parse-vm-compare.txt
```

Verification:

```sh
git diff --check
```

## Artifacts

Raw logs:

- `/tmp/shape-broad-suite-triage/current/shape-runtime-lib.log`
- `/tmp/shape-broad-suite-triage/current/shape-test-book-policy.log`
- `/tmp/shape-broad-suite-triage/current/shape-vm-lib.log`
- `/tmp/shape-broad-suite-triage/base/shape-runtime-lib.log`
- `/tmp/shape-broad-suite-triage/base/shape-vm-lib.log`

Parsed failure sets:

- `/tmp/shape-broad-suite-triage/current/failure-sets/runtime.failures`
- `/tmp/shape-broad-suite-triage/current/failure-sets/vm.failures`
- `/tmp/shape-broad-suite-triage/current/failure-sets/book_policy.failures`
- `/tmp/shape-broad-suite-triage/current/vm-surface.failures`
- `/tmp/shape-broad-suite-triage/current/vm-nonsurface.failures`

Comparison outputs:

- `/tmp/shape-broad-suite-triage/base/parse-runtime-compare.txt`
- `/tmp/shape-broad-suite-triage/base/parse-vm-compare.txt`
- `/tmp/shape-broad-suite-triage/base/failure-sets/current_runtime_only_vs_base_runtime.failures`
- `/tmp/shape-broad-suite-triage/base/failure-sets/base_runtime_only_vs_current_runtime.failures`
- `/tmp/shape-broad-suite-triage/base/failure-sets/current_vm_only_vs_base_vm.failures`
- `/tmp/shape-broad-suite-triage/base/failure-sets/base_vm_only_vs_current_vm.failures`

## Results

| Suite | Current result | Current failures | Base result | Failure-name diff |
|---|---:|---:|---:|---:|
| `shape-runtime --lib` | 1180 pass / 5 fail | 5 | 1173 pass / 5 fail | current-only 0, base-only 0 |
| `shape-vm --lib` | 2077 pass / 249 fail / 57 ignored | 249 | 1498 pass / 817 fail / 57 ignored | current-only 0, base-only 568 |
| `shape-test --test book_policy` | 3 pass / 0 fail | 0 | not rerun | n/a |

The broad-suite ratchet is clean: no current runtime or VM failure name is new
relative to the local coordination-base checkout. The VM branch removed 568
base failure names in this fresh comparison.

## Runtime failures

All five `shape-runtime --lib` failures are in
`crates/shape-runtime/src/type_system/inference/**` and the same five fail on
the local coordination base:

- `try_operator_unwraps_ok_constructor_call`: `Ok(1)?` infers `T0` instead of
  `int`.
- `test_expression_style_ok_union_infers_result_inner_union`: expression-style
  `Ok` return collapses the inner union to `string`.
- `test_numeric_body_constraint_refines_unannotated_param_type`: unannotated
  param constrained by `c + 1` no longer refines to `number`.
- `test_numeric_body_constraint_rejects_non_numeric_callsite`: object callsite
  is accepted into an `int | object` union instead of rejected.
- `test_best_effort_preserves_callsite_unions_under_numeric_conflict`: same
  numeric/object conflict is not emitted in best-effort mode.

Likely owner: runtime inference solver, especially fallible constructor
unwrapping, callsite-union preservation, and numeric body-constraint
propagation. These are real strict-typing semantics failures, not stale tests.

## VM failure taxonomy

The 249 current VM failures split into:

| Class | Count | Notes |
|---|---:|---|
| Explicit `phase-2c` / `SURFACE` stop | 201 | Mostly test bodies intentionally replaced with `todo!()` pending host-tier kinded eval/marshal/constant APIs, plus typed-array construction-cascade `NotImplemented` surfaces. |
| Non-surface correctness / stale-test / diagnostic failures | 48 | Smaller set needing owner triage below. |

Surface bucket counts:

| Count | Bucket | Likely owner |
|---:|---|---|
| 38 | `executor::tests::matrix_ops` | Phase-2c host-tier kinded eval/marshal and kinded constant table; tests currently `todo!()`. |
| 29 | `executor::tests::v2_opcode_tests` | Same host-tier ValueWord/`Constant::Value` deletion fallout. |
| 21 | `executor::tests::typed_array_ops` | Same host-tier plus typed-array construction cascade. |
| 19 | `executor::tests::set_ops` | Host-tier set method harness. |
| 14 | `executor::tests::table_iteration` | Kinded constant variant / table iteration host harness. |
| 14 | `executor::tests::channel_ops` | Host-tier channel carrier/marshal rebuild. |
| 12 | `executor::tests::try_operator` | Deleted execute helpers / KindedSlot heap accessors. |
| 12 | `executor::tests::deque_ops` | Host-tier deque harness. |
| 11 | `executor::tests::priority_queue_ops` | Host-tier priority queue harness. |
| 11 | `executor::tests::io_integration` | Host-tier IO marshal plus one non-surface export assertion below. |
| 8 | `executor::tests::type_system_integration` | Typed-array construction surfaces under table/content examples. |
| 7 | `executor::tests::decimal_ops` | Host-tier decimal marshal. |
| 5 | window/array misc | Typed-array `op_new_array` construction-cascade surfaces. |

Per `docs/cluster-audits/v0.3-classification/TRUTH-SET.md`, many of these
SURFACE families are still release-blocking SCOPE-RECLAIM, not v0.4 deferrals.
They are not book-policy failures, but the current classification docs treat
V3-S5 `op_new_array`/typed-array cascade and phase-2c host-tier marshal work as
pulled into v0.3.3 correctness scope.

Non-surface bucket counts:

| Count | Bucket | Evidence | Likely owner / disposition |
|---:|---|---|---|
| 9 | `executor::tests::type_system_integration` | Queryable trait fixtures fail to parse old comma method syntax; Table row/select fixtures report `count`/`filter`/`select` not found or `Table<Record>` cannot have fields. | Split: two stale-syntax fixtures contradict current book syntax; remaining table method registry/type inference is real `tables_queryable` correctness. |
| 7 | `compiler::v2_typed_emission` | Heterogeneous arrays now fail constraints (`int` incompatible with `string`/`bool`) where tests expect legacy fallback; two clean-error tests still do not match expected diagnostics. | Compiler typed-emission / array-literal fallback policy. Needs decision: support heterogeneous `Array<any|union>` or update stale fallback tests. |
| 6 | `executor::tests::mutation_writeback` | Set methods not found on `Set`; one bytecode expectation fails for `Dup; StoreLocal`. | Method registry/stdlib collection methods plus writeback codegen. |
| 4 | `compiler::patterns` | Struct constructor-pattern tests rejected as enum-only variant patterns. | Pattern/object destructuring. Current truth-set says object destructuring is v0.3 scope. |
| 3 | `executor::tests::pop_mutation` | PriorityQueue generic/reference mismatch and bytecode writeback assertions. | Mutable receiver writeback + generic container normalization. |
| 2 | `executor::v2_stack_tests` | Hand-built tests push arg count as `Constant::Number(0.0)`; executor now expects integer arg count. | Likely isolated test-harness update to `Constant::Int(0)`, but tests are outside this cluster's write scope. |
| 2 | `executor::tests::try_operator` | Invalid infallible Option cast and `Some/None as int` symmetry assertions fail. | Result/Option cast/assert pipeline; overlaps runtime inference and error-handling correctness. |
| 2 | `executor::tests::trait_object_thunks` | `Self` method lookup fails (`Method 'name' not found on type 'Self'`). | Trait-object thunk/self-type method resolution. |
| 2 | compiler expression diagnostics | `type_info` gate reports undefined function; enum pattern error highlights wrong semantic; `!!` unwrap loses type into binop. | Comptime builtin diagnostics, pattern diagnostics, Result `!!` type propagation. |
| 1 each | loops, monomorphization, statements, module resolution, stdlib, extension integration, runtime error payload, IO export | See raw log lines in artifacts. | Narrow owners listed in backlog. |

## Documented-semantics contradictions

`shape-test --test book_policy` passes 3/3, so the book syntax/link policy gate
is green.

Failures that do contradict current documented or classified semantics:

1. Runtime inference's five failures contradict the strict-typing inference
   promise and are also base-stable. These are current correctness issues.
2. Result/Option `?` and `!!` failures in runtime/VM contradict the
   `error_handling` classification, which treats Result `?`/`!!` runtime
   behavior as FN-REG-CORRECTNESS.
3. Table/query failures (`filter`, `select`, `count`, `Table<Record>` field
   projection) contradict the `tables_queryable` and truth-set classifications
   for v0.3.3 correctness. The two Queryable parser failures are different:
   their fixtures use stale comma-style trait method syntax, so the tests
   contradict current syntax rather than proving implementation wrong.
4. Struct-pattern/object-destructuring failures contradict the truth-set
   "object destructuring must fully work" scope.
5. The VM unit tests named `test_window_row_number_builtin_executes` and
   `test_window_lag_lead_builtin_executes` contradict the current
   `window_functions` classification, which says those methods are
   legitimately absent and current behavior should be a method-missing
   diagnostic. Treat these two as stale/future TDD tests unless a newer
   product decision pulled window builtins into scope.

## Ranked correctness backlog

1. **Runtime inference core (5 runtime failures, plus VM cascades).**
   Owner: `shape-runtime::type_system::inference`. Fix fallible constructor
   unwrapping, numeric body constraints, and callsite-union conflict recording.
   This is the smallest high-leverage correctness cluster and is entirely real
   semantics.

2. **Result/Option error pipeline and typed propagation.**
   Owner: VM compiler/executor around `?`, `!!`, `result_option_carrier`, and
   runtime error payload normalization. Covers `ws3_f3_error_context...`,
   try-operator non-surface failures, `runtime_error_payload_tests`, and likely
   parts of the runtime inference bucket.

3. **Phase-2c host-tier kinded eval/marshal/constant API restoration.**
   Owner: VM host boundary/test utilities plus kinded constant-table support.
   This clears most of the 201 explicit surfaces: matrix, V2 opcodes,
   typed arrays, collections, channels, decimal, and table iteration. It is
   high count but should be handled as a shared harness/API rebuild, not as
   per-test behavior fixes.

4. **Collection/table method registry and mutable receiver writeback.**
   Owner: VM method registry/stdlib method metadata and collection handlers.
   Targets Set method lookup, Table `count`/`filter`/`select`, HashSet
   writeback, and PriorityQueue pop-mutation generic normalization.

5. **Array/typed-emission fallback policy.**
   Owner: `compiler::v2_typed_emission` and runtime type inference. Decide and
   implement the strict-flip policy for heterogeneous literals and legacy
   fallback tests. If heterogeneous literals are intentionally rejected, update
   the stale tests; otherwise restore a sound union/Any carrier.

6. **Pattern/object destructuring and diagnostics.**
   Owner: compiler pattern binding and diagnostics. Fix non-enum struct
   destructuring and preserve the enum-unknown diagnostic target.

7. **Narrow singletons.**
   Owners by file: `module_resolution.rs` forward type alias compatibility;
   `stdlib.rs` Snapshot enum schema in core bytecode;
   extension integration namespace const specialization; IO module export
   `join`; comptime `type_info` gate diagnostic; Type-C auto conversion helper
   registration.

## Close state

No broad behavior fixes landed in this cluster. Deliverables are this audit doc
and `scripts/compare_cargo_failures.sh`.
