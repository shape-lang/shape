# Wave 24 Disabled Current Triage

Date: 2026-07-09
Role: Wave-24B current disabled-book triage worker

Scope honored:

- Wrote only this report.
- Used the current sibling manifest at
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Used sibling book source pages and existing cluster audit docs for static
  classification.
- Did not use stale `/tmp/shape-current-book-snippets`.
- Did not run cargo, just, nextest, rustc, build, test, or book-truth commands.
- Did not edit `AGENTS.md`.

## Sources

- Current manifest: generated `2026-07-09T21:02:13.224Z`, 745 snippets.
- Sibling book pages under
  `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/**`.
- Prior/current reports:
  `docs/cluster-audits/wave22-disabled-current-triage.md`,
  `docs/cluster-audits/wave22-real-async-next-lane.md`,
  `docs/cluster-audits/wave22-comptime-typed-ergonomics.md`,
  `docs/cluster-audits/wave22-global-proof-gap-map.md`,
  `docs/cluster-audits/wave23-semantic-proof-bridge-plan.md`, and
  `docs/cluster-audits/wave20-current-state.md`.

## Current Counts

| Metric | Count |
|---|---:|
| Total snippets | 745 |
| Runnable snippets | 535 |
| Disabled snippets | 210 |
| Deferred snippets | 0 |

Wave 23 moved the old low-risk book-only rows out of the disabled set: prose-only
fences were removed, the basic table row literal is runnable, and bounded current
`state.capture()` / `state.args()` examples are runnable. The remaining disabled
set is now weighted toward implementation, fixture, proof, and v0.4 preview
surfaces.

## Primary Buckets

Each disabled snippet is counted once by first dispatch owner. "External/manual"
includes live network/process/stdin examples, host-state examples, extension
runtime examples, and examples that need a controlled fixture even if some
underlying implementation exists.

| Bucket | Count | Current read |
|---|---:|---|
| External/manual/fixture | 78 | Native IO/HTTP/env/archive/time, live remote/transport examples, Python/TypeScript/native-C/DuckDB/polyglot fixtures, module/package inputs |
| Active implementation gap | 69 | State/resume/diff, traits/conversions/testing/property testing, DateTime JIT parity, math/statistics/stochastic carriers, ownership/CoW edges, comptime/annotation target gaps |
| Preview/out-of-scope | 27 | v0.4 content traits/adapters, named join results, table query DSL/loaders, ownership primitives, domain simulation APIs |
| Proof/design gap | 25 | Security permissions, developer tools, transport/content-addressed composition, wire protocol, remote annotation proof surfaces |
| Intentional diagnostic | 6 | Use-after-move, dangling references, out-of-bounds, bad named args, comptime scope negative |
| Stale-green/count-reduction candidate | 3 | Resumability hash expected-pattern support, stale comptime-codegen warning, definition-only `Drop` trait example |
| Old syntax/book rewrite | 2 | Frontmatter executable fence and reference/variable grammar sketch |
| Total | 210 | |

## Exact Page Counts

