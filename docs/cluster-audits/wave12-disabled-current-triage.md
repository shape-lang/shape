# Wave 12 Disabled Current Book Triage

Date: 2026-07-09
Worker: Wave-12A current disabled-book manifest triage

## Scope And Method

Authoritative manifest:
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`
generated at `2026-07-09T12:32:00.821Z`.

Current manifest totals:

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 500 |
| Disabled snippets | 256 |

This is a source/manifest triage only. I did not run cargo, just, rustc,
nextest, shape-test, book-truth, the Shape binary, or build/test commands. I
used the current manifest, the Wave-6/Wave-8 triage notes, current sibling book
pages, and current source/test files for classification evidence.

Classification key:

| Bucket | Meaning |
|---|---|
| `stale/flip candidate` | Implementation likely exists; next worker should rewrite into a deterministic standalone snippet and verify in the serialized lane before flipping. |
| `active feature gap` | Current code or page comments show a real implementation blocker or unsupported user surface. |
| `external/manual` | Needs live files, network, stdin/processes, remote servers, package fixtures, extension builds, native libraries, permissions, or other manual environment setup. |
| `old syntax/policy rewrite` | Snippet teaches retired syntax, internal APIs, stale error/result policy, or pseudo-signatures that should be rewritten rather than implemented. |
| `design/proof gap` | Pieces may exist, but the composed user story still needs a deterministic proof harness, fixture, or security/distributed/snapshot proof. |
| `preview/out-of-scope` | Conceptual host API, intentional negative diagnostic, v0.4 preview, comment fragment, or non-runnable design sketch. |

## Headline Counts

| Classification | Count |
|---|---:|
| `active feature gap` | 75 |
| `external/manual` | 65 |
| `stale/flip candidate` | 35 |
| `preview/out-of-scope` | 33 |
| `design/proof gap` | 30 |
| `old syntax/policy rewrite` | 18 |
| **Total** | **256** |

The disabled set has moved materially since Wave-6/Wave-8. Resource-management
and annotation/module stale examples were mostly flipped; `state.hash`,
`state.fn_hash`, `state.schema_hash`, scalar `state.serialize` /
`state.deserialize`, local snapshot store selection, `remote::call_async`, and
dynamic Python/TypeScript remote snapshot/resume now have source/test evidence.
The remaining disabled set is therefore less "old book drift" and more
implementation/proof/environment debt.

## Top Disabled Pages

| Page | Disabled | stale | active | external | old | design | preview | Current read |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| `stdlib/native/io.mdx` | 25 | 0 | 1 | 23 | 0 | 0 | 1 | Mostly filesystem/network/process/stdin; `read_file_async` remains active. |
| `stdlib/core/state.mdx` | 16 | 0 | 11 | 0 | 0 | 5 | 0 | Capture/resume/diff/introspection still blocked; scalar hash/serde examples already left disabled. |
| `advanced/ownership-deep-dive.mdx` | 10 | 5 | 1 | 0 | 1 | 0 | 3 | Atomic/Lazy/ref examples are close; explicit storage classes are not. |
| `fundamentals/content.mdx` | 10 | 0 | 2 | 0 | 2 | 0 | 6 | Fragments hit an array element proof gap; `c"..."` and adapters are preview/retired. |
| `advanced/content-addressed-bytecode.mdx` | 9 | 0 | 4 | 0 | 1 | 3 | 1 | State capture/resume/diff plus transport/store composition. |
| `advanced/security-permissions.mdx` | 9 | 0 | 0 | 0 | 1 | 4 | 4 | Host/security proof sketches, not user Shape snippets. |
| `stdlib/native/http.mdx` | 9 | 0 | 0 | 9 | 0 | 0 | 0 | Live network examples plus stale `await` shape. |
| `advanced/transport-layer.mdx` | 7 | 0 | 1 | 0 | 1 | 4 | 1 | TCP/memoized transport needs loopback proof; QUIC remains gated. |
| `stdlib/core/remote.mdx` | 7 | 0 | 1 | 2 | 4 | 0 | 0 | `@remote` Result wording and `remote::__call` are stale; execute/ping need serve fixtures. |
| `fundamentals/datetime.mdx` | 6 | 1 | 5 | 0 | 0 | 0 | 0 | Page states VM-safe but JIT-disabled DateTime methods. |
| `fundamentals/error-handling.mdx` | 6 | 2 | 3 | 0 | 0 | 0 | 1 | `!!` stubs are close; Convert/TryFrom remains active. |
| `fundamentals/traits.mdx` | 6 | 0 | 5 | 0 | 0 | 0 | 1 | Named impl dispatch, generic trait args, conversions, associated types. |
| `tooling/python-extension.mdx` | 6 | 0 | 3 | 1 | 1 | 1 | 0 | Scalar extension is environment-gated; async and typed-object return need work/proof. |

## All Page Matrix

| Page | Disabled | stale | active | external | old | design | preview |
|---|---:|---:|---:|---:|---:|---:|---:|
| `advanced/annotations.mdx` | 4 | 0 | 2 | 1 | 0 | 0 | 1 |
| `advanced/comptime-annotations-cookbook.mdx` | 4 | 0 | 2 | 0 | 0 | 1 | 1 |
| `advanced/comptime-llm-patterns.mdx` | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| `advanced/comptime.mdx` | 2 | 0 | 1 | 0 | 0 | 0 | 1 |
| `advanced/content-addressed-bytecode.mdx` | 9 | 0 | 4 | 0 | 1 | 3 | 1 |
| `advanced/developer-tools.mdx` | 5 | 0 | 0 | 0 | 0 | 3 | 2 |
| `advanced/module-distribution.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `advanced/native-c-interop.mdx` | 3 | 0 | 0 | 3 | 0 | 0 | 0 |
| `advanced/ownership-deep-dive.mdx` | 10 | 5 | 1 | 0 | 1 | 0 | 3 |
| `advanced/polyglot-distributed.mdx` | 3 | 0 | 0 | 0 | 0 | 3 | 0 |
| `advanced/resumability.mdx` | 2 | 0 | 1 | 0 | 0 | 1 | 0 |
| `advanced/security-permissions.mdx` | 9 | 0 | 0 | 0 | 1 | 4 | 4 |
| `advanced/transport-layer.mdx` | 7 | 0 | 1 | 0 | 1 | 4 | 1 |
| `advanced/wire-protocol.mdx` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `examples/comptime-codegen.mdx` | 1 | 0 | 0 | 0 | 1 | 0 | 0 |
| `examples/web-request.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `fundamentals/async.mdx` | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| `fundamentals/content.mdx` | 10 | 0 | 2 | 0 | 2 | 0 | 6 |
| `fundamentals/datetime.mdx` | 6 | 1 | 5 | 0 | 0 | 0 | 0 |
| `fundamentals/enums.mdx` | 1 | 0 | 0 | 0 | 0 | 0 | 1 |
| `fundamentals/error-handling.mdx` | 6 | 2 | 3 | 0 | 0 | 0 | 1 |
| `fundamentals/functions.mdx` | 3 | 1 | 0 | 1 | 0 | 0 | 1 |
| `fundamentals/modules.mdx` | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| `fundamentals/objects-arrays.mdx` | 2 | 0 | 1 | 0 | 0 | 0 | 1 |
| `fundamentals/operators.mdx` | 2 | 1 | 1 | 0 | 0 | 0 | 0 |
| `fundamentals/references-borrowing.mdx` | 5 | 0 | 1 | 0 | 1 | 0 | 3 |
| `fundamentals/resource-management.mdx` | 3 | 0 | 2 | 0 | 0 | 0 | 1 |
| `fundamentals/strings.mdx` | 3 | 0 | 2 | 0 | 1 | 0 | 0 |
| `fundamentals/tables.mdx` | 5 | 1 | 2 | 1 | 1 | 0 | 0 |
| `fundamentals/traits.mdx` | 6 | 0 | 5 | 0 | 0 | 0 | 1 |
| `fundamentals/variables.mdx` | 2 | 0 | 1 | 0 | 1 | 0 | 0 |
| `getting-started/basic-concepts.mdx` | 1 | 0 | 0 | 0 | 0 | 0 | 1 |
| `stdlib/core/distributions.mdx` | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/core/math.mdx` | 3 | 3 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/core/monte_carlo.mdx` | 2 | 2 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/core/ode.mdx` | 3 | 3 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/core/property_testing.mdx` | 4 | 0 | 4 | 0 | 0 | 0 | 0 |
| `stdlib/core/remote.mdx` | 7 | 0 | 1 | 2 | 4 | 0 | 0 |
| `stdlib/core/state.mdx` | 16 | 0 | 11 | 0 | 0 | 5 | 0 |
| `stdlib/core/stochastic.mdx` | 4 | 4 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/core/testing.mdx` | 4 | 0 | 4 | 0 | 0 | 0 | 0 |
| `stdlib/core/transport.mdx` | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `stdlib/domain/finance.mdx` | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| `stdlib/domain/iot.mdx` | 2 | 0 | 2 | 0 | 0 | 0 | 0 |
| `stdlib/domain/physics.mdx` | 2 | 2 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/domain/simulation.mdx` | 3 | 0 | 3 | 0 | 0 | 0 | 0 |
| `stdlib/math/interpolation.mdx` | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/math/optimize.mdx` | 3 | 3 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/math/rotation.mdx` | 3 | 1 | 2 | 0 | 0 | 0 | 0 |
| `stdlib/native/archive.mdx` | 5 | 2 | 3 | 0 | 0 | 0 | 0 |
| `stdlib/native/csv.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `stdlib/native/env.mdx` | 4 | 0 | 0 | 4 | 0 | 0 | 0 |
| `stdlib/native/file.mdx` | 5 | 0 | 0 | 5 | 0 | 0 | 0 |
| `stdlib/native/http.mdx` | 9 | 0 | 0 | 9 | 0 | 0 | 0 |
| `stdlib/native/io.mdx` | 25 | 0 | 1 | 23 | 0 | 0 | 1 |
| `stdlib/native/json.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `stdlib/native/math.mdx` | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| `stdlib/native/time.mdx` | 3 | 0 | 0 | 3 | 0 | 0 | 0 |
| `tooling/docstrings.mdx` | 2 | 0 | 0 | 0 | 0 | 0 | 2 |
| `tooling/execution-server.mdx` | 1 | 0 | 0 | 0 | 1 | 0 | 0 |
| `tooling/extensions.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `tooling/frontmatter.mdx` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `tooling/polyglot.mdx` | 4 | 0 | 1 | 2 | 0 | 1 | 0 |
| `tooling/python-extension.mdx` | 6 | 0 | 3 | 1 | 1 | 1 | 0 |
| `tooling/typescript-extension.mdx` | 5 | 0 | 1 | 2 | 1 | 1 | 0 |

