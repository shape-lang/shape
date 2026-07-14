# Wave 19 Disabled Current Triage

Date: 2026-07-09
Role: Wave-19C disabled-corpus current triage scout

Scope honored:

- Static triage over the current manifest and generated snippet files.
- Incorporated supervisor disabled reprobe evidence from
  `/tmp/shape-wave19-disabled-probe.json`.
- No cargo, just, nextest, rustc, build, test, benchmark, or book-truth
  commands were run by this scout.
- No edits outside this report.

## Sources

- Registry policy: `AGENTS.md`.
- Current action map:
  `docs/cluster-audits/wave17-disabled-action-map.md`.
- Current completeness snapshot:
  `docs/cluster-audits/wave14-current-completeness-snapshot.md`.
- Content/container scope:
  `docs/cluster-audits/wave17-content-container-parity-scope.md`.
- Async/distributed next lane:
  `docs/cluster-audits/wave18-real-async-distributed-next-lane.md`.
- JIT heap barrier blocker:
  `docs/cluster-audits/wave18-jit-heap-barrier-blocker.md`.
- Current manifest:
  `../shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`
  generated at `2026-07-09T17:10:05.171Z`.

## Current Counts

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 511 |
| Disabled snippets | 245 |

Supervisor reprobe over the 245 disabled snippets with the existing release
binary:

| Probe class | Count |
|---|---:|
| `both-fail` | 227 |
| `disabled-but-green` | 13 |
| `jit-only-fail` | 3 |
| `vm-only-fail` | 1 |
| `output-divergence` | 1 |

Interpretation: disabled-but-green is a review queue, not an automatic flip
queue. Several green snippets are definition-only, comment-only, or external
extension declarations that pass because they do not exercise the advertised
behavior.

## Top Disabled Pages

| Page | Disabled | First honest lane |
|---|---:|---|
| `stdlib/native/io.mdx` | 25 | deterministic native fixture split; leave sockets/stdin/process manual |
| `stdlib/core/state.mdx` | 16 | state capture/resume/diff/full introspection |
| `advanced/ownership-deep-dive.mdx` | 10 | separate intentional diagnostics from preview primitives |
| `advanced/content-addressed-bytecode.mdx` | 9 | state/resume plus transport/store proof |
| `advanced/security-permissions.mdx` | 9 | security/proof APIs and prose rewrites |
| `stdlib/native/http.mdx` | 9 | loopback HTTP fixture plus async/native HTTP surface |
| `fundamentals/content.mdx` | 8 | preview content traits/content-string rewrites after content-array parity |
| `advanced/transport-layer.mdx` | 7 | distributed transport proof matrix |
| `stdlib/core/remote.mdx` | 7 | serve fixtures, stale internal remote call examples |
| `fundamentals/error-handling.mdx` | 6 | conversion traits and `!!` ergonomics |
| `fundamentals/traits.mdx` | 6 | named impls, generic traits, conversion traits, associated types |
| `tooling/python-extension.mdx` | 6 | extension fixture/ABI rebuild lane |
| `tooling/typescript-extension.mdx` | 5 | extension fixture/ABI rebuild lane |
| `stdlib/native/archive.mdx` | 5 | deterministic archive carrier/API lane |
| `stdlib/native/file.mdx` | 5 | deterministic filesystem fixture lane |
| `fundamentals/tables.mdx` | 5 | basic table literal probe; keep loaders/query DSL preview disabled |
| `fundamentals/references-borrowing.mdx` | 5 | intentional diagnostics plus CoW/JIT alias path |
| `advanced/developer-tools.mdx` | 5 | debug/proof preview APIs |

## Primary Buckets

These are static primary classifications. Some snippets could move after a
focused reprobe or a fixture rewrite, but each row has one dispatch owner.

