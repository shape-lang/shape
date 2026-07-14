# Wave 8 Disabled Remaining Book Triage

Generated from `/tmp/shape-wave8-local-snippets/manifest.json`
(`2026-07-09T10:54:21.027Z`) and supervisor-updated after the Wave-8
resource/annotation/module/log book flips passed the page and full book gates.

This is a source/manifest triage only. I did not run cargo, rustc, just,
nextest, shape-test, book-truth, the Shape binary, or any build/test command.

## Scope

Current verified manifest totals after Wave-8 book flips:

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 492 |
| Disabled snippets | 264 |

The Wave-6 triage reports were based on 756 total snippets, 462 runnable, and
294 disabled. The Wave-7 supervisor record later reported 466 runnable and 290
disabled. Wave-8 book flips raised the current verified gate to 492 runnable
and 264 disabled.

This report covers 47 currently disabled snippets on pages not covered, or only
weakly covered, by the three Wave-6 disabled-snippet triage reports. The other
217 current disabled snippets remain under those Wave-6 scopes.

Scoped pages:

- `advanced/module-distribution.mdx`, `advanced/resumability.mdx`,
  `advanced/wire-protocol.mdx`
- `examples/web-request.mdx`
- `fundamentals/async.mdx`, `fundamentals/objects-arrays.mdx`,
  `fundamentals/operators.mdx`, `fundamentals/variables.mdx`
- `getting-started/basic-concepts.mdx`
- `stdlib/core/distributions.mdx`, `stdlib/core/monte_carlo.mdx`,
  `stdlib/core/ode.mdx`,
  `stdlib/core/transport.mdx`
- `stdlib/domain/finance.mdx`, `stdlib/domain/iot.mdx`,
  `stdlib/domain/physics.mdx`, `stdlib/domain/simulation.mdx`
- `stdlib/math/interpolation.mdx`, `stdlib/math/optimize.mdx`,
  `stdlib/math/rotation.mdx`
- residual native/tooling pages: `stdlib/native/{csv,json,math,time}.mdx`,
  `tooling/{docstrings,execution-server,extensions,frontmatter}.mdx`

## Totals

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 15 |
| `active_feature_gap` | 12 |
| `old_syntax_or_policy_rewrite` | 2 |
| `external_environment_or_permission` | 9 |
| `design_or_proof_gap` | 5 |
| `preview_or_out_of_scope` | 3 |
| `intentional_negative_or_diagnostic` | 1 |
| `unknown_needs_execution` | 0 |
| **Total scoped** | **47** |

## Page Triage

### `advanced/module-distribution.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L563
  `E__advanced__module-distribution__0__L563.shape` imports from a packaged
  `stdlib-0.3.1.shapec` dependency and uses undefined `a`/`b`. Keep disabled
  unless book-truth gains a packaged-module fixture.

### `advanced/resumability.mdx`

Counts: `design_or_proof_gap` 2.

- `design_or_proof_gap`: L21 `E__advanced__resumability__0__L21.shape`
  covers the real `snapshot()` first-pass shape, but the hash is per-run and
  the illustrated resume flow is explicitly v0.4-bound.
- `design_or_proof_gap`: L105 `E__advanced__resumability__1__L105.shape`
  describes function-level checkpoint/resume. The page says this is planned
  for v0.4 and not available in v0.3.3.

### `advanced/wire-protocol.mdx`

Counts: `design_or_proof_gap` 1.

- `design_or_proof_gap`: L90 `E__advanced__wire-protocol__0__L90.shape`
  combines `state::serialize`/`deserialize`, transport compression, and a live
  `10.0.0.5:9000` peer. This belongs with the state/transport proof lane, not
  a standalone book flip.

### `examples/web-request.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L22
  `C__examples__web-request__0__L22.shape` depends on external HTTP endpoints,
  network permission, and schema-aware JSON loading. Keep disabled or rewrite
  around deterministic local data.

### `fundamentals/async.mdx`

Counts: `active_feature_gap` 1.

