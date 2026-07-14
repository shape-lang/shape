# Wave 27 Disabled Current Triage

Date: 2026-07-09
Role: Wave-27A current disabled-book triage worker

Scope honored:

- Wrote only this report.
- Read the current sibling manifest at
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Read sibling book pages and prior cluster audit docs for static
  classification.
- Did not run cargo, rustc, just, nextest, shape-test, build, or book-truth
  gate commands.
- Did not edit `AGENTS.md`, production code, tests, or sibling book files.

## Current Manifest

| Metric | Count |
|---|---:|
| Generated | `2026-07-09T22:54:33.494Z` |
| Total snippets | 745 |
| Runnable snippets | 538 |
| Disabled snippets | 207 |
| Deferred snippets | 0 |

Wave 25 moved the bounded `state.capture_module()` row to runnable. Wave 26
moved the bounded scalar/string `state.diff` and `state.patch` rows to
runnable. The remaining disabled set is still concentrated in state/resume,
native/extension fixtures, distributed proof surfaces, traits/conversions, and
v0.4 preview APIs.

## Bucket Counts

Each disabled snippet is counted once by first dispatch owner.

| Bucket | Count | Dispatch meaning |
|---|---:|---|
| Active implementation gap | 71 | Runtime/compiler/stdlib behavior is still missing or too narrow for the shown snippet |
| External/manual/fixture/server/env/permission dependent | 70 | Needs loopback server, tempdir, process/stdin, host env, extension runtime, package fixture, or permission harness |
| Proof/design gap | 30 | API/protocol/security/debug design exists in prose but needs a proof matrix or a smaller honest API |
| Preview/out-of-scope | 25 | v0.4 or domain preview surface, not a current book-truth target without a product decision |
| Intentional diagnostic/negative example | 6 | Snippet is meant to fail; keep disabled or move to expected-error truth support |
| Stale-green/count-reduction/book-only candidate | 3 | Likely removable, pattern-matchable, or stale prose once rechecked |
| Old syntax/book rewrite | 2 | Executable fence should become prose or be rewritten to current syntax |
| Total | 207 | |

## Top Disabled Pages

| Page | Disabled | Main owner |
|---|---:|---|
| `stdlib/native/io.mdx` | 12 | Native fixture/manual split |
| `stdlib/core/state.mdx` | 11 | State/resume/content-addressed worker |
| `advanced/ownership-deep-dive.mdx` | 10 | Ownership/CoW plus preview split |
| `advanced/content-addressed-bytecode.mdx` | 9 | State plus distributed proof |
| `stdlib/native/http.mdx` | 9 | Loopback HTTP fixture |
| `advanced/security-permissions.mdx` | 8 | Security/proof API design |
| `advanced/transport-layer.mdx` | 7 | Distributed transport proof |
| `stdlib/core/remote.mdx` | 7 | Remote fixture/proof matrix |
| `fundamentals/error-handling.mdx` | 6 | Conversion and error carrier worker |
| `fundamentals/traits.mdx` | 6 | Trait dispatch/conversion worker |
| `tooling/python-extension.mdx` | 6 | Python extension fixture |
| `advanced/developer-tools.mdx` | 5 | Debug/proof API design |
| `fundamentals/references-borrowing.mdx` | 5 | Ownership/borrow semantics plus one rewrite |
| `tooling/typescript-extension.mdx` | 5 | TypeScript extension fixture |

## Row Classification

