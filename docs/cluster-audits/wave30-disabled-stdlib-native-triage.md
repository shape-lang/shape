# Wave 30C Disabled Stdlib/Native Triage

Date: 2026-07-09
Role: Wave-30C stdlib/native disabled-book triage worker

## Scope Honored

- Wrote only this report.
- Used the current sibling manifest:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Used sibling book pages and local source/test grep for static
  classification only.
- Did not run cargo, just, nextest, rustc, build, test, extractor, or
  book-truth gate commands.
- Did not edit production code, sibling book pages, or `AGENTS.md`.

## Current Manifest

| Metric | Count |
|---|---:|
| Generated | `2026-07-09T23:40:40.617Z` |
| Total snippets | 707 |
| Runnable snippets | 541 |
| Disabled snippets | 166 |
| Deferred snippets | 0 |
| Scoped disabled snippets | 30 |

Scope filter covered `stdlib/native/**`, selected `stdlib/core/*` math/testing
pages, `stdlib/math/**`, and `stdlib/domain/**`. No disabled rows remain under
`stdlib/native/**` in the current manifest: native pages are 110/110 runnable,
including `stdlib/native/archive.mdx` at 3/3 runnable.

## Bucket Counts

| Bucket | Count |
|---|---:|
| Active implementation gap | 27 |
| Preview/out-of-scope | 2 |
| Stale-green/count-reduction candidate | 1 |
| External/manual/fixture/server/env/permission dependent | 0 |
| Proof/design gap | 0 |
| Intentional diagnostic | 0 |
| Old syntax/book rewrite | 0 |
| Total | 30 |

Several active implementation rows also need small data fixtures before they can
become book-gate rows. The primary blocker is still implementation, so they are
not counted as fixture-dependent here.

## Disabled Rows

| Page:line | Snippet id | Bucket | Reason |
|---|---|---|---|
| `stdlib/core/distributions.mdx:49` | `B__stdlib__core__distributions__2__L49.shape` | Active implementation gap | `dist_sample_n` is still disabled for the named-distribution typed-array/kinded carrier path. |
| `stdlib/core/math.mdx:70` | `B__stdlib__core__math__5__L70.shape` | Active implementation gap | `correlation(Array<number>, Array<number>)` awaits the statistic intrinsic kinded-carrier migration. |
| `stdlib/core/math.mdx:86` | `B__stdlib__core__math__6__L86.shape` | Active implementation gap | `covariance` has the same statistic intrinsic carrier gap. |
| `stdlib/core/math.mdx:102` | `B__stdlib__core__math__7__L102.shape` | Active implementation gap | `percentile` still blocks the public statistic helper row. |
| `stdlib/core/monte_carlo.mdx:82` | `B__stdlib__core__monte_carlo__2__L82.shape` | Active implementation gap | `monte_carlo_stats` depends on the disabled `percentile` path and returns a stats object. |
| `stdlib/core/property_testing.mdx:19` | `B__stdlib__core__property_testing__0__L19.shape` | Active implementation gap | Importing the module still loads the generic property source with function-field schema blockers. |
| `stdlib/core/property_testing.mdx:32` | `B__stdlib__core__property_testing__1__L32.shape` | Active implementation gap | `property<T>` depends on function fields in `PropertyResult`/`PropertySpec` schema handling. |
| `stdlib/core/property_testing.mdx:49` | `B__stdlib__core__property_testing__2__L49.shape` | Active implementation gap | `run_properties<T>` still needs specialization plus typed empty-array/result carrier work. |
| `stdlib/core/property_testing.mdx:77` | `B__stdlib__core__property_testing__3__L77.shape` | Active implementation gap | Generator closure and `gen_array` path depends on the same property/function-field route. |
| `stdlib/core/stochastic.mdx:30` | `B__stdlib__core__stochastic__1__L30.shape` | Active implementation gap | `brownian_motion` row is still gated on stochastic intrinsic kinded-carrier migration. |
| `stdlib/core/stochastic.mdx:47` | `B__stdlib__core__stochastic__2__L47.shape` | Active implementation gap | `gbm` has the same stochastic process carrier gap. |
| `stdlib/core/stochastic.mdx:64` | `B__stdlib__core__stochastic__3__L64.shape` | Active implementation gap | `ou_process` has the same stochastic process carrier gap. |
| `stdlib/core/stochastic.mdx:80` | `B__stdlib__core__stochastic__4__L80.shape` | Active implementation gap | `random_walk` has the same stochastic process carrier gap. |
| `stdlib/core/testing.mdx:44` | `B__stdlib__core__testing__2__L44.shape` | Active implementation gap | Imported generic `assert_eq<T>` still fails book-smoke call-site type inference. |
| `stdlib/core/testing.mdx:59` | `B__stdlib__core__testing__3__L59.shape` | Active implementation gap | Imported generic `assert_ne<T>` has the same inference gap. |
| `stdlib/core/testing.mdx:88` | `B__stdlib__core__testing__5__L88.shape` | Active implementation gap | `assert_ok` calls `Result.isOk()`, which is unavailable on ordinary Result carriers in this path. |
| `stdlib/core/testing.mdx:103` | `B__stdlib__core__testing__6__L103.shape` | Active implementation gap | `assert_err` has the same Result method dispatch blocker. |
| `stdlib/domain/finance.mdx:16` | `B__stdlib__domain__finance__0__L16.shape` | Stale-green/count-reduction candidate | Import-only row; finance has narrower import/call proof tests, so recheck exact multi-imports or rewrite to a behavior assertion. |
| `stdlib/domain/iot.mdx:17` | `B__stdlib__domain__iot__0__L17.shape` | Preview/out-of-scope | Domain IoT package import surface is still a preview row without current gate proof. |
| `stdlib/domain/iot.mdx:126` | `B__stdlib__domain__iot__1__L126.shape` | Preview/out-of-scope | IoT monitoring example needs domain fixture data and the core simulation path before it is a current gate target. |
| `stdlib/domain/physics.mdx:20` | `B__stdlib__domain__physics__0__L20.shape` | Active implementation gap | Page says physics module import waits for strict-type acceptance in VM and JIT modes. |
| `stdlib/domain/physics.mdx:81` | `B__stdlib__domain__physics__1__L81.shape` | Active implementation gap | `simulate_projectile` depends on the same physics strict-type/module readiness. |
| `stdlib/domain/simulation.mdx:32` | `B__stdlib__domain__simulation__1__L32.shape` | Active implementation gap | `DataTable.simulate` is still a SURFACE runtime method; example also needs a `prices` fixture. |
| `stdlib/domain/simulation.mdx:82` | `B__stdlib__domain__simulation__2__L82.shape` | Active implementation gap | `simulate_correlated` needs correlated ctx TypedObject construction and fixture series. |
| `stdlib/domain/simulation.mdx:106` | `B__stdlib__domain__simulation__3__L106.shape` | Active implementation gap | `replay` wraps the same `table.simulate` path and currently has placeholder variables. |
| `stdlib/math/interpolation.mdx:51` | `B__stdlib__math__interpolation__1__L51.shape` | Active implementation gap | Public `Mat<number>` construction is not available enough for user snippets. |
| `stdlib/math/optimize.mdx:58` | `B__stdlib__math__optimize__3__L58.shape` | Active implementation gap | `optimize::minimize` waits on strict typed-array construction inside optimizer internals. |
| `stdlib/math/optimize.mdx:78` | `B__stdlib__math__optimize__4__L78.shape` | Active implementation gap | Bounded optimizer row has the same typed-array/options carrier blocker. |
| `stdlib/math/rotation.mdx:32` | `B__stdlib__math__rotation__1__L32.shape` | Active implementation gap | `euler_to_matrix` returns `Mat<number>` and depends on public Mat construction/carriers. |
| `stdlib/math/rotation.mdx:43` | `B__stdlib__math__rotation__2__L43.shape` | Active implementation gap | `matrix_to_euler` depends on the `Mat<number>` return/input path and needs a real assertion. |