| Page | Disabled | First honest owner |
|---|---:|---|
| `stdlib/core/state.mdx` | 14 | State/resume/diff worker |
| `stdlib/native/io.mdx` | 12 | Native fixture/manual split |
| `advanced/ownership-deep-dive.mdx` | 10 | Ownership/CoW plus preview/diagnostic split |
| `advanced/content-addressed-bytecode.mdx` | 9 | State carriers plus distributed proof |
| `stdlib/native/http.mdx` | 9 | Loopback HTTP fixture |
| `advanced/security-permissions.mdx` | 8 | Security/proof API design |
| `advanced/transport-layer.mdx` | 7 | Transport/distributed proof |
| `stdlib/core/remote.mdx` | 7 | Remote fixture/proof matrix |
| `fundamentals/error-handling.mdx` | 6 | Conversion traits and AnyError surface |
| `fundamentals/traits.mdx` | 6 | Trait dispatch/conversion worker |
| `tooling/python-extension.mdx` | 6 | Python extension fixture |
| `advanced/developer-tools.mdx` | 5 | Debug/proof API design |
| `fundamentals/references-borrowing.mdx` | 5 | Ownership/borrow semantics and book rewrite |
| `tooling/typescript-extension.mdx` | 5 | TypeScript extension fixture |
| `advanced/annotations.mdx` | 4 | Annotation targets plus remote proof |
| `advanced/comptime-annotations-cookbook.mdx` | 4 | Comptime/extension fixture split |
| `fundamentals/content.mdx` | 4 | v0.4 content trait/adapters |
| `fundamentals/tables.mdx` | 4 | v0.4 table/query DSL |
| `stdlib/core/property_testing.mdx` | 4 | Testing/property carrier worker |
| `stdlib/core/stochastic.mdx` | 4 | Stochastic carrier worker |
| `stdlib/core/testing.mdx` | 4 | Testing/generic Result helper worker |
| `stdlib/native/archive.mdx` | 4 | Archive fixture/API lane |
| `stdlib/native/env.mdx` | 4 | Host env fixture/manual split |
| `tooling/polyglot.mdx` | 4 | Polyglot extension fixture |
| `advanced/native-c-interop.mdx` | 3 | Native-C/DuckDB/Arrow fixture |
| `advanced/polyglot-distributed.mdx` | 3 | Polyglot distributed fixture/proof |
| `fundamentals/datetime.mdx` | 3 | DateTime JIT parity |
| `fundamentals/resource-management.mdx` | 3 | Async resource fixtures plus one book candidate |
| `fundamentals/strings.mdx` | 3 | v0.4 content-formatting surface |
| `stdlib/core/math.mdx` | 3 | Statistics/math carriers |
| `stdlib/domain/simulation.mdx` | 3 | Domain simulation preview |
| `stdlib/native/time.mdx` | 3 | Async time/live polling fixture |
| `advanced/comptime.mdx` | 2 | Diagnostic plus DuckDB fixture |
| `advanced/resumability.mdx` | 2 | Snapshot hash pattern/resume preview |
| `fundamentals/functions.mdx` | 2 | Intentional diagnostic plus Python fixture |
| `fundamentals/modules.mdx` | 2 | Package/module fixture |
| `fundamentals/objects-arrays.mdx` | 2 | Diagnostic plus HashMap method carrier |
| `fundamentals/operators.mdx` | 2 | Error-context/conversion operators |
| `fundamentals/variables.mdx` | 2 | CoW plus filesystem helper fixture |
| `stdlib/core/transport.mdx` | 2 | Transport fixture |
| `stdlib/domain/iot.mdx` | 2 | Domain IoT preview |
| `stdlib/domain/physics.mdx` | 2 | Domain physics preview |
| `stdlib/math/optimize.mdx` | 2 | Optimizer typed-array carrier |
| `stdlib/math/rotation.mdx` | 2 | Mat/rotation carrier |
| `advanced/comptime-llm-patterns.mdx` | 1 | Typed fragment/source-string gap |
| `advanced/module-distribution.mdx` | 1 | External package fixture |
| `advanced/wire-protocol.mdx` | 1 | Wire/state proof |
| `examples/comptime-codegen.mdx` | 1 | Stale comptime book candidate |
| `examples/web-request.mdx` | 1 | HTTP fixture |
| `fundamentals/async.mdx` | 1 | v0.4 named join results |
| `stdlib/core/distributions.mdx` | 1 | Distribution carrier |
| `stdlib/core/monte_carlo.mdx` | 1 | Monte Carlo carrier |
| `stdlib/domain/finance.mdx` | 1 | Finance package fixture |
| `stdlib/math/interpolation.mdx` | 1 | Mat/interpolation carrier |
| `tooling/execution-server.mdx` | 1 | Remote server fixture |
| `tooling/extensions.mdx` | 1 | DuckDB extension fixture |
| `tooling/frontmatter.mdx` | 1 | Book/frontmatter rewrite |