| Page | Lines | Count | Bucket | Next action |
|---|---:|---:|---|---|
| `advanced/annotations.mdx` | 73, 89 | 2 | Active implementation gap | Prove/fix expression and await annotation targets against current comptime lowering. |
| `advanced/annotations.mdx` | 480, 508 | 2 | Proof/design gap | Keep disabled until remote/await annotation wrappers have a real proof surface. |
| `advanced/comptime-annotations-cookbook.mdx` | 31 | 1 | External/manual/fixture | Needs a DuckDB/native-C fixture or should stay prose. |
| `advanced/comptime-annotations-cookbook.mdx` | 183, 308, 329 | 3 | Proof/design gap | Await host routing, stacked resilience annotations, and snapshot handoff need proof APIs. |
| `advanced/comptime-llm-patterns.mdx` | 170 | 1 | Active implementation gap | Replace source-string generation with typed fragment/quasiquote or document current source-string boundary. |
| `advanced/comptime.mdx` | 76 | 1 | Intentional diagnostic | Negative comptime-scope example; needs expected-error truth support if executable. |
| `advanced/comptime.mdx` | 266 | 1 | External/manual/fixture | DuckDB native fixture. |
| `advanced/content-addressed-bytecode.mdx` | 154, 168, 226, 264, 541 | 5 | Active implementation gap | Full `VmState`, resume, object/collection serialization, object deltas, caller/locals. |
| `advanced/content-addressed-bytecode.mdx` | 282, 321, 396, 515 | 4 | Proof/design gap | Remote/FaaS/live-migration/transport composition needs a bounded protocol proof. |
| `advanced/developer-tools.mdx` | 86, 137, 238, 320, 462 | 5 | Proof/design gap | Hot reload, time travel, prefetch, and execution proofs need real debug APIs or should be prose. |
| `advanced/module-distribution.mdx` | 563 | 1 | External/manual/fixture | Needs package/module fixture. |
| `advanced/native-c-interop.mdx` | 139, 155, 286 | 3 | External/manual/fixture | DuckDB/Arrow/native pointer fixtures. |
| `advanced/ownership-deep-dive.mdx` | 45, 54, 459, 470, 483 | 5 | Preview/out-of-scope | Ownership class hints, mutex, atomic, and lazy APIs are preview surfaces. |
| `advanced/ownership-deep-dive.mdx` | 81, 259, 399, 425 | 4 | Active implementation gap | Auto move/clone, auto-borrow, borrow-return solver, clone-in-async ownership examples. |
| `advanced/ownership-deep-dive.mdx` | 141 | 1 | Intentional diagnostic | Use-after-move negative example. |
| `advanced/polyglot-distributed.mdx` | 71, 143, 203 | 3 | External/manual/fixture | Requires remote server plus C/Python/snapshot fixtures. |
| `advanced/resumability.mdx` | 21 | 1 | Stale-green/count-reduction | First-pass hash example may become runnable with expected-pattern support; resume claim stays out. |
| `advanced/resumability.mdx` | 105 | 1 | Preview/out-of-scope | Function-level resume remains v0.4 preview in the page itself. |
| `advanced/security-permissions.mdx` | 329, 346, 360, 383, 414, 441, 466, 498 | 8 | Proof/design gap | Permission grants, host limits, and compile/load/run enforcement need an executable security harness. |
| `advanced/transport-layer.mdx` | 79, 213, 282, 347, 367, 401, 436 | 7 | Proof/design gap | TCP/QUIC/memoized transport and FaaS transport require controlled distributed proof rows. |
| `advanced/wire-protocol.mdx` | 90 | 1 | Proof/design gap | Wire compression/serialization needs a protocol proof tied to state carriers. |
| `examples/comptime-codegen.mdx` | 22 | 1 | Stale-green/count-reduction | Prior docs say typed target iteration is now current; recheck and flip or rewrite stale warning. |
| `examples/web-request.mdx` | 22 | 1 | External/manual/fixture | Needs loopback HTTP fixture. |
| `fundamentals/async.mdx` | 123 | 1 | Preview/out-of-scope | Named `join all` result object is not the current homogeneous array surface. |
| `fundamentals/content.mdx` | 51, 61, 107, 453 | 4 | Preview/out-of-scope | Content styling/render adapters and `ContentFor` remain v0.4-style surfaces. |
| `fundamentals/datetime.mdx` | 19, 364, 404 | 3 | Active implementation gap | DateTime JIT/parity and deterministic formatting/parse rows need focused proof. |
| `fundamentals/error-handling.mdx` | 90, 186, 207, 224, 275, 287 | 6 | Active implementation gap | `AnyError`, `From`/`TryFrom`, `Into`/`TryInto`, `!!`, and `?` composition. |
| `fundamentals/functions.mdx` | 229 | 1 | Intentional diagnostic | Bad named-argument negative example. |
| `fundamentals/functions.mdx` | 413 | 1 | External/manual/fixture | Python extension fixture. |
| `fundamentals/modules.mdx` | 80, 191 | 2 | External/manual/fixture | External module/package fixtures. |
| `fundamentals/objects-arrays.mdx` | 37 | 1 | Intentional diagnostic | Out-of-bounds negative example. |
| `fundamentals/objects-arrays.mdx` | 366 | 1 | Active implementation gap | HashMap `keys`/`values`/`entries` carrier methods. |
| `fundamentals/operators.mdx` | 436, 503 | 2 | Active implementation gap | Error-context operator and cast-with-options conversion surface. |
| `fundamentals/references-borrowing.mdx` | 30, 192 | 2 | Intentional diagnostic | Move-after-use and escaping-reference negative examples. |
| `fundamentals/references-borrowing.mdx` | 73, 253 | 2 | Active implementation gap | CoW alias mutation and async owned snapshot borrow rules. |
| `fundamentals/references-borrowing.mdx` | 269 | 1 | Old syntax/book rewrite | Reference/variable grammar sketch should be prose or current syntax. |
| `fundamentals/resource-management.mdx` | 139 | 1 | Stale-green/count-reduction | Definition-only `Drop` trait row is likely prose or expected compile-only. |
| `fundamentals/resource-management.mdx` | 365, 387 | 2 | External/manual/fixture | Async DB/subscription/resource fixtures. |
| `fundamentals/strings.mdx` | 277, 302, 397 | 3 | Preview/out-of-scope | Rich content formatting/string presentation helpers. |
| `fundamentals/tables.mdx` | 56, 76, 109, 125 | 4 | Preview/out-of-scope | Table/query DSL and loaders. |
| `fundamentals/traits.mdx` | 71, 172, 249, 265, 330, 387 | 6 | Active implementation gap | Generic traits, associated types, named impls, conversion traits. |
| `fundamentals/variables.mdx` | 82 | 1 | Active implementation gap | CoW/alias mutation semantics. |
| `fundamentals/variables.mdx` | 168 | 1 | External/manual/fixture | Filesystem helper fixture. |
| `stdlib/core/distributions.mdx` | 49 | 1 | Active implementation gap | Distribution carrier/functions. |
| `stdlib/core/math.mdx` | 70, 86, 102 | 3 | Active implementation gap | Correlation/covariance/percentile carriers. |
| `stdlib/core/monte_carlo.mdx` | 82 | 1 | Active implementation gap | Monte Carlo stats carrier. |
| `stdlib/core/property_testing.mdx` | 19, 32, 49, 77 | 4 | Active implementation gap | Property-testing generators, closures, result schema. |
| `stdlib/core/remote.mdx` | 36, 68, 95, 121, 139, 154, 185 | 7 | External/manual/fixture | Needs controlled remote server plus polyglot/async/negative endpoint fixtures. |
| `stdlib/core/state.mdx` | 163, 190, 220, 238, 331, 398, 419, 453, 476, 506, 533 | 11 | Active implementation gap | `capture_all`, `capture_call`, full resume, `resume_frame`, generic serialization, transport deltas, caller/locals, cache payloads. |
| `stdlib/core/stochastic.mdx` | 30, 47, 64, 80 | 4 | Active implementation gap | Stochastic process functions/carriers. |
| `stdlib/core/testing.mdx` | 44, 59, 88, 103 | 4 | Active implementation gap | Imported generic testing helpers over `Result`. |
| `stdlib/core/transport.mdx` | 56, 88 | 2 | External/manual/fixture | Needs loopback or controlled transport endpoint. |
| `stdlib/domain/finance.mdx` | 16 | 1 | External/manual/fixture | Finance package/module fixture. |
| `stdlib/domain/iot.mdx` | 17, 126 | 2 | Preview/out-of-scope | Domain IoT package preview. |
| `stdlib/domain/physics.mdx` | 20, 81 | 2 | Preview/out-of-scope | Domain physics package preview. |
| `stdlib/domain/simulation.mdx` | 32, 82, 106 | 3 | Preview/out-of-scope | Domain simulation DSL/replay preview. |
| `stdlib/math/interpolation.mdx` | 51 | 1 | Active implementation gap | Mat/interpolation carrier. |
| `stdlib/math/optimize.mdx` | 58, 78 | 2 | Active implementation gap | Optimizer typed-array/options carrier. |
| `stdlib/math/rotation.mdx` | 32, 43 | 2 | Active implementation gap | Rotation matrix/euler carriers. |
| `stdlib/native/archive.mdx` | 40, 79, 93, 102 | 4 | Active implementation gap | Archive create/extract carriers and deterministic fixture wiring. |
| `stdlib/native/env.mdx` | 29, 39, 48, 57 | 4 | External/manual/fixture | Host env/cwd/os/arch values are environment dependent. |
| `stdlib/native/http.mdx` | 39, 50, 64, 73, 82, 100, 109, 118, 136 | 9 | External/manual/fixture | Replace live `api.example.com` calls with loopback fixture before flipping. |
| `stdlib/native/io.mdx` | 212, 222, 249, 279, 296, 308, 340, 348, 520, 538, 557 | 11 | External/manual/fixture | Network, process, stdin/stdout, git, and watcher examples need fixtures or manual classification. |
| `stdlib/native/io.mdx` | 377 | 1 | Active implementation gap | Async file I/O surface remains deferred by prior async audit. |
| `stdlib/native/time.mdx` | 84, 121, 193 | 3 | External/manual/fixture | Polling examples depend on fetch/server/time behavior. |
| `tooling/execution-server.mdx` | 127 | 1 | External/manual/fixture | Requires a running execution server. |
| `tooling/extensions.mdx` | 120 | 1 | External/manual/fixture | DuckDB extension fixture. |
| `tooling/frontmatter.mdx` | 12 | 1 | Old syntax/book rewrite | Frontmatter executable fence should be split from Shape code. |
| `tooling/polyglot.mdx` | 14, 96, 126, 186 | 4 | External/manual/fixture | Python/polyglot runtimes and dependency fixtures. |
| `tooling/python-extension.mdx` | 68, 117, 142, 163, 184, 197 | 6 | External/manual/fixture | Python extension runtime, aiohttp, marshal-error fixture. |
| `tooling/typescript-extension.mdx` | 74, 134, 163, 180, 238 | 5 | External/manual/fixture | TypeScript runtime, fetch, eval/import fixture. |

