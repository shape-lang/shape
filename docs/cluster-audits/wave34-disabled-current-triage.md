# Wave 34 Disabled Book Inventory Refresh

Date: 2026-07-10
Role: Wave-34C current disabled-book inventory refresh scout

## Scope

- Manifest source:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Sibling book source:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs`.
- Static inspection only: manifest JSON, sibling MDX, and prior audit notes.
- No cargo, just, nextest, rustc, build, test, Miri, benchmark, extractor, or
  book-truth gate commands were run.
- Wrote only this report.

## Manifest

Generated: `2026-07-10T00:40:17.473Z`.

| Metric | Count |
|---|---:|
| Total Shape snippets | 707 |
| Runnable snippets | 550 |
| Plain runnable snippets | 544 |
| Fixture snippets | 6 |
| Expected-output snippets | 6 |
| Disabled snippets | 157 |
| Deferred snippets | 0 |

The drop from Wave-30's 166 disabled rows is consistent with the Wave-31 stale
probe cleanup plus Waves 32-33 flipping bounded `state.capture_call`, live
`fixture=serve` remote rows, execution-server `@remote`, and the distributed
extern-C snapshot/resume fixture rows.

## Category Totals

| Category | Count |
|---|---:|
| Active missing feature | 69 |
| External/manual/fixture-only | 41 |
| Preview/out-of-scope | 22 |
| Intentional diagnostic | 10 |
| Old syntax/book rewrite | 8 |
| Proof/design gap | 5 |
| Implemented-but-disabled/stale | 2 |
| **Total** | **157** |

## Area Breakdown

| Area | Total | Active | External | Stale | Old | Diagnostic | Preview | Proof/design |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Language surface | 56 | 22 | 8 | 0 | 5 | 8 | 13 | 0 |
| State/snapshot/distributed/proof | 41 | 13 | 11 | 2 | 2 | 1 | 7 | 5 |
| Comptime/extensions/tooling | 30 | 6 | 22 | 0 | 1 | 1 | 0 | 0 |
| Stdlib/math/domain | 30 | 28 | 0 | 0 | 0 | 0 | 2 | 0 |
| **Total** | **157** | **69** | **41** | **2** | **8** | **10** | **22** | **5** |

There are still zero disabled rows under `stdlib/native/**`. The native-C rows
below are `advanced/native-c-interop.mdx` examples that need DuckDB/Arrow/native
fixtures, so they are counted in the comptime/extensions/tooling area.

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

Intentional diagnostics, 8: `advanced/ownership-deep-dive.mdx:141`, `:399`,
`:425`; `fundamentals/functions.mdx:229`;
`fundamentals/objects-arrays.mdx:37`; `fundamentals/references-borrowing.mdx:30`,
`:192`, `:253`.

Old syntax/book rewrite, 5: `advanced/ownership-deep-dive.mdx:259`;
`fundamentals/error-handling.mdx:90`; `fundamentals/references-borrowing.mdx:269`;
`fundamentals/strings.mdx:397`; `fundamentals/tables.mdx:125`.

### State, Snapshot, Distributed, Proof

Active missing features, 13: `advanced/content-addressed-bytecode.mdx:154`,
`:168`, `:226`, `:264`, `:541`; `stdlib/core/state.mdx:163`, `:216`, `:234`,
`:327`, `:394`, `:477`, `:507`, `:534`.

External/manual/fixture-only, 11: `advanced/content-addressed-bytecode.mdx:515`;
`advanced/module-distribution.mdx:563`; `advanced/polyglot-distributed.mdx:149`;
`advanced/security-permissions.mdx:441`; `advanced/wire-protocol.mdx:90`;
`stdlib/core/remote.mdx:42`, `:77`, `:107`, `:220`;
`stdlib/core/transport.mdx:61`, `:95`.

Implemented-but-disabled/stale, 2: `advanced/resumability.mdx:21`, `:105`.
Snapshot/resume exists elsewhere, but these rows still need a deterministic
two-run harness or conversion to prose before they are truthful book rows.

Preview/out-of-scope, 7: `advanced/content-addressed-bytecode.mdx:321`;
`advanced/security-permissions.mdx:329`, `:346`, `:360`, `:383`, `:466`, `:498`.

Proof/design gaps, 5: `advanced/developer-tools.mdx:86`, `:137`, `:238`, `:320`,
`:462`. These are planned `std::debug`/proof APIs rather than shipped Shape
stdlib surfaces.

Old syntax/book rewrite, 2: `advanced/content-addressed-bytecode.mdx:282`,
`:396`. Both still use retired `__original__(args)` forwarding.

Intentional diagnostic, 1: `advanced/security-permissions.mdx:414`.

### Comptime, Extensions, Tooling

Active missing features, 6: `advanced/annotations.mdx:73`, `:89`;
`advanced/comptime-annotations-cookbook.mdx:31`;
`advanced/comptime-llm-patterns.mdx:170`; `advanced/comptime.mdx:266`;
`tooling/polyglot.mdx:96`.

External/manual/fixture-only, 22: `advanced/annotations.mdx:480`, `:508`;
`advanced/comptime-annotations-cookbook.mdx:183`, `:329`;
`advanced/native-c-interop.mdx:139`, `:155`, `:286`; `tooling/extensions.mdx:120`;
`tooling/polyglot.mdx:14`, `:126`, `:186`; `tooling/python-extension.mdx:68`,
`:117`, `:142`, `:163`, `:184`, `:197`;
`tooling/typescript-extension.mdx:74`, `:134`, `:163`, `:180`, `:238`.

Intentional diagnostic, 1: `advanced/comptime.mdx:76`.

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

## Rows That Should Stay Disabled For Now

External/manual rows should stay disabled unless the book gate grows the needed
fixture class. This includes live HTTP/filesystem/module examples, extension
runtimes, DuckDB/Arrow/native libraries, live `shape serve` or framed transport
peers, selected snapshot stores, permission-configured execution contexts, and
external package/module bundles.

Preview/out-of-scope rows should stay disabled or become prose until the shipped
surface exists. This includes v0.4 content/table rendering ideas, ownership
storage-class pinning and concurrency primitives, host-side security grant APIs,
FaaS policy sketches, and IoT domain package examples.

Intentional diagnostics should stay disabled until the extractor supports
expected-fail snippets. The current diagnostic set is the 10 rows listed above:
ownership/use-after-move/borrow errors, bad named arguments, out-of-bounds
access, permission denial, and the comptime scope violation.

## Recommended Next Lanes

1. External fixture lane for book truth. Extend fixture support beyond the
   current live-serve rows to extension runtimes, Python/TypeScript scalar/error
   rows, live transport peers, selected snapshot stores, permission contexts,
   native DuckDB/Arrow fixtures, and multi-file/module fixtures. This is the
   largest count reducer: 41 rows.

2. State/resume/content-addressed completeness. Finish public `state.resume`,
   resumable `FrameState`, full `VmState`, arbitrary serialize/deserialize,
   object/path/module deltas, and cache/dispatch helpers. This owns 13 active
   rows plus the 2 stale resumability rows.

3. Trait, conversion, error-context, Result, and generic-call inference. Cover
   `From`/`TryFrom`/`TryInto`, `as Type?`, `!!`, named/generic trait dispatch,
   associated types, `HashMap.keys/values/entries`, imported generic
   `assert_eq`/`assert_ne`, and Result method dispatch.

4. Stdlib math/statistics/stochastic/domain carriers. Finish statistic
   intrinsics, stochastic process carriers, `dist_sample_n`,
   `monte_carlo_stats`, public `Mat<number>` construction, optimizer options,
   physics strict-module readiness, and simulation/domain table carriers.

5. Comptime typed-generation and annotation-target semantics. Finish
   expression/await annotation targets, connector-generated typed returns,
   TypeRef/typed fragment generation beyond source strings, and the
   `Vec<struct>` foreign-marshalling return gap.

## Uncertainty

This was a static classification. The counts are exact against the manifest,
but runtime status was not reprobed. DateTime fixed-time rows are counted as
active missing features because Wave-31 probes reported VM success but JIT
segfaults. Some remote/annotation rows may become fixture rows quickly now that
`fixture=serve` exists, but they remain external/manual until the exact book
row has a deterministic fixture.
