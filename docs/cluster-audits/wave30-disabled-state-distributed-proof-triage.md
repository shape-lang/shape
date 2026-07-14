# Wave-30B Disabled State/Distributed/Proof Triage

Manifest: `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`
generated `2026-07-09T23:40:40.617Z`.

Global manifest expectation from supervisor: 707 total / 541 runnable / 166
disabled / 0 deferred. This report uses `runnable == false` as the disabled
predicate and covers only the state/snapshot/distributed/proof/security/transport
scope. Static inspection only; no cargo/just/nextest/rustc/build/test/book-truth
commands were run.

Scoped disabled rows: 48.

Pages with zero disabled rows in this scope after Wave-29:
`stdlib/core/snapshot.mdx` and `advanced/transport-layer.mdx`.

## Bucket Counts

| Bucket | Count |
|---|---:|
| active implementation gap | 15 |
| external/manual/fixture/server/env/permission dependent | 17 |
| proof/design gap | 1 |
| preview/out-of-scope | 10 |
| intentional diagnostic | 1 |
| stale-green/count-reduction candidate | 2 |
| old syntax/book rewrite | 2 |

## Classification

| Page:line | Snippet id | Bucket | Reason |
|---|---|---|---|
| `advanced/content-addressed-bytecode.mdx:154` | `E__advanced__content-addressed-bytecode__3__L154.shape` | active implementation gap | Presents portable full frame/VM capture. `state.capture` and bounded `capture_all` exist, but full portable all-frame/all-binding state is still not the public truth surface. |
| `advanced/content-addressed-bytecode.mdx:168` | `E__advanced__content-addressed-bytecode__4__L168.shape` | active implementation gap | `state::resume(vm)` from arbitrary captured state still depends on W17 resume payload/frame schema work and cannot be a standalone book truth row. |
| `advanced/content-addressed-bytecode.mdx:226` | `E__advanced__content-addressed-bytecode__6__L226.shape` | active implementation gap | Object/field `state::diff` and `state::patch` remain beyond the Wave-26 scalar/string root-replacement Delta carrier. |
| `advanced/content-addressed-bytecode.mdx:264` | `E__advanced__content-addressed-bytecode__8__L264.shape` | active implementation gap | Hash-addressed storage uses arbitrary-value serialize/deserialize and a store API; current state serialization is bounded and not a general content store surface. |
| `advanced/content-addressed-bytecode.mdx:282` | `E__advanced__content-addressed-bytecode__9__L282.shape` | old syntax/book rewrite | Uses retired `__original__(args)` array forwarding. Current compiler comments/tests expect direct typed forwarding such as `__original__(a, b)`. `capture_call` is still a follow-up after the rewrite. |
| `advanced/content-addressed-bytecode.mdx:321` | `E__advanced__content-addressed-bytecode__10__L321.shape` | preview/out-of-scope | FaaS cluster scheduling policy is conceptual, not a shipped first-party runtime. It also leans on generic `args`/`capture_call` packaging. |
| `advanced/content-addressed-bytecode.mdx:396` | `E__advanced__content-addressed-bytecode__13__L396.shape` | old syntax/book rewrite | Live-migration wrapper still calls `__original__(args)` and names scheduler/coroutine policy objects. Rewrite the forwarding contract before any implementation triage. |
| `advanced/content-addressed-bytecode.mdx:515` | `E__advanced__content-addressed-bytecode__14__L515.shape` | external/manual/fixture/server/env/permission dependent | TCP send/connect examples require a live peer at a real address and a framed transport fixture. |
| `advanced/content-addressed-bytecode.mdx:541` | `E__advanced__content-addressed-bytecode__15__L541.shape` | active implementation gap | Claims full `Array<any>` args and `Map<string, any>` locals. Wave-28 made `caller`, homogeneous `args`, and string-only `locals` honest, but full Any carriers remain open. |
| `advanced/developer-tools.mdx:86` | `E__advanced__developer-tools__0__L86.shape` | active implementation gap | Rust hot-reload machinery exists, but the Shape-level `std::debug`/`HotReloader` user API shown here is not a shipped book-runnable surface. |
| `advanced/developer-tools.mdx:137` | `E__advanced__developer-tools__1__L137.shape` | preview/out-of-scope | Page labels TimeTravel as planned; this is a design-spec Shape API, not a current stdlib export. |
| `advanced/developer-tools.mdx:238` | `E__advanced__developer-tools__2__L238.shape` | preview/out-of-scope | Time-travel workflow depends on planned capture/cursor APIs and example domain functions. |
| `advanced/developer-tools.mdx:320` | `E__advanced__developer-tools__3__L320.shape` | preview/out-of-scope | Blob prefetcher is explicitly planned and lacks a current Shape-facing stdlib API. |
| `advanced/developer-tools.mdx:462` | `E__advanced__developer-tools__4__L462.shape` | proof/design gap | `ExecutionProofBuilder` exists as Rust-side runtime machinery, but proof generation is not integrated as a Shape API or instruction-boundary trace contract. |
| `advanced/module-distribution.mdx:563` | `E__advanced__module-distribution__0__L563.shape` | external/manual/fixture/server/env/permission dependent | Requires a packaged `.shapec`/`shape.toml` dependency fixture and populated module bundle cache. |
| `advanced/polyglot-distributed.mdx:74` | `E__advanced__polyglot-distributed__1__L74.shape` | external/manual/fixture/server/env/permission dependent | Remote `extern C` transfer needs a live `shape serve` receiver with FFI permission posture configured. |
| `advanced/polyglot-distributed.mdx:149` | `E__advanced__polyglot-distributed__2__L149.shape` | external/manual/fixture/server/env/permission dependent | Snapshot-resume across Python calls needs the Python extension, a snapshot store, and a two-process resume harness. |
| `advanced/polyglot-distributed.mdx:212` | `E__advanced__polyglot-distributed__3__L212.shape` | external/manual/fixture/server/env/permission dependent | Combined remote transfer plus receiver snapshot needs a live receiver, FFI allow-list, selected receiver snapshot store, and resume harness. Current tests cover this, but the book gate cannot host it. |
| `advanced/resumability.mdx:21` | `E__advanced__resumability__0__L21.shape` | stale-green/count-reduction candidate | Snapshot/resume is now covered elsewhere, but this row needs a scripted two-run hash/resume harness or conversion to prose; the page still carries stale v0.4-preview framing. |
| `advanced/resumability.mdx:105` | `E__advanced__resumability__1__L105.shape` | stale-green/count-reduction candidate | Function-level snapshot example defines the pattern but does not prove resume in a standalone gate row. Rewrite into a deterministic harness or prose. |
| `advanced/security-permissions.mdx:329` | `E__advanced__security-permissions__7__L329.shape` | preview/out-of-scope | `PermissionGrant`/`ScopeConstraints` construction is host-side conceptual API, not callable Shape source. |
| `advanced/security-permissions.mdx:346` | `E__advanced__security-permissions__8__L346.shape` | preview/out-of-scope | Network scoping grant construction is conceptual host API. |
| `advanced/security-permissions.mdx:360` | `E__advanced__security-permissions__9__L360.shape` | preview/out-of-scope | Resource-scoping grant construction is conceptual host API. |
| `advanced/security-permissions.mdx:383` | `E__advanced__security-permissions__10__L383.shape` | preview/out-of-scope | `compile(...)`, `PermissionSet.readonly()`, and `vm.load_program_with_permissions(...)` are host embedding pseudocode, not current Shape source. |
| `advanced/security-permissions.mdx:414` | `E__advanced__security-permissions__11__L414.shape` | intentional diagnostic | Negative permission example is supposed to fail. It should stay non-runnable or become prose, and the I/O names should be checked in a book-only wave. |
| `advanced/security-permissions.mdx:441` | `E__advanced__security-permissions__12__L441.shape` | external/manual/fixture/server/env/permission dependent | Runtime network denial needs a permission-configured execution context and network fixture. |
| `advanced/security-permissions.mdx:466` | `E__advanced__security-permissions__13__L466.shape` | preview/out-of-scope | `ResourceLimits`/`ResourceUsage` are host-side sandbox control concepts, not a Shape API. |
| `advanced/security-permissions.mdx:498` | `E__advanced__security-permissions__14__L498.shape` | preview/out-of-scope | Three-tier security setup is host embedding pseudocode combining non-Shape APIs. |
| `advanced/wire-protocol.mdx:90` | `E__advanced__wire-protocol__0__L90.shape` | external/manual/fixture/server/env/permission dependent | Compression example requires a live framed transport peer and dataset fixture; transparent compression is not self-proving in a standalone snippet. |
| `stdlib/core/remote.mdx:41` | `B__stdlib__core__remote__1__L41.shape` | external/manual/fixture/server/env/permission dependent | Basic `@remote` call needs a live receiver at `worker:9527`. |
| `stdlib/core/remote.mdx:76` | `B__stdlib__core__remote__2__L76.shape` | external/manual/fixture/server/env/permission dependent | Covered value-type matrix needs a controlled `shape serve` fixture. |
| `stdlib/core/remote.mdx:106` | `B__stdlib__core__remote__3__L106.shape` | external/manual/fixture/server/env/permission dependent | Remote Python example needs receiver extension install/opt-in and live server. The `async fn python` syntax is not a `remote::call_async` proof. |
| `stdlib/core/remote.mdx:134` | `B__stdlib__core__remote__4__L134.shape` | external/manual/fixture/server/env/permission dependent | `remote::execute` needs a live `shape serve` endpoint. |
| `stdlib/core/remote.mdx:154` | `B__stdlib__core__remote__5__L154.shape` | external/manual/fixture/server/env/permission dependent | `remote::ping` needs a live `shape serve` endpoint. |
| `stdlib/core/remote.mdx:171` | `B__stdlib__core__remote__6__L171.shape` | external/manual/fixture/server/env/permission dependent | `remote::call` needs a live endpoint and function-transfer fixture. |
| `stdlib/core/remote.mdx:204` | `B__stdlib__core__remote__7__L204.shape` | external/manual/fixture/server/env/permission dependent | Negative endpoint example needs a reserved unused port; the book gate cannot guarantee `127.0.0.1:1` behavior. |
| `stdlib/core/state.mdx:163` | `B__stdlib__core__state__9__L163.shape` | active implementation gap | Full `capture_all` over all frames and module bindings is broader than the current bounded schema-backed carrier. |
| `stdlib/core/state.mdx:190` | `B__stdlib__core__state__11__L190.shape` | active implementation gap | `state::capture_call` still surfaces a `CallPayload` carrier gap and the example also needs transport. |
| `stdlib/core/state.mdx:220` | `B__stdlib__core__state__12__L220.shape` | active implementation gap | Public `state::resume(vm)` still lacks the full live dispatch/frame-schema resume story for arbitrary captured VM state. |
| `stdlib/core/state.mdx:238` | `B__stdlib__core__state__13__L238.shape` | active implementation gap | `resume_frame` refuses metadata-only `FrameState`; resumable frame payloads need structural call-frame fields. |
| `stdlib/core/state.mdx:331` | `B__stdlib__core__state__19__L331.shape` | active implementation gap | Content-addressed cache row assumes arbitrary-value serialize/deserialize plus a cache API. Current scalar serialization is not that full surface. |
| `stdlib/core/state.mdx:398` | `B__stdlib__core__state__22__L398.shape` | active implementation gap | State synchronization needs object/module deltas; current `diff`/`patch` support homogeneous scalar/string root replacement only. |
| `stdlib/core/state.mdx:481` | `B__stdlib__core__state__26__L481.shape` | active implementation gap | Remote dispatch helper depends on `capture_call`, Any args/results, generic deserialize, spread calls, and transport. |
| `stdlib/core/state.mdx:511` | `B__stdlib__core__state__27__L511.shape` | active implementation gap | Module-state sync needs `ModuleState` diffing, function argument spreading, and live transport; current bounded Delta cannot express it. |
| `stdlib/core/state.mdx:538` | `B__stdlib__core__state__28__L538.shape` | active implementation gap | Function cache needs stable function+argument hash packaging, Any serialize/deserialize, spread invocation, and an external store contract. |
| `stdlib/core/transport.mdx:61` | `B__stdlib__core__transport__3__L61.shape` | external/manual/fixture/server/env/permission dependent | One-shot send needs a live Shape-framed peer and request byte fixture. |
| `stdlib/core/transport.mdx:95` | `B__stdlib__core__transport__4__L95.shape` | external/manual/fixture/server/env/permission dependent | Persistent connection example needs a live peer and deterministic payload/reply fixture. |
| `tooling/execution-server.mdx:130` | `E__tooling__execution-server__0__L130.shape` | external/manual/fixture/server/env/permission dependent | `@remote` call example needs a running execution server at `127.0.0.1:9527`. |