## Priority Notes

- Typed field mutation: this disabled-book set does not expose many direct
  typed-field mutation rows, but the global proof priority remains real. Wave
  23E identified the next semantic proof bridge: Miri/runtime/JIT probes for
  `TypedObjectStorage::write_slot_in_place`, VM `SetFieldTyped`, option
  carriers, and `jit_typed_object_set_field`.
- Distributed/snapshot/polyglot: already working evidence includes the
  distributed proof matrix row, ordered homogeneous `join all` values, plain-TCP
  `remote::call_async` caller-side cancellation, `capture_module`, and bounded
  scalar/string `Delta`. Still missing for the book: full state resume,
  `capture_call`, generic object/array/map serialization, object/path deltas,
  controlled remote/transport fixtures, TLS async cancellation, and polyglot
  snapshot fixture rows.
- Real async: `Future<T>`, `await`, homogeneous `join all`, local race/scope
  cancellation, and `remote::call_async` are real in the current evidence. The
  disabled rows still need named join-result objects, async file I/O, stream
  protocol decisions, fixture-backed async HTTP/time examples, and TLS-side
  remote cancellation proof.
- Comptime ergonomics/type-safety: TypeRef-first reflection and `set return`
  TypeRef support landed in Wave 23D. Remaining book blockers are expression
  and await annotation target proof, source-level `set param name: (expr)`,
  typed fragments/quasiquote/hygiene, and DuckDB/extension fixtures.