## Immediately Flippable Candidates

The best near-term book-truth gain is the 35-snippet `stale/flip candidate`
pool. The cleanest first wave is the deterministic numeric/domain subset:

- `stdlib/core/math.mdx` L74/L87/L100, `stdlib/core/stochastic.mdx`
  L34/L46/L58/L69, `stdlib/core/distributions.mdx` L50,
  `stdlib/core/monte_carlo.mdx` L41/L84, `stdlib/core/ode.mdx`
  L34/L58/L80.
- `stdlib/math/interpolation.mdx` L48, `stdlib/math/optimize.mdx`
  L19/L59/L76, `stdlib/math/rotation.mdx` L72, `stdlib/native/math.mdx`
  L65, and `stdlib/domain/physics.mdx` L17/L75.

These 21 snippets should need imports, literal fixtures, stable predicates, or
bounded output checks rather than production-code changes.

Other small flip candidates:

- `fundamentals/error-handling.mdx` L275/L287: add tiny `Result` stubs for
  `find_user`, `value`, and `other_call`; keep Convert/TryFrom examples
  disabled.
- `fundamentals/operators.mdx` L503, `fundamentals/functions.mdx` L185, and
  `fundamentals/tables.mdx` L31: add local scaffolding and verify the exact
  runtime path.