## Priority Lanes

1. Testing and property testing strictness.
   Own `crates/shape-runtime/stdlib-src/core/utils/{testing,property_testing}.shape`,
   compiler generic-call/Result dispatch paths under `crates/shape-vm/src/compiler/**`,
   and existing coverage in `bin/shape-cli/tests/stdlib/stdlib_advanced.rs`.
   The current blockers are imported generic assertion inference, Result
   `isOk`/`isErr` dispatch, function-field schemas in `PropertySpec<T>`, and
   `run_properties<T>` specialization/empty-array inference.

2. Math, stochastic, distribution, and Monte Carlo carriers.
   Own `crates/shape-runtime/stdlib-src/core/{math,stochastic,distributions,monte_carlo}.shape`,
   `crates/shape-runtime/src/intrinsics/{statistical,distributions,stochastic}.rs`,
   VM intrinsic dispatch around `crates/shape-vm/src/executor/vm_impl/builtins.rs`,
   and focused rows in `tools/shape-test/tests/stdlib_math/**` plus
   `bin/shape-cli/tests/stdlib/simulation.rs`. This lane clears the requested
   stochastic/math carrier blockers.

3. Public Mat and optimizer carriers.
   Own `crates/shape-runtime/stdlib-src/math/{interpolation,optimize,rotation}.shape`,
   `crates/shape-vm/src/compiler/expressions/matrix_ops.rs`,
   `crates/shape-vm/src/executor/builtins/matrix_intrinsics.rs`, and
   `crates/shape-vm/src/executor/tests/{matrix_ops,operator_overload}.rs`.
   Start with a public `mat(rows, cols, flat_array)` path that the book gate can
   use, then prove optimizer typed-array/options construction.

4. Domain simulation and physics readiness.
   Own `crates/shape-vm/src/executor/objects/datatable_methods/simulation.rs`,
   `crates/shape-runtime/stdlib-src/core/simulation.shape`,
   `crates/shape-runtime/stdlib-src/{physics,iot}/**`, and
   `bin/shape-cli/tests/stdlib/simulation.rs`. The core blocker is the
   `datatable.simulate` SURFACE method: correlated ctx construction, handler
   return TypedObject extraction, event-log replay, and small deterministic
   table fixtures.

5. Book-only count reduction after implementation probes.
   Recheck or rewrite import-only rows at `stdlib/domain/finance.mdx:16`,
   `stdlib/core/property_testing.mdx:19`, `stdlib/domain/iot.mdx:17`, and
   `stdlib/domain/physics.mdx:20`. `stdlib/domain/simulation.mdx:106` is also
   prose-like until `replay` has a fixture. `stdlib/math/rotation.mdx:32` and
   `:43` should gain assertions before any future flip.

## Specific Callouts

- Native archive creation: not present in the current disabled scoped set.
  `stdlib/native/archive.mdx` has 3 snippets and all are runnable in the
  Wave-29 manifest, so no native archive lane should be dispatched from this
  report.
- Property testing/testing: still a real active gap. The local
  `stdlib_advanced.rs` assertions explicitly expect function-field schema and
  `run_properties` specialization errors.
- Stochastic/math carriers: still present exactly on `distributions`,
  `math.correlation`, `math.covariance`, `math.percentile`, all four
  `stochastic` rows, and `monte_carlo_stats`.

## Files Changed

- `docs/cluster-audits/wave30-disabled-stdlib-native-triage.md`
