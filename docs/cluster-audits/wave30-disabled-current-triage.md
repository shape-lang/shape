# Wave 30 Current Disabled Book Triage

Date: 2026-07-09
Supervisor: book-truth completeness campaign

## Manifest

Authoritative manifest:
`../shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.

Generated: `2026-07-09T23:40:40.617Z`.

| Metric | Count |
|---|---:|
| Total Shape snippets | 707 |
| Runnable snippets | 541 |
| Disabled snippets | 166 |
| Deferred snippets | 0 |

Last full release-binary book gate before this triage:
`run-p612733-i31250203.service`, 541/541 passed, report
`/tmp/shape-wave29-book-truth-report.json`.

## Source Reports

- `wave30-disabled-language-surface-triage.md` — fundamentals, examples,
  ownership, developer-tools.
- `wave30-disabled-state-distributed-proof-triage.md` — state, content-
  addressed bytecode, remote/transport, resumability, security, distributed
  composition.
- `wave30-disabled-stdlib-native-triage.md` — stdlib native, math, testing,
  property testing, domain modules.
- `wave30-disabled-comptime-extension-triage.md` — comptime, annotations,
  native C, polyglot extension tooling.

Reconciliation note: the language and state/proof reports both classified the
five `advanced/developer-tools.mdx` rows. This synthesis counts them once, using
the language-surface report's ownership. The reports also missed
`tooling/frontmatter.mdx:12`; this synthesis counts it as an external/manual
fixture row because it depends on local module paths and a Python extension.

## Bucket Counts

| Bucket | Count |
|---|---:|
| Active implementation gap | 66 |
| External/manual/fixture/server/env/permission dependent | 48 |
| Preview/out-of-scope | 23 |
| Intentional diagnostic | 10 |
| Old syntax/book rewrite | 8 |
| Stale-green/count-reduction candidate | 6 |
| Proof/design gap | 5 |
| **Total** | **166** |

## What This Means

The remaining disabled manifest is no longer primarily stale book debt. After
Wave-29, the disabled set is mostly real implementation work plus examples that
need deterministic harnesses.

`stdlib/native/**` is no longer the disabled problem: the current manifest has
zero disabled native rows, including `archive.mdx` at 3/3 runnable. Native
archive creation is still not exported, but the disabled book set no longer
contains archive rows.

Distributed/snapshot/polyglot composition is implemented in e2e tests, but the
book rows remain disabled because they need live receivers, FFI/runtime opt-in,
selected snapshot stores, and two-process resume harnesses. They are fixture
gaps for book truth, not evidence that the core distributed path is absent.

Comptime is partially typed: TypeRef and typed descriptor carriers exist, but
source generation still relies on `extend (expr)` / `replace module (expr)`
source strings and connector examples still compute textual type payloads. The
disabled comptime rows reflect that ergonomic/type-safety gap.

Typed field mutation is not a current disabled-book cluster. It remains a proof
and semantic-bridge concern: the next proof lane is still Miri/runtime probes for
typed-object field writes, snapshot/wire restore, JIT FFI return tags, and
trait/object carriers.

## Highest-Value Next Lanes

1. State carriers and resume completeness.

   Targets: `capture_call`, public `state.resume`, resumable `FrameState`,
   full `VmState`, arbitrary-value serialize/deserialize, object/array/map/path
   `Delta`, module-state sync, and function-cache packaging.

   Likely files: `crates/shape-vm/src/executor/state_builtins/{core,introspection}.rs`,
   `crates/shape-vm/src/executor/resume.rs`,
   `crates/shape-runtime/stdlib-src/core/state.shape`, focused state builtin
   tests, and content-addressed/resumability book rows.

2. Trait conversion and error-context inference.

   Targets: source-side `TryInto`, target-side `From`/`TryFrom`, auto-derived
   conversions through `as` / `as?`, associated types, named impl dispatch, and
   `!!` plus `?` composition.

   Likely files: compiler expression/statement lowering, trait lookup in
   `crates/shape-runtime/src/type_system/inference/**`, core conversion stdlib
   modules, and `tools/shape-test/tests/{traits,error_handling,numeric_conversions}/**`.

3. Testing/property testing plus Result method dispatch.

   Targets: imported generic `assert_eq`/`assert_ne`, `assert_ok`/`assert_err`,
   function-field schemas in `PropertySpec<T>`, `run_properties<T>`
   specialization, empty typed-array/result carriers.

   Likely files: `crates/shape-runtime/stdlib-src/core/utils/{testing,property_testing}.shape`,
   generic call-site inference, Result method dispatch, and stdlib testing
   integration tests.

4. Math/stochastic/distribution/Monte Carlo carriers.

   Targets: `dist_sample_n`, `correlation`, `covariance`, `percentile`,
   `monte_carlo_stats`, and stochastic process rows (`brownian_motion`, `gbm`,
   `ou_process`, `random_walk`).

   Likely files: core math/stochastic/distribution stdlib modules, runtime
   intrinsics, VM builtin dispatch, and focused stdlib math tests.

5. Public `Mat<number>` and optimizer carriers.

   Targets: public matrix construction usable by interpolation/rotation rows,
   optimizer typed-array/options construction, and deterministic assertions for
   rotation examples.

6. Comptime typed generation and annotation target semantics.

   Targets: expression/await annotation targets, typed fragments or a typed
   module/function fragment value, source-level TypeRef authoring, and reduced
   reliance on source-string generation.

7. Distributed book fixtures.

   Targets: loopback `shape serve`, selected snapshot store, extension/runtime
   opt-in, receiver FFI policy, controlled unused-port failures, framed
   transport peers, and scripted resume. This is the path to turning currently
   disabled distributed rows into book-gated truth without weakening the real
   distributed story.

8. Polyglot extension fixture and foreign marshalling completeness.

   Targets: deterministic Python/TypeScript scalar/error rows in the book gate,
   object return rows, and the Python `Vec<struct>` return gap.

9. Ownership/CoW/borrow surfaces and expected-error book truth.

   Targets: `var` alias CoW correctness, async borrow diagnostics, expected
   diagnostic support for intentionally failing book snippets, and prose cleanup
   for conceptual ownership examples.

10. Domain simulation/physics readiness.

    Targets: `datatable.simulate`, correlated context typed-object
    construction, event-log replay, physics strict-module inference, and small
    deterministic domain fixtures.

## Book-Only Count Reduction

These do not need broad production work, but still need serialized supervisor
verification before changing the manifest:

- Fixed DateTime rows in `fundamentals/datetime.mdx:364` and `:404`.
- `fundamentals/resource-management.mdx:139` if kept as compile-only truth or
  converted to prose.
- `stdlib/domain/finance.mdx:16` import row, if rewritten to a behavior
  assertion or proven as an import-only smoke.
- `advanced/resumability.mdx:21` and `:105`, likely prose or scripted harness.
- `advanced/content-addressed-bytecode.mdx:282` and `:396`, which still use
  retired `__original__(args)` forwarding.
- `advanced/comptime-llm-patterns.mdx:170` and
  `advanced/comptime-annotations-cookbook.mdx:308`, likely prose/text until
  typed fragments or policy fixtures exist.
- `tooling/frontmatter.mdx:12`, likely text/prose unless the book gate grows a
  frontmatter script fixture with local modules/extensions.

## Disabled Set Shape

The current disabled set is therefore:

- 66 rows that require real implementation.
- 48 rows that are implemented or plausible but need deterministic external
  fixtures/harnesses.
- 23 rows that describe preview/out-of-scope surfaces.
- 10 intentional diagnostics that need expected-fail support or prose.
- 8 rows that are old syntax or should be non-Shape prose.
- 6 likely count-reduction candidates.
- 5 proof/design API rows.

The most direct route toward 100% truthful coverage is not more stale flips. It
is a sequence of implementation lanes plus a book-fixture lane for distributed,
polyglot, transport, permissions, and frontmatter examples.