- Global proof gaps: source guards are not semantic proof. The next proof lanes
  should bridge typed field mutation first, then snapshot/wire restore, JIT FFI
  return tags, and trait/object carrier semantics.

## Recommended Next Waves

1. State/resume/content-addressed remainder.
   Own `crates/shape-vm/src/executor/state_builtins/{core,introspection}.rs`,
   `crates/shape-vm/src/executor/{snapshot.rs,resume.rs,vm_state_snapshot.rs}`,
   `crates/shape-runtime/stdlib-src/core/state.shape`, focused state tests, and
   sibling pages `stdlib/core/state.mdx`,
   `advanced/content-addressed-bytecode.mdx`, and
   `advanced/resumability.mdx`. This attacks the highest strategic blocker:
   `capture_all`, `capture_call`, full `resume`, `resume_frame`, caller/locals,
   generic serialization, and object/path deltas. Keep the existing large-file
   debt visible; extract helpers instead of growing state builtins further.

2. Distributed transport and remote fixture/proof matrix.
   Own `crates/shape-vm/src/remote.rs`,
   `crates/shape-vm/src/executor/builtins/remote_builtins.rs`,
   `bin/shape-cli/src/commands/serve_cmd.rs`, focused distributed e2e tests,
   and sibling pages `stdlib/core/{remote,transport}.mdx`,
   `advanced/{transport-layer,wire-protocol,security-permissions}.mdx`, and
   `tooling/execution-server.mdx`. Start with deterministic loopback rows and
   honest negative endpoints; treat QUIC/memoized/FaaS/security examples as
   proof rows until the protocol is executable.

