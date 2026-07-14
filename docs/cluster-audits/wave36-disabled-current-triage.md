# Wave 36 Disabled Book Inventory Refresh

Date: 2026-07-10
Role: Wave-36A current disabled-book inventory refresh worker

## Scope

- Manifest source:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Sibling book source:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs`.
- Baseline comparison: `docs/cluster-audits/wave34-disabled-current-triage.md`.
- Static inspection only: manifest JSON, sibling MDX, prior audit notes, and
  AGENTS evidence supplied by the supervisor.
- No cargo, just, nextest, rustc, build, tests, extractor, or book-truth gate
  commands were run.
- Wrote only this report.

## Manifest

Generated: `2026-07-10T02:32:58.980Z`.

| Metric | Count |
|---|---:|
| Total Shape snippets | 707 |
| Runnable snippets | 557 |
| Disabled snippets | 150 |
| Deferred snippets | 0 |
| Expected-output snippets | 6 |
| Expected-fail snippets | 6 |
| Fixture snippets | 6 |

Supervisor verification: full release-binary book gate passed `557/557` in
`run-p1378033-i32031890.service`, report
`/tmp/shape-wave35-book-truth-report.json`.

The disabled count dropped from Wave-34's 157 to 150. The seven retired disabled
rows are the six stable diagnostic rows converted to `expected-fail` in
Wave-35A and the bounded `state.capture_all()` metadata row made runnable in
Wave-35B. The remaining disabled set is still mostly real implementation work
or examples that need a deterministic fixture.

## Category Totals

| Category | Count |
|---|---:|
| Active missing feature | 68 |
| External/manual/fixture-only | 41 |
| Preview/out-of-scope | 22 |
| Old syntax/book rewrite | 8 |
| Proof/design gap | 5 |
| Intentional diagnostic still not expected-fail | 4 |
| Stale/flip candidate | 2 |
| **Total** | **150** |

## Area Breakdown

| Area | Total | Active | External | Stale | Old | Diagnostic | Preview | Proof/design |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Language surface | 51 | 22 | 8 | 0 | 5 | 3 | 13 | 0 |
| State/snapshot/distributed/proof | 40 | 12 | 11 | 2 | 2 | 1 | 7 | 5 |
| Comptime/extensions/tooling | 29 | 6 | 22 | 0 | 1 | 0 | 0 | 0 |
| Stdlib/math/domain | 30 | 28 | 0 | 0 | 0 | 0 | 2 | 0 |
| **Total** | **150** | **68** | **41** | **2** | **8** | **4** | **22** | **5** |

There are still zero disabled rows under `stdlib/native/**`. The native-C rows
below are `advanced/native-c-interop.mdx` examples that need DuckDB, Arrow, or
native-library fixtures, so they are counted with comptime/extensions/tooling.

## Row Inventory

### Language Surface

Active missing features, 22: `advanced/ownership-deep-dive.mdx:81`;
`examples/comptime-codegen.mdx:22`; `fundamentals/datetime.mdx:364`, `:404`;
`fundamentals/error-handling.mdx:186`, `:207`, `:224`, `:275`, `:287`;
`fundamentals/objects-arrays.mdx:366`; `fundamentals/operators.mdx:436`, `:503`;
`fundamentals/references-borrowing.mdx:73`; `fundamentals/strings.mdx:277`,
`:302`; `fundamentals/tables.mdx:109`; `fundamentals/traits.mdx:71`, `:172`,
`:249`, `:265`, `:387`; `fundamentals/variables.mdx:82`.

External/manual/fixture-only, 8: `examples/web-request.mdx:22`;
`fundamentals/datetime.mdx:19`; `fundamentals/functions.mdx:413`;
`fundamentals/modules.mdx:80`, `:191`; `fundamentals/resource-management.mdx:365`,
`:387`; `fundamentals/variables.mdx:168`.

Preview/out-of-scope, 13: `advanced/ownership-deep-dive.mdx:45`, `:54`, `:459`,
`:470`, `:483`; `fundamentals/async.mdx:123`; `fundamentals/content.mdx:51`,
`:61`, `:107`, `:453`; `fundamentals/tables.mdx:56`, `:76`;
`fundamentals/traits.mdx:330`.

Old syntax/book rewrite, 5: `advanced/ownership-deep-dive.mdx:259`;
`fundamentals/error-handling.mdx:90`; `fundamentals/references-borrowing.mdx:269`;
`fundamentals/strings.mdx:397`; `fundamentals/tables.mdx:125`.

Intentional diagnostics still not expected-fail, 3:
`advanced/ownership-deep-dive.mdx:399`, `:425`;
`fundamentals/references-borrowing.mdx:253`. These are still mixed or unstable
borrow/concurrency examples rather than the stable one-error rows Wave-35A
flipped.

### State, Snapshot, Distributed, Proof

Active missing features, 12: `advanced/content-addressed-bytecode.mdx:154`,
`:168`, `:226`, `:264`, `:541`; `stdlib/core/state.mdx:225`, `:241`, `:334`,
`:401`, `:484`, `:514`, `:541`.

External/manual/fixture-only, 11: `advanced/content-addressed-bytecode.mdx:515`;
`advanced/module-distribution.mdx:563`; `advanced/polyglot-distributed.mdx:149`;
`advanced/security-permissions.mdx:441`; `advanced/wire-protocol.mdx:90`;
`stdlib/core/remote.mdx:42`, `:77`, `:107`, `:220`;
`stdlib/core/transport.mdx:61`, `:95`.

Stale/flip candidates, 2: `advanced/resumability.mdx:20`, `:100`. Snapshot and
resume are real through the CLI path, but these examples need a deterministic
two-process snapshot/resume fixture against one snapshot store before they can
become truth-gated book rows.

Preview/out-of-scope, 7: `advanced/content-addressed-bytecode.mdx:321`;
`advanced/security-permissions.mdx:329`, `:346`, `:360`, `:383`, `:466`, `:498`.

Proof/design gaps, 5: `advanced/developer-tools.mdx:86`, `:137`, `:238`, `:320`,
`:462`. These describe planned `std::debug` hot reload, time travel, prefetch,
and proof APIs rather than shipped Shape stdlib surfaces.

Old syntax/book rewrite, 2: `advanced/content-addressed-bytecode.mdx:282`,
`:396`. Both still rely on retired `__original__(args)` forwarding in rewritten
annotation bodies.

Intentional diagnostic still not expected-fail, 1:
`advanced/security-permissions.mdx:414`. It describes permission-configured
compile-time rejection and still needs a stable permission fixture/error shape.

### Comptime, Extensions, Tooling

Active missing features, 6: `advanced/annotations.mdx:73`, `:89`;
`advanced/comptime-annotations-cookbook.mdx:31`;
`advanced/comptime-llm-patterns.mdx:170`; `advanced/comptime.mdx:266`;
`tooling/polyglot.mdx:96`.

External/manual/fixture-only, 22: `advanced/annotations.mdx:480`, `:508`;
`advanced/comptime-annotations-cookbook.mdx:183`, `:329`;
`advanced/native-c-interop.mdx:139`, `:155`, `:286`;
`tooling/extensions.mdx:120`; `tooling/polyglot.mdx:14`, `:126`, `:186`;
`tooling/python-extension.mdx:68`, `:117`, `:142`, `:163`, `:184`, `:197`;
`tooling/typescript-extension.mdx:74`, `:134`, `:163`, `:180`, `:238`.

Old syntax/book rewrite, 1: `advanced/comptime-annotations-cookbook.mdx:308`.

### Stdlib, Math, Domain

Active missing features, 28: `stdlib/core/distributions.mdx:49`;
`stdlib/core/math.mdx:70`, `:86`, `:102`; `stdlib/core/monte_carlo.mdx:82`;
`stdlib/core/property_testing.mdx:19`, `:32`, `:49`, `:77`;
`stdlib/core/stochastic.mdx:30`, `:47`, `:64`, `:80`;
`stdlib/core/testing.mdx:44`, `:59`, `:88`, `:103`;
`stdlib/domain/finance.mdx:16`; `stdlib/domain/physics.mdx:20`, `:81`;
`stdlib/domain/simulation.mdx:32`, `:82`, `:106`;
`stdlib/math/interpolation.mdx:51`; `stdlib/math/optimize.mdx:58`, `:78`;
`stdlib/math/rotation.mdx:32`, `:43`.

Preview/out-of-scope, 2: `stdlib/domain/iot.mdx:17`, `:126`.

## Wave-35 Stale Notes

No clearly new stale/flip row remains after Wave-35. The two stale candidates
are the already-known `advanced/resumability.mdx` rows, now stronger fixture
candidates because the book gate already knows how to run selected two-phase
distributed/serve fixtures elsewhere.

Rows that might look stale but should stay disabled:

- `stdlib/core/state.mdx:225`, `:241`: `state.capture_all()` is now runnable,
  but its `VmState` is metadata-only, and `FrameState` still lacks executable
  arg/local/upvalue slots and a validated resume IP.
- `advanced/ownership-deep-dive.mdx:399`, `:425`,
  `fundamentals/references-borrowing.mdx:253`, and
  `advanced/security-permissions.mdx:414`: expected-fail support exists, but
  these are not the stable single-diagnostic rows Wave-35A flipped.
- `advanced/polyglot-distributed.mdx:149`: the sibling distributed
  snapshot/resume fixture covers the extern-C combined row, not dynamic Python
  extension loading plus selected snapshot-store resume through a foreign frame.

## Recommended Next Lanes

1. External fixture expansion. Add deterministic fixture classes for extension
   runtimes, Python/TypeScript scalar/error rows, selected snapshot stores,
   permission-configured execution contexts, native DuckDB/Arrow rows, multi-file
   module/package rows, live transport peers, and controlled HTTP/network rows.
   This is the largest count reducer at 41 rows.

2. State/resume/content-addressed completeness. Finish public `state.resume`,
   resumable `FrameState`, executable `VmState` carriers, arbitrary
   serialize/deserialize, object/path/module deltas, dispatch/cache helpers, and
   a two-process snapshot/resume book fixture. This owns 12 active rows plus the
   2 stale resumability rows.

3. Stdlib math/statistics/stochastic/domain carriers. Finish correlation,
   covariance, percentile, named distribution sampling, stochastic process
   carriers, Monte Carlo stats, property/testing helpers, public `Mat<number>`
   construction, optimizer options, finance/domain module readiness, physics,
   and simulation table carriers. This owns 28 active rows.

4. Trait, conversion, error-context, and collection ergonomics. Cover
   `From`/`TryFrom`/`TryInto`, `as Type?`, `!!`, named/generic trait dispatch,
   associated types, `HashMap.keys/values/entries`, imported generic
   `assert_eq`/`assert_ne`, and Result method dispatch. This covers the most
   central language-surface active gaps.

5. Comptime typed-generation and annotation target semantics. Finish expression
   and await annotation targets, connector-generated typed returns,
   TypeRef/typed fragment generation beyond source-string `extend`, and
   `Vec<struct>` foreign-marshalling returns. This is smaller by count but
   removes several high-leverage examples across annotations, comptime, and
   polyglot docs.

## Uncertainty

This was a static classification against the manifest and MDX source. Counts are
exact against `runnable == false`, but runtime status of disabled rows was not
reprobed. DateTime fixed-time rows remain active gaps because the last focused
probe reported VM success but default-JIT failure. Several external rows may
become easy fixture flips now that `fixture=serve` and expected-fail support
exist, but each still needs a row-specific deterministic harness before it is a
truth-gated book example.