- `advanced/comptime-llm-patterns.mdx` L170: wrap the current source-string
  `extend` pattern into a self-contained comptime smoke.
- `advanced/ownership-deep-dive.mdx` L470/L483: Atomic and Lazy methods look
  closest; return-reference and async-boundary snippets need splitting because
  they mix positive and negative examples.
- `stdlib/native/archive.mdx` L49/L73 can flip only with embedded deterministic
  zip/tar fixture bytes. `zip_create` / `tar_create` are still not registered
  in `crates/shape-runtime/src/stdlib/archive.rs`, so creation and roundtrip
  examples remain implementation work.

## Implementation-Required Lanes

1. State/resume/content-addressed completion.
   `stdlib/core/state.mdx` carries 11 active gaps and 5 proof gaps. The active
   work is `capture*`, `capture_call`, public `state::resume`,
   `resume_frame`, `diff`, `patch`, `caller`, `args`, and `locals`. The
   content-addressed, wire-protocol, transport, and resumability pages then
   need a deterministic proof harness over the completed state surface.

2. Trait/Convert/testing/property lane.
   `fundamentals/traits.mdx`, `fundamentals/error-handling.mdx`,
   `stdlib/core/testing.mdx`, and `stdlib/core/property_testing.mdx` are the
   main cluster. Convert opcode trait dispatch, target-side `From`/`TryFrom`,
   named impl dispatch, associated-type substitution, imported generic
   assertions, Result assertion methods, and property function-field schemas
   should be closed before further doc flips.

