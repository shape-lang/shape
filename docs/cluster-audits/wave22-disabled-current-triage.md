# Wave 22 Disabled Current Triage

Date: 2026-07-09
Role: Wave-22A post-Wave-21 disabled triage worker

Scope honored:

- Wrote exactly this report.
- Read current sibling manifest, sibling book MDX, and prior disabled/current
  state reports.
- Did not edit `AGENTS.md`.
- Did not run cargo, just, nextest, rustc, build, test, or book-truth commands.

## Sources

- Current manifest:
  `../shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`,
  generated at `2026-07-09T20:23:13.040Z`.
- Current sibling book pages under
  `../shape-web/book/book-site/src/content/docs/**`.
- Prior reports:
  `docs/cluster-audits/wave19-disabled-current-triage.md`,
  `docs/cluster-audits/wave20-current-state.md`,
  `docs/cluster-audits/wave17-disabled-action-map.md`,
  `docs/cluster-audits/wave17-content-container-parity-scope.md`,
  `docs/cluster-audits/wave18-real-async-distributed-next-lane.md`,
  and `docs/cluster-audits/wave18-jit-heap-barrier-blocker.md`.

## Current Counts

Supervisor-verified current state:

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 532 |
| Disabled snippets | 224 |
| Deferred snippets | 0 |

Wave 21 moved 21 deterministic native examples into the runnable set. The
classification below treats that movement as a reduction in the
external/manual/fixture bucket, not as an implementation-gap reduction.

## Top Disabled Pages

| Page | Disabled | First honest lane |
|---|---:|---|
| `stdlib/core/state.mdx` | 16 | state capture/resume/diff plus bounded introspection doc rewrites |
| `stdlib/native/io.mdx` | 12 | keep network/stdin/process/watcher manual; fixture only where deterministic |
| `advanced/ownership-deep-dive.mdx` | 10 | storage-class preview, CoW/JIT, and intentional diagnostics |
| `advanced/content-addressed-bytecode.mdx` | 9 | state/resume, distributed state transfer, content-addressed proofs |
| `advanced/security-permissions.mdx` | 9 | v0.4 host/security API proof surface and prose cleanup |
| `stdlib/native/http.mdx` | 9 | loopback HTTP fixture plus async/native HTTP semantics |
| `fundamentals/content.mdx` | 8 | preview content traits/adapters plus stale definition-only snippets |
| `advanced/transport-layer.mdx` | 7 | transport API, QUIC, memoization, distributed composition proofs |
| `stdlib/core/remote.mdx` | 7 | remote harness and low-level remote call examples |
| `fundamentals/error-handling.mdx` | 6 | conversion traits, AnyError fields, `!!` inference |
| `fundamentals/traits.mdx` | 6 | named impls, generic traits, conversion traits, associated types |
| `tooling/python-extension.mdx` | 6 | extension fixture/runtime availability |
| `tooling/typescript-extension.mdx` | 5 | extension fixture/runtime availability |
| `fundamentals/tables.mdx` | 5 | one row-literal review candidate; rest v0.4 query/table APIs |
| `fundamentals/references-borrowing.mdx` | 5 | intentional diagnostics, CoW/JIT, task-boundary preview |
| `advanced/developer-tools.mdx` | 5 | debug/proof APIs not exposed as Shape-level stdlib yet |

## Primary Buckets

These are primary dispatch buckets. A few snippets have secondary causes, but
each disabled row is counted once.

| Bucket | Count | Current read |
|---|---:|---|
| Active implementation gap | 81 | state/resume/diff, traits/conversion/testing, DateTime JIT parity, comptime target ergonomics, Mat/table/statistics carriers |
| External/manual/fixture | 61 | live IO/HTTP/process/stdin/watcher, extension runtimes, host env, archive fixtures, app/module inputs |
| Preview/out-of-scope | 27 | v0.4 content traits/adapters, table query DSL/loaders, domain simulation APIs, ownership primitives |
| Proof/design gap | 23 | security permissions, developer tools, transport/content-addressed proof surfaces |
| Stale-green/count-reduction candidate | 13 | current line-mapped review queue from disabled-but-green/prose-only candidates |
| Old syntax/book rewrite | 9 | retired content strings, conceptual enum/docstring snippets, prose-only sketches |
| Intentional diagnostic | 10 | use-after-move, borrow escape/conflict, invalid operators/args, negative comptime examples |
| Total | 224 | |