- `active_feature_gap`: L123 `A__fundamentals__async__7__L123.shape` uses
  named `join all` branches and property access on the joined result. The page
  marks this as v0.4; current v0.3.3 returns a `TaskGroup` summary instead.

### `fundamentals/objects-arrays.mdx`

Counts: `active_feature_gap` 1, `intentional_negative_or_diagnostic` 1.

- `intentional_negative_or_diagnostic`: L37
  `A__fundamentals__objects-arrays__1__L37.shape` intentionally indexes past
  the end of an array to demonstrate a runtime error.
- `active_feature_gap`: L366
  `A__fundamentals__objects-arrays__20__L366.shape` uses `HashMap.keys()`,
  `values()`, and `entries()`, which the page says still hit the V3-S5
  ckpt-5 consumer-cascade result-carrier gap.

### `fundamentals/operators.mdx`

Counts: `stale_disabled_candidate` 1, `active_feature_gap` 1.

- `active_feature_gap`: L436 `A__fundamentals__operators__23__L436.shape`
  uses `Result<T, E> !! string`; the page says inference still rejects this
  binary operator form at HEAD.
- `stale_disabled_candidate`: L503 `A__fundamentals__operators__28__L503.shape`
  is a fragment for `as Percent { decimals: 4 }`. Parser and type-alias
  coverage exist; next worker should add a local `Percent`/`value` scaffold and
  smoke the runtime path before flipping.

### `fundamentals/variables.mdx`

Counts: `active_feature_gap` 1, `old_syntax_or_policy_rewrite` 1.

- `active_feature_gap`: L82 `A__fundamentals__variables__4__L82.shape`
  documents `var` alias plus `.push` copy-on-write. The snippet comment names
  the current VM/JIT failure class, so keep it disabled.
- `old_syntax_or_policy_rewrite`: L168 `A__fundamentals__variables__8__L168.shape`
  uses old `fs.read(path)` plus a `Result<string>` shorthand and also depends
  on `config.txt`. Rewrite to current `std::core::file::read_text` or keep as
  external filesystem prose.

### `getting-started/basic-concepts.mdx`

Counts: `preview_or_out_of_scope` 1.

- `preview_or_out_of_scope`: L149
  `C__getting-started__basic-concepts__9__L149.shape` is a type declaration
  plus comments pointing users to loaders, not a runnable behavioral example.

### `stdlib/core/distributions.mdx`

Counts: `stale_disabled_candidate` 1.

- `stale_disabled_candidate`: L50
  `B__stdlib__core__distributions__2__L50.shape` calls `dist_sample_n`.
  Runtime source has the intrinsic path; rewrite with import plus length/range
  predicates rather than raw random output, then verify under the serialized
  lane before flipping.

### `stdlib/core/monte_carlo.mdx`

Counts: `stale_disabled_candidate` 2.

- `stale_disabled_candidate`: L41
  `B__stdlib__core__monte_carlo__1__L41.shape` matches current module shape
  and acceptance-program usage. Add deterministic seeding or predicate output.
- `stale_disabled_candidate`: L84
  `B__stdlib__core__monte_carlo__2__L84.shape` should import both
  `monte_carlo` and random support, then print stable summary predicates.

### `stdlib/core/ode.mdx`

Counts: `stale_disabled_candidate` 3.

- `stale_disabled_candidate`: L34 `B__stdlib__core__ode__1__L34.shape`
  has current source/test evidence for scalar Euler; flip with a bounded
  numeric predicate instead of approximate-comment output.
- `stale_disabled_candidate`: L58 `B__stdlib__core__ode__2__L58.shape`
  has current source/test evidence for `rk4_system`; add a stable length or
  endpoint check.
- `stale_disabled_candidate`: L80 `B__stdlib__core__ode__3__L80.shape`
  has current source/test evidence for `rk45`; add import and deterministic
  endpoint/length checks.

### `stdlib/core/transport.mdx`

Counts: `design_or_proof_gap` 2.

- `design_or_proof_gap`: L56 `B__stdlib__core__transport__3__L56.shape`
  requires a live peer and request bytes. `transport::tcp()` is proven, but
  live send behavior belongs in a loopback transport proof lane.