3. Async and annotation lane.
   `remote::call_async` itself is implemented, but disabled snippets still
   need named `join all` materialization, expression/await annotation target
   handling, async resource/drop semantics, and async Python/TypeScript
   foreign runtime support. The current expression-annotation regression test
   still expects an `op_new_array(0)` surface at the annotation args boundary.

4. Content/DateTime/container proof lane.
   `Content.fragment([...])` is implemented but blocked by array element type
   proof for separately-built content nodes. DateTime examples are VM-safe per
   the page but disabled until JIT DateTime method dispatch is fixed. HashMap
   `keys`/`values`/`entries`, table formatting f-strings, rotation matrix
   construction, and table method chaining share container/materialization
   proof surfaces.

5. Transport/security/distributed proof lane.
   `remote::execute`, `remote::ping`, `remote::call`, `remote::call_async`,
   extern-C remote transfer, and dynamic Python/TypeScript receiver snapshot
   resume now have source/test evidence. The disabled docs still need loopback
   transport fixtures, permission derivation/refusal proofs, host/security API
   separation, and extension-aware distributed book fixtures. Wave-12B already
   owns the TLS user remote-call surface; do not overlap that lane.

6. External/manual rewrite lane.
   The 65 external/manual snippets are dominated by native IO, HTTP, file/env,
   live remote servers, Python/TypeScript/DuckDB extensions, package fixtures,
   and frontmatter script context. Do not flip these into the default book gate
   unless each is rewritten around inline data, a controlled fixture, or a
   separate opt-in integration lane.

## User Priority Mapping

Typed field mutation:

- No current disabled book snippet directly exercises the typed
  `Option<T>` field-mutation path that was closed earlier. Current proof docs
  record that `SetFieldTyped` for typed-object `Option<T>` field mutation is
  covered by schema metadata and canonical `__Option.Some/None` validation.
- Adjacent risks remain in state `resume_frame` typed-object field decode,
  state capture typed-object return projection, Python/TypeScript typed-object
  returns, `Content.fragment` array typing, and rotation/Mat construction.

Distributed/snapshot/polyglot:

- This is still the highest-value proof cluster. The implementation moved
  forward: `remote::call_async` has e2e source; local snapshot hash/resume and
  receiver snapshot hash/resume have e2e source; dynamic Python/TypeScript
  receiver snapshot/resume has source gated on built extensions.
- Remaining disabled snippets mostly need harnesses and proof boundaries:
  transport loopback, extension fixture availability, permission/TLS refusal,
  content-addressed state composition, and state capture/diff/introspection.