## Best Count-Reduction Candidates

These are not automatic flips. They are the best current candidates for a
book-only Wave-23 pass followed by supervisor verification.

| Candidate | Current line(s) | Why it is count-reducible |
|---|---|---|
| Prose/comment-only permission mapping | `../shape-web/book/book-site/src/content/docs/advanced/security-permissions.mdx:162` | Contains only comments about permission checks; convert to prose or a real negative harness. |
| Definition-only/prose snippets | `../shape-web/book/book-site/src/content/docs/getting-started/basic-concepts.mdx:149`, `../shape-web/book/book-site/src/content/docs/tooling/docstrings.mdx:118`, `../shape-web/book/book-site/src/content/docs/tooling/docstrings.mdx:124`, `../shape-web/book/book-site/src/content/docs/fundamentals/enums.mdx:179` | These are conceptual declarations or doc-comment examples, not useful runtime book snippets. |
| Content preview definitions | `../shape-web/book/book-site/src/content/docs/fundamentals/content.mdx:47`, `../shape-web/book/book-site/src/content/docs/fundamentals/content.mdx:55`, `../shape-web/book/book-site/src/content/docs/fundamentals/content.mdx:475` | Current page already says these v0.4 content trait/adapter examples are preview-only. Convert to prose or keep disabled with explicit preview text. |
| Import/type illustration smokes | `../shape-web/book/book-site/src/content/docs/fundamentals/datetime.mdx:12`, `../shape-web/book/book-site/src/content/docs/fundamentals/functions.mdx:185`, `../shape-web/book/book-site/src/content/docs/fundamentals/traits.mdx:387` | Green or near-green as declarations, but low value unless rewritten to assert behavior. |
| Bounded state introspection rewrites | `../shape-web/book/book-site/src/content/docs/stdlib/core/state.mdx:138`, `../shape-web/book/book-site/src/content/docs/stdlib/core/state.mdx:403`, `../shape-web/book/book-site/src/content/docs/stdlib/core/state.mdx:417`, `../shape-web/book/book-site/src/content/docs/stdlib/core/state.mdx:428` | Current runtime has real bounded `capture`, `caller`, `args`, and `locals` slices, but the page still describes broader `Any`/full-frame behavior. Rewrite to current scalar/string cases before probing. |
| Basic table row literal | `../shape-web/book/book-site/src/content/docs/fundamentals/tables.mdx:31` | Prior content/container scope found compiler/runtime support for the basic row-literal shape; needs a focused current failure read before flipping. |
| Host-dependent command example | `../shape-web/book/book-site/src/content/docs/stdlib/native/io.mdx:538` | `git log` is a real command example but host/repo dependent. Rewrite to a deterministic fixture command or keep manual. |
| Extension definition-only snippets | `../shape-web/book/book-site/src/content/docs/fundamentals/functions.mdx:421`, `../shape-web/book/book-site/src/content/docs/tooling/python-extension.mdx:163`, `../shape-web/book/book-site/src/content/docs/tooling/typescript-extension.mdx:180` | These can pass as definitions, but should stay gated until the extension fixture lane proves invocation. |
| Function-level snapshot definition | `../shape-web/book/book-site/src/content/docs/advanced/resumability.mdx:105` | Defines a function without invoking it. Needs deterministic invocation/output or should remain explanatory. |

Do not blindly flip `advanced/resumability.mdx:21`: current hashes are
per-run/content dependent, so it needs expected-pattern support or a deterministic
rewrite before joining the default gate.

## Highest-Priority Implementation Gaps

All paths in this table are under
`../shape-web/book/book-site/src/content/docs/`.