## Priority Lanes

1. State carriers and resume completeness.
   Own likely files: `crates/shape-vm/src/executor/state_builtins/{core,introspection}.rs`,
   `crates/shape-vm/src/executor/resume.rs`,
   `crates/shape-vm/src/executor/state_builtins_tests.rs`,
   `crates/shape-runtime/stdlib-src/core/state.shape`.
   Focus: real `capture_call`, public `state.resume`, resumable `FrameState`,
   full `VmState` frame data, and honest Any carriers.

2. State diff/patch and generalized serialization.
   Own likely files: `crates/shape-vm/src/executor/state_builtins/core.rs`
   plus focused state builtin tests. Focus: object/array/map/path deltas,
   `ModuleState` deltas, and deserialize projection beyond scalar/string/bool.

3. Remote/transport fixture proof lane.
   Own likely files/tests: `crates/shape-vm/src/executor/builtins/remote_builtins.rs`,
   `crates/shape-vm/src/executor/builtins/transport_builtins.rs`,
   `bin/shape-cli/src/commands/serve_cmd.rs`,
   `bin/shape-cli/tests/distributed_*`.
   Most remote/transport snippets are not implementation gaps; they need a
   controlled live-server loopback fixture or should stay disabled.

4. Distributed snapshot/polyglot composition proof lane.
   Own likely tests: `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs`,
   `bin/shape-cli/tests/distributed_dynamic_snapshot_e2e.rs`,
   `bin/shape-cli/tests/distributed_extern_c_snapshot_e2e.rs`,
   `bin/shape-cli/tests/distributed_proof_matrix_e2e.rs`, and
   `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`.
   Blockers are fixtures: extension `.so` availability, receiver FFI opt-in,
   receiver snapshot-store selection, and two-process resume.