3. Native and extension fixture split.
   Own sibling book pages `stdlib/native/{io,http,env,time,archive}.mdx`,
   `tooling/{python-extension,typescript-extension,polyglot,extensions}.mdx`,
   `advanced/native-c-interop.mdx`, and fixture helpers under
   `bin/shape-cli/tests/support/**` if implementation proof is needed. This is
   the largest count-reduction wave, but only deterministic loopback/tempdir
   rows should join the default gate; live network, stdin, host env, process,
   DuckDB, aiohttp, and TypeScript runtime rows need fixtures or manual status.

4. Traits, conversions, testing, and property testing.
   Own trait/conversion compiler paths under `crates/shape-vm/src/compiler/**`,
   stdlib helpers under `crates/shape-runtime/stdlib-src/core/**`, and focused
   ShapeTest rows for `From`/`TryFrom`, `Into`/`TryInto`, `!!`, named impls,
   associated types, generic assertions, and property-testing schemas. This
   clears shared blockers across error handling, operators, traits, testing,
   and property testing.

5. Comptime annotations and typed generation ergonomics.
   Own `crates/shape-vm/src/compiler/{comptime.rs,comptime_target.rs,comptime_builtins.rs,functions_annotations.rs,statements.rs}`,
   stdlib derives, and focused `tools/shape-test/tests/{comptime,annotations_comptime}/**`.
   Build on the landed TypeRef work; do not overclaim source-string generation
   until typed fragments or a smaller typed directive payload bridge exists.

6. Math, stochastic, DateTime, and domain package carriers.
   Own core math/stochastic/distribution modules, math package modules, DateTime
   JIT/parity tests, and the relevant sibling pages. This is a bounded
   mid-count wave: correlation/covariance/percentile, Monte Carlo stats,
   stochastic paths, rotation/interpolation/optimize carriers, and deterministic
   DateTime parse/format behavior.

7. Semantic proof bridge for typed field mutation and restore.
   Own `crates/shape-value/src/heap_value.rs`,
   `crates/shape-vm/src/executor/typed_object_ops.rs`,
   `crates/shape-jit/src/ffi/typed_object/field_access.rs`,
   `crates/shape-runtime/src/{snapshot.rs,wire_conversion.rs}`,
   `scripts/check-miri-provenance.sh`, and a focused closeout report. This may
   not immediately reduce disabled book count, but it directly answers the
   global proof priority and de-risks typed-object mutation, snapshot/wire
   restore, and JIT FFI field writes.

8. Ownership/CoW/borrow current-semantics split.
   Own ownership lowering/runtime paths and sibling pages
   `advanced/ownership-deep-dive.mdx`,
   `fundamentals/references-borrowing.mdx`, and `fundamentals/variables.mdx`.
   Separate true current implementation gaps from preview APIs and intentional
   diagnostics; convert grammar sketches and non-current ownership classes to
   prose unless an implementation wave is explicitly scheduled.

## Uncertainty

This is a static classification over manifest rows and source snippets. I did
not run disabled snippets, so "stale-green" means plausible count-reduction
candidate, not proven green. Rows involving current implementation details
without direct prior proof, especially DateTime JIT parity, archive carriers,
and HashMap method carriers, should be rechecked in a serialized supervisor
lane before flipping.