| Bucket | Count | Read |
|---|---:|---|
| Active implementation gaps | 81 | state, traits/conversion/testing, async join values, DateTime JIT, Mat/table/statistical carriers, archive carriers, CoW/JIT |
| External/manual/fixture-needed | 82 | IO/file/env/HTTP, live remotes, extension examples, native C/DuckDB, module/file fixtures |
| Preview/out-of-scope | 27 | content traits/render adapters, tables query DSL, simulation/domain previews, ownership primitives |
| Proof/design gaps | 23 | security permissions, transport proofs, content-addressed composition, developer tools/proofs |
| Stale-green review candidates | 13 | exact disabled-but-green list from the Wave-19 reprobe |
| Old syntax/book rewrites | 9 | retired content strings, prose-only snippets, docstrings, built-in enum sketches |
| Intentional diagnostics | 10 | use-after-move, bad named args, out-of-bounds, escaping references, comptime error examples |
| Total | 245 | |

## Reprobe Review Queue

Disabled-but-green review candidates:

- `advanced/resumability.mdx:L105`: function definition only; invoke or convert
  to prose.
- `advanced/security-permissions.mdx:L162`: comment-only permission mapping;
  convert to prose or a real negative harness.
- `fundamentals/content.mdx:L47`, `L55`, `L475`: definitions only / preview
  content trait material.
- `fundamentals/datetime.mdx:L12`: import-only smoke; flip only if import
  smokes are considered useful.
- `fundamentals/functions.mdx:L185`: trait-bound type illustration; needs a
  real call or prose.
- `fundamentals/functions.mdx:L421`: Python extension definition; keep fixture
  gated.
- `fundamentals/traits.mdx:L387`: associated-type definition; behavior remains
  a trait gap.
- `getting-started/basic-concepts.mdx:L149`: type declaration plus comments;
  likely prose.
- `stdlib/native/io.mdx:L449`: host-dependent `git log`; rewrite to a
  deterministic command fixture or keep disabled.
- `tooling/python-extension.mdx:L163` and
  `tooling/typescript-extension.mdx:L180`: async extension definitions only;
  keep extension-fixture gated.

Other non-both-fail rows:

- JIT-only failures: `fundamentals/datetime.mdx:L23`, `L368`, `L408`.
  Treat as DateTime JIT parity, not book drift.
- VM-only failure: `advanced/ownership-deep-dive.mdx:L459`.
  This is a preview ownership primitive surface, not a safe flip.
- Output divergence: `advanced/resumability.mdx:L21`.
  Snapshot hashes differ VM/JIT; needs expected-pattern support or deterministic
  rewrite before default gate inclusion.

## Count-Reduction Candidates

Best review candidates after a supervisor probe:

- Prose/definition cleanup: security `L162`, datetime `L12`, basic concepts
  `L149`, docstrings `L118`/`L124`, enums `L179`, content `L47`/`L55`/`L475`.
- Deterministic native fixture candidates: archive `L37`/`L49`/`L62`/`L73`/`L79`,
  file `L28`/`L38`/`L46`/`L57`, env `L48`/`L57`, CSV `L76`, JSON `L267`, and the
  filesystem-only subset of native IO.
- Table/basic container review: `fundamentals/tables.mdx:L31` remains both-fail
  in the reprobe but prior source inspection says basic `Table<T>` row literal
  support exists. It needs a focused failure read before a book edit.
- Resumability output: `advanced/resumability.mdx:L21` may be count-reducible
  only with expected-pattern support for content-dependent snapshot hashes.

Do not blindly flip:

- Live HTTP, TCP/UDP, stdin, long-running process, and remote-server examples.
- Python/TypeScript/native-C extension examples without rebuilt extension
  fixtures.
- Intentional diagnostics; move them to negative tests or keep disabled with
  precise prose.
- Preview content/table/query/domain APIs that currently pass only as
  definitions.

## Core Lane Blockers

