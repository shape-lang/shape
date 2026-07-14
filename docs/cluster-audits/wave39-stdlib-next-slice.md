# Wave 39C: Stdlib Completeness Next Slice

Date: 2026-07-10  
Role: lightweight stdlib completeness scout  
Baseline: `docs/cluster-audits/wave38-disabled-current-triage.md`

## Scope and Method

The Wave-38A inventory has 28 active disabled rows in `stdlib/core`,
`stdlib/math`, and `stdlib/domain`. This audit compared the current Shape
stdlib source, runtime intrinsic factories, VM dispatch, JIT registration, VM
and JIT test surfaces, and the current sibling MDX. Static inspection only was
used. No cargo, just, nextest, rustc, build, test, extraction, or book-truth
command was run. Only this report is changed.

## Recommendation

Choose the **typed statistical intrinsic dispatch slice**:

| Book row | Public API | Current blocker | Expected disposition |
|---|---|---|---|
| `stdlib/core/math.mdx:70` | `correlation(Array<number>, Array<number>) -> number` | VM dispatch still enters the Wave-5d surface arm | Flip after VM/JIT parity proof |
| `stdlib/core/math.mdx:86` | `covariance(Array<number>, Array<number>) -> number` | Same shared dispatch arm | Flip after VM/JIT parity proof |
| `stdlib/core/math.mdx:102` | `percentile(Array<number>, number) -> number` | Same shared dispatch arm | Flip after VM/JIT parity proof |

This is the smallest closed carrier family with three rows, existing typed
array collection code, an existing runtime implementation, and existing JIT
symbol registration. It has a bounded implementation surface and does not
require a new value or schema architecture. The expected reduction is **3
rows**, from 28 active stdlib/math/domain rows to **25**.

## Current Evidence

### Runtime and stdlib

- `crates/shape-runtime/src/intrinsics/statistical.rs:25-121` already exposes
  typed-marshal entries for all four statistical intrinsics. The three target
  entries accept `Array<number>` carriers and return `ConcreteType::Number`.
- The same file defines the current semantics: correlation and covariance
  reject unequal lengths; empty input returns `NaN`; percentile accepts
  `0..=100`, copies the input, and selects the rounded
  `p / 100 * (n - 1)` index.
- `crates/shape-runtime/src/stdlib/mod.rs:56-65` registers the statistical
  intrinsic module. `crates/shape-runtime/src/type_system/environment/mod.rs:1341-1363`
  also declares the matching concrete builtin signatures.
- `crates/shape-runtime/stdlib-src/core/math.shape:37-50` already gives the
  three public wrappers exact `Array<number>` and `number` annotations. No
  public API redesign is needed.

### VM

- `crates/shape-vm/src/executor/builtins/math.rs:404-435` already provides the
  v2 typed-array reader `collect_number_series`, including numeric element
  coercion and `Ptr(HeapKind::TypedArray)` validation.
- `crates/shape-vm/src/executor/builtins/intrinsics/statistical.rs:1-10` is
  intentionally empty and documents the missing `KindedSlot` migration.
- `crates/shape-vm/src/executor/vm_impl/builtins.rs:738-784` still groups
  `IntrinsicCorrelation`, `IntrinsicCovariance`, and `IntrinsicPercentile`
  under the `phase-1b-vm-wave-5d-intrinsic` surface error. This is the actual
  blocker behind the current MDX text, not the already-complete runtime
  factory.

### JIT

- `crates/shape-jit/src/ffi_symbols/intrinsics/mod.rs:106-157` and
  `crates/shape-jit/src/ffi_symbols/math_symbols.rs:120-131` already register
  JIT symbols for percentile, correlation, and covariance.
- Those symbols currently call `extract_column` in
  `crates/shape-jit/src/ffi/value_ffi.rs:481-500`, which expects the older
  column-reference carrier rather than the v2 raw `TypedArray<T>` carrier.
  `crates/shape-jit/src/compiler/accessors.rs:471-475` consequently needs a
  parity decision in this slice: either update the symbols to read the v2
  `Array<number>` carrier, or remove these three direct JIT entries and let
  JIT use its existing bytecode fallback while the VM owns the canonical
  typed implementation. The fallback is the recommended minimum because it
  preserves semantics without introducing a new FFI ABI.

## Exact Implementation Contract

The worker should implement the following and nothing broader:

1. Replace the three statistical cases in the VM surface arm with
   `KindedSlot` handlers. Each handler must validate arity, read numeric
   `Array<number>`/`Array<int>` values through the v2 typed-array view, and
   return `KindedSlot::from_number(...)`.
2. Preserve runtime semantics exactly:
   - correlation: Pearson coefficient; unequal lengths are a runtime error;
     empty or zero-variance input produces `NaN`.
   - covariance: sample covariance with the existing `n - 1` denominator;
     unequal lengths are a runtime error; empty or insufficient input follows
     the existing intrinsic `NaN` behavior.
   - percentile: reject values outside `[0, 100]`; empty input produces
     `NaN`; use the existing rounded order-statistic rule.
3. Do not route through `ValueWord`, NaN-box tag inspection, or the old
   `extract_column` carrier. Reuse `collect_number_series` or extract its
   small shared reader only if the existing helper cannot be called from the
   focused module.
4. Keep the existing JIT FFI symbols only if they can read the current v2
   carrier safely. Otherwise remove their direct-accessor eligibility and
   exercise the VM fallback. A later optimization lane can add v2-native JIT
   symbols without changing this public contract.

## Dependency Edges