| Lane | Exact disabled surface |
|---|---|
| State/resume completeness | `stdlib/core/state.mdx:150`, `stdlib/core/state.mdx:161`, `stdlib/core/state.mdx:173`, `stdlib/core/state.mdx:203`, `stdlib/core/state.mdx:221`, `stdlib/core/state.mdx:314`, `stdlib/core/state.mdx:354`, `stdlib/core/state.mdx:374`, `stdlib/core/state.mdx:382`, `stdlib/core/state.mdx:442`, `stdlib/core/state.mdx:472`, `stdlib/core/state.mdx:499`; also `advanced/content-addressed-bytecode.mdx:154`, `advanced/content-addressed-bytecode.mdx:168`, `advanced/content-addressed-bytecode.mdx:226`, `advanced/content-addressed-bytecode.mdx:264`, `advanced/content-addressed-bytecode.mdx:396`, `advanced/content-addressed-bytecode.mdx:515`, `advanced/content-addressed-bytecode.mdx:541` |
| Distributed/remote/transport composition | `stdlib/core/remote.mdx:36`, `stdlib/core/remote.mdx:68`, `stdlib/core/remote.mdx:95`, `stdlib/core/remote.mdx:121`, `stdlib/core/remote.mdx:139`, `stdlib/core/remote.mdx:154`, `stdlib/core/remote.mdx:185`; `advanced/transport-layer.mdx:79`, `advanced/transport-layer.mdx:213`, `advanced/transport-layer.mdx:282`, `advanced/transport-layer.mdx:347`, `advanced/transport-layer.mdx:367`, `advanced/transport-layer.mdx:401`, `advanced/transport-layer.mdx:436`; `advanced/polyglot-distributed.mdx:71`, `advanced/polyglot-distributed.mdx:143`, `advanced/polyglot-distributed.mdx:203` |
| Traits, conversions, and testing | `fundamentals/traits.mdx:71`, `fundamentals/traits.mdx:172`, `fundamentals/traits.mdx:249`, `fundamentals/traits.mdx:265`, `fundamentals/traits.mdx:330`, `fundamentals/traits.mdx:387`; `fundamentals/error-handling.mdx:186`, `fundamentals/error-handling.mdx:207`, `fundamentals/error-handling.mdx:275`, `fundamentals/error-handling.mdx:287`; `stdlib/core/testing.mdx:44`, `stdlib/core/testing.mdx:59`, `stdlib/core/testing.mdx:88`, `stdlib/core/testing.mdx:103`; `fundamentals/operators.mdx:436`, `fundamentals/operators.mdx:503` |
| Comptime typed ergonomics and policy annotations | `advanced/annotations.mdx:73`, `advanced/annotations.mdx:89`, `advanced/annotations.mdx:480`, `advanced/annotations.mdx:508`; `advanced/comptime-annotations-cookbook.mdx:31`, `advanced/comptime-annotations-cookbook.mdx:183`, `advanced/comptime-annotations-cookbook.mdx:308`, `advanced/comptime-annotations-cookbook.mdx:329`; `advanced/comptime.mdx:266`; `examples/comptime-codegen.mdx:22` |
| Containers, Mat, statistics, and property carriers | `stdlib/math/interpolation.mdx:51`; `stdlib/math/rotation.mdx:32`, `stdlib/math/rotation.mdx:43`; `stdlib/math/optimize.mdx:58`, `stdlib/math/optimize.mdx:78`; `stdlib/core/math.mdx:70`, `stdlib/core/math.mdx:86`, `stdlib/core/math.mdx:102`; `stdlib/core/stochastic.mdx:30`, `stdlib/core/stochastic.mdx:47`, `stdlib/core/stochastic.mdx:64`, `stdlib/core/stochastic.mdx:80`; `stdlib/core/property_testing.mdx:32`, `stdlib/core/property_testing.mdx:49`, `stdlib/core/property_testing.mdx:77` |
| DateTime JIT parity | `fundamentals/datetime.mdx:23`, `fundamentals/datetime.mdx:368`, `fundamentals/datetime.mdx:408` |
| Native async/HTTP/IO fixture boundary | `stdlib/native/http.mdx:39`, `stdlib/native/http.mdx:50`, `stdlib/native/http.mdx:64`, `stdlib/native/http.mdx:73`, `stdlib/native/http.mdx:82`, `stdlib/native/http.mdx:100`, `stdlib/native/http.mdx:109`, `stdlib/native/http.mdx:118`, `stdlib/native/http.mdx:136`; `stdlib/native/io.mdx:212`, `stdlib/native/io.mdx:222`, `stdlib/native/io.mdx:249`, `stdlib/native/io.mdx:279`, `stdlib/native/io.mdx:296`, `stdlib/native/io.mdx:308`, `stdlib/native/io.mdx:340`, `stdlib/native/io.mdx:348`, `stdlib/native/io.mdx:377`, `stdlib/native/io.mdx:520`, `stdlib/native/io.mdx:538`, `stdlib/native/io.mdx:557`; `stdlib/native/archive.mdx:40`, `stdlib/native/archive.mdx:79`, `stdlib/native/archive.mdx:93`, `stdlib/native/archive.mdx:102` |
| Security/proof/developer tools | `advanced/security-permissions.mdx:333`, `advanced/security-permissions.mdx:350`, `advanced/security-permissions.mdx:364`, `advanced/security-permissions.mdx:387`, `advanced/security-permissions.mdx:418`, `advanced/security-permissions.mdx:445`, `advanced/security-permissions.mdx:470`, `advanced/security-permissions.mdx:502`; `advanced/developer-tools.mdx:86`, `advanced/developer-tools.mdx:137`, `advanced/developer-tools.mdx:238`, `advanced/developer-tools.mdx:320`, `advanced/developer-tools.mdx:462`; `advanced/wire-protocol.mdx:90` |