| Lane | Disabled surface |
|---|---|
| State/resume | `stdlib/core/state.mdx` 16; `advanced/content-addressed-bytecode.mdx` 9; `advanced/resumability.mdx` 2; transport/state snippets in `advanced/transport-layer.mdx` |
| Real async/distributed futures | `fundamentals/async.mdx:L123`; async time/http/io snippets; annotation/host await examples; distributed fan-in needs `await join all` values |
| Distributed/snapshot/polyglot | `stdlib/core/remote.mdx`, `stdlib/core/transport.mdx`, `advanced/transport-layer.mdx`, `advanced/polyglot-distributed.mdx`, execution-server, wire-protocol |
| Comptime typed ergonomics | annotation expression/await targets, typed fragments/quasiquote, codegen cookbook, LLM schema generation |
| JIT parity | DateTime JIT-only rows and the heap typed-object mutation barrier blocker from Wave 18 |
| Content/table/Mat containers | preview content traits, table literals/query DSL, public `mat(rows, cols, flat_array)`, optimizer arrays, rotation/interpolation, statistical/stochastic carriers |
| Traits/conversion/testing | `From`/`TryFrom`/`Into`/`TryInto`, named impl dispatch, generic traits, associated types, testing Result helpers, property-testing schemas |
| Native IO/HTTP/extension fixtures | native IO/file/env/http/archive/csv/json/time, Python/TypeScript/native-C, frontmatter/extensions |
| Proof/security | security permissions, developer tools, content-addressed transport, wire protocol, global Miri/source-guard gaps |

## Recommended Wave-20 Dispatches

1. Native deterministic fixture/count-reduction worker.
   Own sibling book files only:
   `stdlib/native/{io,file,env,csv,json,archive}.mdx`, and narrow fixture docs if
   the book harness has a fixture convention. No production ownership unless a
   deterministic archive/file API bug is proven and supervisor re-scopes it.
   Expected value: review roughly 42 disabled snippets; likely 10-20 safe
   reductions after release-binary probes, while keeping network/stdin/process
   manual.

2. State capture/resume/diff implementation worker.
   Own `crates/shape-vm/src/executor/state_builtins/{core,introspection}.rs`,
   `crates/shape-vm/src/executor/{snapshot.rs,resume.rs}`,
   `crates/shape-runtime/stdlib-src/core/state.shape`, and focused state tests.
   Do not own distributed transport or book files in the first patch.
   Expected value: retire core blockers behind `state.mdx`, resumability, and
   content-addressed bytecode.

3. Async `join all` value materialization worker.
   Own `crates/shape-vm/src/executor/async_ops/mod.rs`, optional new
   `crates/shape-vm/src/executor/async_join_values.rs`, and focused tests in
   `tools/shape-test/tests/async_concurrency/join_strategies.rs` plus
   `bin/shape-cli/tests/distributed_async_e2e.rs`.
   Expected value: one direct book candidate now, but higher strategic value for
   distributed async fan-out/fan-in and later HTTP/time examples.

4. Container/Mat/statistical carrier worker.
   First slice should be public `mat(rows, cols, flat_array)` and the minimal
   table row-literal failure read. Own
   `crates/shape-vm/src/executor/builtins/datetime_builtins.rs`,
   `crates/shape-vm/src/executor/vm_impl/builtins.rs`, focused compiler/table
   code only if the failure proves it, and math stdlib files
   `rotation.shape`, `interpolation.shape`, or `optimize.shape` only as needed.
   Expected value: unblock rotation/interpolation and clarify whether table
   basics are stale-green or real gaps before touching query DSL pages.

5. Traits/conversion/testing worker.
   Own `crates/shape-vm/src/compiler/statements.rs`,
   `crates/shape-vm/src/compiler/expressions/{function_calls,type_ops,property_access}.rs`,
   `crates/shape-runtime/stdlib-src/core/{into,try_into}.shape`, and
   `crates/shape-runtime/stdlib-src/core/utils/{testing,property_testing}.shape`
   only as needed. Keep table/content traits out of scope.
   Expected value: clears shared blockers across traits, error handling,
   operators, testing, and property-testing pages.

## Uncertainty

- Bucket counts are dispatch classifications, not root-cause proofs for every
  snippet.
- The reprobe uses the current release binary and current local extension
  state. Extension-related failures may change after a fixture rebuild.
- A few snippets are intentionally ambiguous between preview/prose and active
  implementation. The report biases toward dispatch ownership over taxonomy.