## Highest-Value Next Waves

1. State/resume/content-addressed completeness.
   Own `crates/shape-vm/src/executor/state_builtins/{core,introspection}.rs`,
   `crates/shape-vm/src/executor/{snapshot.rs,resume.rs,vm_state_snapshot.rs}`,
   `crates/shape-runtime/stdlib-src/core/state.shape`, focused state tests, and
   sibling pages `stdlib/core/state.mdx`, `advanced/resumability.mdx`, and
   `advanced/content-addressed-bytecode.mdx`. Finish public `VmState`,
   `capture_module`, `capture_call`, `resume`, `resume_frame`, `diff`, `patch`,
   and honest `caller`/`locals` examples before widening docs.

2. Native, extension, and host fixture split.
   Own sibling pages `stdlib/native/{io,http,archive,env,time}.mdx`,
   `tooling/{python-extension,typescript-extension,polyglot,extensions}.mdx`,
   `advanced/native-c-interop.mdx`, and fixture support under
   `bin/shape-cli/tests/support/**` if implementation proof is needed. Expected
   reduction is large, but only for deterministic loopback, tempdir, archive, and
   extension-artifact rows. Keep live network, stdin, file watcher, host env, and
   third-party DuckDB/manual rows out of the default gate unless controlled.

3. Traits, conversions, testing, and property testing.
   Own compiler trait/conversion dispatch in
   `crates/shape-vm/src/compiler/{statements.rs,expressions/**}`,
   runtime/stdlib conversion and testing helpers under
   `crates/shape-runtime/stdlib-src/core/**`, and focused ShapeTest rows for
   named impls, generic traits, associated types, `From`/`TryFrom`,
   `Into`/`TryInto`, `!!`, imported generic assertions, `Result` method dispatch,
   and property-testing schemas. This clears shared blockers across
   `traits.mdx`, `error-handling.mdx`, `operators.mdx`, `testing.mdx`, and
   `property_testing.mdx`.

4. Distributed/transport/security proof matrix.
   Own `crates/shape-vm/src/remote.rs`,
   `crates/shape-vm/src/executor/builtins/remote_builtins.rs`,
   `bin/shape-cli/src/commands/serve_cmd.rs`, distributed e2e tests, and sibling
   pages `stdlib/core/{remote,transport}.mdx`,
   `advanced/{transport-layer,security-permissions,wire-protocol}.mdx`.
   Coordination note: Wave-24A is actively working prompt
   `remote::call_async` cancellation and has not closed it. Do not claim or
   overlap that promptness fix; use its eventual result as input for later book
   and proof rows.

5. Comptime annotations and typed generation ergonomics.
   Own `crates/shape-vm/src/compiler/{comptime.rs,comptime_target.rs,comptime_builtins.rs,functions_annotations.rs,statements.rs}`,
   stdlib derives, and focused `tools/shape-test/tests/{comptime,annotations_comptime}/**`.
   Wave 23 added TypeRef-first reflection; the next useful slice is expression
   and await annotation target proof, typed fragments/quasiquote or a smaller
   typed directive payload bridge, and book cleanup for stale `comptime for`
   warnings and source-string-only examples.

Count-only runner-up: math/statistics/stochastic/domain carriers plus DateTime
JIT parity would reduce a noticeable number of rows, especially
`stdlib/core/{math,stochastic,distributions,monte_carlo}.mdx`,
`stdlib/math/{rotation,interpolation,optimize}.mdx`, and
`fundamentals/datetime.mdx`. It is lower strategic priority than state,
distributed proof, and trait/testing blockers.

## Notes

- Category counts are static dispatch classifications, not root-cause proofs for
  every snippet.
- `runnable=true` book truth remains green by prior supervisor gate evidence;
  this report only classifies the 210 disabled rows.
- Several disabled examples are intentionally not good flip candidates even if
  they parse: live remotes, extension definitions without artifacts, v0.4
  previews, intentional diagnostics, and security host-embedding pseudocode.