- `design_or_proof_gap`: L88 `B__stdlib__core__transport__4__L88.shape`
  uses persistent connections to `10.0.0.5:9527` plus undefined `payload`;
  keep disabled until connection send/recv/close has a deterministic fixture.

### `stdlib/domain/finance.mdx`

Counts: `active_feature_gap` 1.

- `active_feature_gap`: L16 `B__stdlib__domain__finance__0__L16.shape`
  imports umbrella `std::finance::types`, `std::finance::risk`, and indicators.
  Newer tests prove selected finance paths, but this exact risk/umbrella
  import surface is not yet an honest book flip candidate.

### `stdlib/domain/iot.mdx`

Counts: `active_feature_gap` 2.

- `active_feature_gap`: L17 `B__stdlib__domain__iot__0__L17.shape` imports
  IoT modules whose source still includes old type syntax and current-time
  helper calls (`now()`) noted in previous domain audits.
- `active_feature_gap`: L126 `B__stdlib__domain__iot__1__L126.shape` depends
  on `temperature_readings`, table simulation, threshold helpers, and the same
  IoT stdlib gaps.

### `stdlib/domain/physics.mdx`

Counts: `stale_disabled_candidate` 2.

- `stale_disabled_candidate`: L17 `B__stdlib__domain__physics__0__L17.shape`
  now has current source and unignored test evidence for projectile/collision
  paths. Next worker should smoke the exact import set before flipping.
- `stale_disabled_candidate`: L75 `B__stdlib__domain__physics__1__L75.shape`
  aligns with current `simulate_projectile` test coverage. Rewrite output to a
  small deterministic predicate instead of printing the whole trajectory.

### `stdlib/domain/simulation.mdx`

Counts: `active_feature_gap` 3.

- `active_feature_gap`: L32 `B__stdlib__domain__simulation__1__L32.shape`
  uses `prices.simulate(...)`, generic row/state access, and object spread over
  inferred state. Previous audits identify this as a table/simulation language
  gap.
- `active_feature_gap`: L82 `B__stdlib__domain__simulation__2__L82.shape`
  uses `simulate_correlated`, external data series, and generic state handling.
- `active_feature_gap`: L106 `B__stdlib__domain__simulation__3__L106.shape`
  is a fragment over `prices`, `my_handler`, and `my_config`; `replay` also
  depends on table simulation support.

### `stdlib/math/interpolation.mdx`

Counts: `stale_disabled_candidate` 1.

- `stale_disabled_candidate`: L48
  `B__stdlib__math__interpolation__1__L48.shape` now uses the current
  five-argument `bilinear` signature. Add a print/assert for `vals[0]` before
  flipping.

### `stdlib/math/optimize.mdx`

Counts: `stale_disabled_candidate` 3.

- `stale_disabled_candidate`: L19 `B__stdlib__math__optimize__0__L19.shape`
  is an import-only snippet; older arity drift was corrected in the page.
- `stale_disabled_candidate`: L59 `B__stdlib__math__optimize__3__L59.shape`
  uses the current `OptimizeOptions` shape. Add bounded deterministic result
  checks.
- `stale_disabled_candidate`: L76 `B__stdlib__math__optimize__4__L76.shape`
  same; verify bounded optimization and print stable predicates.

### `stdlib/math/rotation.mdx`

Counts: `stale_disabled_candidate` 1, `active_feature_gap` 2.

- `active_feature_gap`: L32 `B__stdlib__math__rotation__1__L32.shape` calls
  `euler_to_matrix`, which constructs a `Mat<number>` through an array-backed
  path previously tied to V3-S5 typed-array/Mat construction surfaces.
- `active_feature_gap`: L43 `B__stdlib__math__rotation__2__L43.shape`
  depends on `euler_to_matrix` plus `matrix_to_euler`, so it shares the Mat
  construction gap.
- `stale_disabled_candidate`: L72 `B__stdlib__math__rotation__3__L72.shape`
  calls `normalize_euler` over a plain array. Add the missing import and stable
  output before flipping.

### `stdlib/native/csv.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L76 `B__stdlib__native__csv__5__L76.shape`
  explicitly depends on external `data.csv` and filesystem access.