```text
stdlib/core/math wrappers
        |
        v
compiler BuiltinFunction mapping
        |
        +--> VM KindedSlot statistical handlers --> simd_statistics
        |
        +--> JIT direct symbol OR existing bytecode fallback
```

`monte_carlo_stats` is a downstream consumer of the percentile edge, but it
is not part of the guaranteed row count for this slice. Its local
`percentile` helper at `crates/shape-runtime/stdlib-src/core/monte_carlo.shape:8-10`
is unannotated, and its result record has nullable fields in the empty-result
branch. It should be promoted only after the three direct statistical rows
prove the carrier path; adding a named `MonteCarloStats` nullable typed-object
carrier is a separate, bounded follow-up rather than a reason to widen this
slice now.

## Owned Files and Focused Proofs

### Production ownership

- `crates/shape-vm/src/executor/builtins/intrinsics/statistical.rs`: add the
  three typed handlers and focused unit helpers.
- `crates/shape-vm/src/executor/vm_impl/builtins.rs`: replace only the three
  statistical surface cases with handler calls.
- `crates/shape-jit/src/compiler/accessors.rs`: only if needed to select the
  existing bytecode fallback for v2 arrays.
- `crates/shape-jit/src/ffi_symbols/intrinsics/mod.rs` and
  `crates/shape-jit/src/ffi_symbols/math_symbols.rs`: only if the worker proves
  the direct JIT path can be migrated to the current typed-array carrier in
  this same slice. Do not add a parallel legacy carrier.

### VM and JIT tests

Use a small dedicated ShapeTest integration target, for example
`tools/shape-test/tests/stdlib_statistics/main.rs`, so the existing
`ShapeTest::with_jit()` path exercises both interpreters without expanding the
already broad legacy math file. The cases should cover:

- correlation of `[10.0, 20.0, 30.0]` and `[2.0, 4.0, 6.0]` yielding `1.0`;
- covariance of `[1.0, 2.0, 3.0]` and `[2.0, 4.0, 6.0]` yielding `2.0`;
- percentile 50 and 95 on `[10.0, 20.0, 30.0, 40.0, 50.0]`;
- VM/JIT equality for the three normal cases and at least one empty-input or
  invalid-percentile error case;
- an integer-element input where the declared `Array<number>` carrier accepts
  the existing lossless numeric element path, plus a mismatched-length error.

The JIT assertions must compare the complete result and error behavior with
the VM. They must not assert that a native JIT symbol was used; correct
fallback is sufficient for this row-flipping slice.

## Comparison With Other Candidate Groups

### Testing and property helpers

`stdlib/core/testing.mdx:44,59,88,103` remains a compiler/type-system lane.
`crates/shape-runtime/stdlib-src/core/utils/testing.shape:34-57` uses generic
`assert_eq`/`assert_ne`, while `assert_ok`/`assert_err` at `:86-105` depend on
Result method dispatch. `property_testing` is broader still:
`PropertySpec<T>` contains function fields and `gen_array` relies on generic
empty-array element inference (`property_testing.shape:16-21,63-83,125-140`).
These rows do not share the numeric intrinsic dispatch boundary and should be
deferred.

### Stochastic and distributions

The runtime factories are typed (`intrinsics/stochastic.rs:203-367` and
`intrinsics/distributions.rs:502-668`), but the VM still surfaces all four
stochastic intrinsics and `dist_sample_n` in the same broad Wave-5d arm at
`vm_impl/builtins.rs:738-746`. Their JIT side has no equivalent v2-native
symbol family. They are a coherent **next** intrinsic cluster, but require
more handler code and seeded array-return proofs than the statistics slice.
`stdlib/core/distributions.mdx:49` and all four stochastic rows should remain
disabled for now; their migration notes are not merely stale.

### Domain carriers

- Finance (`stdlib/domain/finance.mdx:16`) is an import-only row. It is a
  stale/count-reduction candidate or needs a behavior assertion, not a numeric
  intrinsic proof. The current finance source also contains broadly untyped
  row helpers such as `types/ohlcv.shape:8-61`.
- Physics (`stdlib/domain/physics.mdx:20,81`) uses typed object state and
  arithmetic bodies, but requires a strict-module proof independent of the
  statistical carrier.
- Simulation (`stdlib/domain/simulation.mdx:32,82,106`) depends on
  `DataTable.simulate`, correlated table fixtures, and replay inputs. These are
  runtime/fixture edges, not reusable `Array<number>` intrinsic rows.
- Interpolation and rotation (`stdlib/math/interpolation.mdx:51` and
  `rotation.mdx:32,43`) depend on public `Mat<number>` construction and matrix
  carriers. Optimize (`optimize.mdx:58,78`) adds callback, options, and typed
  simplex-array concerns. Defer all five until the matrix/optimizer carrier
  lane is ready.

## Deferred Row Accounting

| Group | Active rows | Disposition after this slice |
|---|---:|---|
| Chosen math statistics | 3 | Flip after focused VM/JIT proof |
| Monte Carlo stats | 1 | Downstream follow-up after percentile proof and named nullable result carrier |
| Distributions/stochastic | 5 | Next VM intrinsic cluster; not stale |
| Testing/property testing | 8 | Generic-call, Result dispatch, function-field, and empty-array inference lane |
| Finance/physics/simulation | 6 | Domain readiness, stale import candidate, or fixture/data carriers |
| Interpolation/optimize/rotation | 5 | Public matrix/typed-array carrier lane |
| **Total** | **28** | **25 remain after the chosen three-row slice** |

## Changed File

`docs/cluster-audits/wave39-stdlib-next-slice.md`