5. Security/proof public surface.
   Own likely files: `crates/shape-abi-v1/src/lib.rs`,
   `crates/shape-runtime/src/project/permissions.rs`,
   `crates/shape-runtime/src/module_exports.rs`,
   `bin/shape-cli/src/commands/serve_cmd.rs`.
   The Rust/CLI security envelope is real, but most page rows are host API
   pseudocode. Decide whether to expose a Shape API or convert those fences to
   prose.

6. Debug/proof APIs.
   Own likely files: `crates/shape-vm/src/hot_reload.rs`,
   `crates/shape-vm/src/executor/time_travel.rs`,
   `crates/shape-runtime/src/execution_proof.rs`.
   Needed work: a real `std::debug` Shape module/bindings and runtime trace
   integration for proof generation.

## Book-Only Candidates

- `advanced/resumability.mdx:21` and `:105`: stale v0.4-preview framing should
  be corrected. Either add a scripted resume-roundtrip fixture or convert the
  dynamic hash/resume flow to prose.
- `advanced/content-addressed-bytecode.mdx:282` and `:396`: replace retired
  `__original__(args)` examples with direct typed forwarding or convert the
  policy-wrapper examples to prose until `capture_call` is real.
- `advanced/content-addressed-bytecode.mdx:541`: split into current bounded
  runnable examples (`caller`, homogeneous `args`, string-only `locals`) and
  prose for the future Any-carrier shape.
- `advanced/security-permissions.mdx:414`: keep as text/diagnostic, and verify
  the I/O function names against the current stdlib before any future runnable
  row.
- `advanced/developer-tools.mdx` planned `std::debug` examples should be prose
  unless a Shape-facing debug module is shipped.

## Explicit Distributed/Snapshot/Async Notes

- The three `advanced/polyglot-distributed.mdx` disabled cells are fixture-bound,
  not current implementation blockers: remote transfer (`:74`), snapshot-resume
  across foreign calls (`:149`), and combined transfer plus receiver snapshot
  (`:212`) all require live receiver/runtime/snapshot-store setup.
- `advanced/module-distribution.mdx:563` is package/bundle-fixture work, not
  runtime remote execution.
- `advanced/wire-protocol.mdx:90` and the transport rows require live framed
  peers; they should remain disabled unless the book gate grows a loopback
  transport fixture.
- Real async is only adjacent in this scoped manifest. `remote.mdx:106` is a
  remote foreign-function fixture, not a `remote::call_async` proof. Current
  async blockers to remember are distributed cancellation/TLS path coverage and
  the intentional snapshot barrier for live remote futures; relevant tests live
  in `bin/shape-cli/tests/distributed_async_e2e.rs` and
  `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`.