Async:

- `Future<T>` handles and `remote::call_async` are no longer the main gap.
  Remaining async-disabled snippets are named `join all` result objects, async
  foreign functions, `await_expr` policy annotations, and async resource/drop
  examples with streams or fake DB APIs.

Comptime type-safety and ergonomics:

- The source-string `extend (f"...")` LLM pattern is flip-close, but should
  not become the long-term ergonomic endpoint.
- DuckDB schema inference, `set param` public metadata, generated typed return
  contracts, `replace module` re-verification, and typed code fragments /
  quasiquote are still the real comptime implementation lane. Await-routing
  annotations depend on the async lane.

Global proof gaps:

- Source-only typed-opcode proof coverage currently records
  `unproven_gap = 0`, and the ignored-test taxonomy has zero accepted active
  or stale source ignores after the Wave-9B refresh.
- Those are bounded guards, not global semantic proofs. The current disabled
  book set still exposes proof gaps around distributed/FFI/snapshot behavior,
  transport/cache behavior, permissions/resource limits, JIT DateTime, state
  typed-object marshal/return, and targeted-not-global Miri coverage.

## Recommended Next Waves

1. Core numeric/domain stale-flip wave.
   Own only the 21 deterministic stale snippets listed above. Use stable
   predicates, literal inputs, and bounded output; avoid raw random output.

2. State/resume implementation wave.
   Close `capture*`, `capture_call`, `resume`, `resume_frame`, `diff`, `patch`,
   and introspection return projection before trying to flip state or
   content-addressed distributed examples.

3. Trait/Convert/testing/property wave.
   Close Convert trait dispatch and the generic assertion/property-testing
   blockers. This retires many active language-surface snippets without
   depending on external systems.

4. Async/annotation/foreign wave.
   Materialize named `join all` results, close expression/await annotation args
   handling, then decide whether async Python/TypeScript belongs in the default
   gate or an extension-only integration lane.

5. Transport/security/distributed proof wave.
   After Wave-12B's TLS surface, add deterministic loopback transport and
   permission/refusal fixtures, then revisit `remote`, `transport-layer`,
   `polyglot-distributed`, `wire-protocol`, and execution-server snippets.

6. External/manual rewrite wave.
   Rewrite native IO/HTTP/file/env examples around inline data or controlled
   fixtures. Keep live network, stdin, subprocess, local package, DuckDB, and
   extension-install examples disabled unless an opt-in gate owns them.

## Source Anchors

- Current manifest:
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Prior triage baselines:
  `docs/cluster-audits/wave6-disabled-language-surface-triage.md`,
  `docs/cluster-audits/wave6-disabled-native-state-triage.md`,
  `docs/cluster-audits/wave6-disabled-distributed-comptime-proof-triage.md`,
  `docs/cluster-audits/wave8-disabled-remaining-book-triage.md`.
- Remote current surface:
  `crates/shape-runtime/stdlib-src/core/remote.shape`,
  `crates/shape-vm/src/executor/builtins/remote_builtins.rs`,
  `crates/shape-vm/src/compiler/expressions/function_calls.rs`,
  `bin/shape-cli/tests/distributed_async_e2e.rs`,
  `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs`,
  `bin/shape-cli/tests/distributed_dynamic_snapshot_e2e.rs`.
- State current surface:
  `crates/shape-runtime/stdlib-src/core/state.shape`,
  `crates/shape-vm/src/executor/state_builtins/core.rs`,
  `crates/shape-vm/src/executor/state_builtins/introspection.rs`,
  `crates/shape-vm/src/executor/state_builtins_tests.rs`.
- Book-side explicit blockers:
  sibling `fundamentals/datetime.mdx` JIT warning and
  `fundamentals/content.mdx` `Content.fragment` array proof warning.
- Archive registration:
  `crates/shape-runtime/src/stdlib/archive.rs` registers extract functions only;
  `crates/shape-runtime/stdlib-src/core/archive.shape` still declares create
  functions.
- Proof baselines:
  `docs/cluster-audits/w91a-typed-opcode-proof-coverage.md` and
  `docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md`.