## Recommended Wave-23 Dispatches

1. Book-only count-reduction worker.
   Own only sibling book pages. Start with prose/comment snippets, current
   bounded state introspection examples, and the basic table row-literal probe.
   Expected value: small but low-risk reduction, roughly 8-15 snippets if
   supervisor probes agree.

2. State/resume implementation worker.
   Own state builtins/runtime only. Finish schema-backed `VmState` carriers,
   `capture_module`, `capture_call`, `resume`, `resume_frame`, `diff`, and
   `patch` before touching distributed docs. This is still the largest strategic
   disabled cluster.

3. Traits/conversion/testing worker.
   Bundle named impl dispatch, generic trait args, associated-type substitution,
   `From`/`TryFrom`/`Convert`, `AnyError` builder/field access, imported generic
   test helpers, and `Result` method dispatch. These lines block several pages.

4. Container/math carrier worker.
   Implement public `mat(rows, cols, flat_array)`, then choose either statistics
   kinded-carrier migration or optimizer typed-array migration. Keep table query
   DSL and domain simulation previews out of this slice.

5. Native/extension fixture worker.
   Build deterministic loopback/fixture coverage for HTTP, archive create/tar,
   selected process examples, and Python/TypeScript invocation. Keep live network,
   stdin, file watcher, and host env examples manual unless the harness controls
   them.

6. Distributed/snapshot/polyglot proof worker.
   Extend the current matrix with combined TLS, receiver snapshot store,
   remote async fan-in, hash visibility, dynamic runtime refusal, and resume
   cases. This should consume Wave-22B/Wave-22C findings rather than duplicate
   existing rows.

7. Comptime typed-ergonomics worker.
   Move from string/directive authoring toward typed reflection, fragments,
   quasiquote, and hygiene. Use the annotation/host-routing snippets as proof
   targets, but keep external connector/DuckDB examples fixture-gated.

8. DateTime JIT parity worker.
   Narrowly target the three DateTime VM-safe/JIT-failing rows before any broad
   book flip. This is a small, crisp parity lane.

## Files Changed

- `docs/cluster-audits/wave22-disabled-current-triage.md`