### `stdlib/native/json.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L267 `B__stdlib__native__json__12__L267.shape`
  reads `quote.json` through `io` and demonstrates external file/schema
  extraction. Keep disabled unless converted to inline JSON text.

### `stdlib/native/math.mdx`

Counts: `stale_disabled_candidate` 1.

- `stale_disabled_candidate`: L65 `B__stdlib__native__math__2__L65.shape`
  just needs a defined `radius` and stable output; `PI()` is the current
  zero-argument constant function shape.

### `stdlib/native/time.mdx`

Counts: `external_environment_or_permission` 3.

- `external_environment_or_permission`: L84 `B__stdlib__native__time__4__L84.shape`
  is an async polling pattern over undefined `fetch(url)` and external network
  state.
- `external_environment_or_permission`: L121 `B__stdlib__native__time__6__L121.shape`
  is the same polling/backoff pattern plus retries and `error(...)`.
- `external_environment_or_permission`: L193 `B__stdlib__native__time__9__L193.shape`
  is a rate-limited paginated fetch pattern over an external API.

### `tooling/docstrings.mdx`

Counts: `preview_or_out_of_scope` 2.

- `preview_or_out_of_scope`: L118 `E__tooling__docstrings__5__L118.shape`
  is a bare `/// @see ...` doc-comment fragment with no declaration target.
- `preview_or_out_of_scope`: L124 `E__tooling__docstrings__6__L124.shape`
  is a bare `/// @link ...` doc-comment fragment with no declaration target.

### `tooling/execution-server.mdx`

Counts: `old_syntax_or_policy_rewrite` 1.

- `old_syntax_or_policy_rewrite`: L127
  `E__tooling__execution-server__0__L127.shape` teaches `@remote` as returning
  `Ok(...)`. Current remote docs should use bare raising semantics for
  `@remote` and `remote::call` for recoverable `Result`.

### `tooling/extensions.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L120 `D__tooling__extensions__0__L120.shape`
  depends on a DuckDB extension, a local `pricing_data.duckdb`, and queryable
  extension APIs. Keep disabled as an external integration example.

### `tooling/frontmatter.mdx`

Counts: `external_environment_or_permission` 1.

- `external_environment_or_permission`: L12 `E__tooling__frontmatter__0__L12.shape`
  is a file-mode script with frontmatter, local module paths, and an extension
  shared-library path. It is not a pure extracted snippet.

## Next Recommended Waves

1. **Core numeric stale flips.** Own only
   `stdlib/core/{distributions,monte_carlo,ode}.mdx`,
   `stdlib/math/{interpolation,optimize}.mdx`, and the safe `rotation`
   `normalize_euler` snippet. Rewrite to deterministic predicates and avoid
   raw random or full trajectory output.

2. **Small fundamentals cleanup.** Own only
   `fundamentals/{operators,variables,objects-arrays}.mdx` plus
   `getting-started/basic-concepts.mdx`. Split intentional diagnostics from
   positive examples, replace stale `fs.read`, and leave `var` CoW and
   HashMap key/value/entry methods disabled with explicit gap notes.

3. **Domain proof lane.** Smoke the exact `std::finance`, `std::iot`,
   `std::physics`, and `std::core::simulation` book surfaces. Physics appears
   closest to flip-ready; finance risk imports, IoT current-time/type syntax,
   and table simulation should remain gaps until proven or rewritten.

4. **Transport/resume proof lane.** Keep `advanced/resumability`,
   `advanced/wire-protocol`, and `stdlib/core/transport` disabled until a
   deterministic loopback transport fixture and snapshot/resume store story are
   available.

5. **External integration rewrite lane.** Keep web request, CSV/JSON file
   reads, DuckDB extensions, script frontmatter, and async fetch/time examples
   disabled unless each is rewritten with inline data or a controlled fixture.

6. **Remote tooling doc policy.** Rewrite `tooling/execution-server.mdx` to
   match current `@remote` raising semantics and direct users to
   `remote::call`/`remote::call_async` for recoverable result-returning calls.
