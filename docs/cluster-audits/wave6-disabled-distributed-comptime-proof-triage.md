# Wave 6 Disabled Distributed/Comptime/Proof Triage

Date: 2026-07-09
Worker: read-only/docs triage
Manifest: `/tmp/shape-async-snippets/manifest.json`

## Scope And Method

Fresh extraction:

- Generated at: `2026-07-09T09:52:40.362Z`
- Total snippets: 756
- Runnable snippets: 462
- Disabled snippets: 294

This report covers 66 disabled snippets:

- 54 snippets on the requested pages:
  - `stdlib/core/remote.mdx`
  - `advanced/polyglot-distributed.mdx`
  - `advanced/transport-layer.mdx`
  - `advanced/content-addressed-bytecode.mdx`
  - `advanced/developer-tools.mdx`
  - `advanced/comptime.mdx`
  - `advanced/comptime-annotations-cookbook.mdx`
  - `tooling/polyglot.mdx`
  - `tooling/python-extension.mdx`
  - `tooling/typescript-extension.mdx`
- 12 adjacent snippets matching the prompt's "any disabled examples tied to" clause:
  - `stdlib/core/snapshot.mdx`
  - `advanced/security-permissions.mdx`
  - `advanced/comptime-llm-patterns.mdx`
  - `examples/comptime-codegen.mdx`

No book snippets, production code, `AGENTS.md`, cargo, shape-test, book-truth,
or build/test commands were run or edited.

## Classification Totals

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 9 |
| `active_feature_gap` | 13 |
| `design_or_proof_gap` | 21 |
| `old_syntax_or_policy_rewrite` | 10 |
| `preview_or_out_of_scope` | 13 |
| Total covered | 66 |

## Source Evidence Summary

- Public remote shape: `remote::execute`, `remote::ping`, `@remote`, and the
  compiler-recognized public `remote::call` are live surfaces. `remote::__call`
  is explicitly retired from the user surface; current internals are
  `__call_raising` and `__call_result`
  (`crates/shape-runtime/stdlib-src/core/remote.shape:66`,
  `crates/shape-vm/src/executor/builtins/remote_builtins.rs:1053`).
- `remote::call` is compiler-elaborated, positional, typed, and lowered to
  `__call_result` (`crates/shape-vm/src/compiler/expressions/function_calls.rs:5711`,
  `:6130`).
- CLI e2e source covers local snapshot hash/resume, `remote::execute`,
  `remote::call` Ok/Err, int-kind preservation, closure capture, remote
  receiver snapshot hash, and extern-C transfer
  (`bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:8`,
  `:56`, `:80`, `:119`, `:151`, `:175`, `:199`, `:225`).
- Python/TypeScript remote-transfer tests exist in two forms: active
  subprocess tests that skip cleanly without built extensions
  (`bin/shape-cli/src/commands/serve_cmd.rs:2431`, `:2547`), and tracker
  ignored tests in the standalone e2e file for the observed opt-in residual
  (`bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:261`, `:267`).
- `std::core::state` is discoverable, but many bodies still surface W17
  snapshot/resume/marshal-return gaps. `state.hash` is real; `capture*`,
  `capture_call`, `serialize`, `deserialize`, `resume`, `caller`, `args`, and
  `locals` remain partial or marshal-blocked
  (`crates/shape-vm/src/executor/state_builtins/core.rs:39`, `:223`, `:277`,
  `:358`, `:438`, `:587`;
  `crates/shape-vm/src/executor/state_builtins/introspection.rs:1`).
- `std::core::transport` native exports include `tcp`, `send`, `connect`,
  connection operations, `memoized`, `memo_stats`, and `memo_invalidate`, but
  current tests explicitly say live TCP/memoized behavior tests are still
  Phase-2c typed-module-export work
  (`crates/shape-vm/src/executor/builtins/transport_builtins.rs:153`,
  `:214`, `:474`;
  `crates/shape-vm/src/executor/builtins/transport_builtins_tests.rs:18`).
- Comptime generated free functions, generated methods, `type_info`, and LSDS
  diagnostics have VM/JIT flagship tests. `set param` public default metadata
  and generated `replace module` re-verification remain explicit ignored active
  gaps (`tools/shape-test/tests/comptime/flagship_wf3d.rs:1`;
  `tools/shape-test/tests/annotations_comptime/directives.rs:8`, `:130`).
- Proof guard status is source-only: ignored-test source taxonomy currently has
  zero active/stale accepted gaps, Miri coverage is targeted not global, and the
  typed-opcode proof checker records `unproven_gap = 0`
  (`docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:72`,
  `:144`;
  `docs/cluster-audits/w91a-typed-opcode-proof-coverage.md:23`).

## Page Triage

### `stdlib/core/remote.mdx`

Disabled count: 7

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 2 |
| `active_feature_gap` | 1 |
| `old_syntax_or_policy_rewrite` | 4 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `old_syntax_or_policy_rewrite` | L36 `B__stdlib__core__remote__1__L36.shape`; L68 `B__stdlib__core__remote__2__L68.shape`; L185 `B__stdlib__core__remote__7__L185.shape` | These teach `@remote` as returning `Ok(...)`/`Err(...)`. Current `@remote` uses raising semantics; recoverable transport errors belong on public `remote::call`. Rewrite before any flip. |
| `active_feature_gap` | L95 `B__stdlib__core__remote__3__L95.shape` | Remote async Python matrix example combines async foreign execution, distributed transfer, and external NumPy/GPU-worker assumptions. Treat as a feature lane, not a stale disabled. |
| `stale_disabled_candidate` | L121 `B__stdlib__core__remote__4__L121.shape`; L139 `B__stdlib__core__remote__5__L139.shape` | `remote::execute` and `remote::ping` are live exports. `remote::execute` has CLI e2e coverage; `ping` should get a focused serve smoke before flipping. |
| `old_syntax_or_policy_rewrite` | L154 `B__stdlib__core__remote__6__L154.shape` | `remote::__call` is retired user surface. Rewrite to `remote::call(addr, fn, args...)`. |

### `stdlib/core/snapshot.mdx`

Disabled count: 1

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `stale_disabled_candidate` | L44 `B__stdlib__core__snapshot__2__L44.shape` | Local `snapshot()` returns `Result<Snapshot, SnapshotError>` and has CLI hash/resume e2e coverage. Next worker should smoke this exact snippet form under a snapshot-aware harness before flipping. |

### `advanced/polyglot-distributed.mdx`

Disabled count: 3

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 1 |
| `design_or_proof_gap` | 2 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `design_or_proof_gap` | L73 `E__advanced__polyglot-distributed__1__L73.shape` | Python `@remote` transfer likely exists when extensions are built, but proof is extension-gated and split between skip-clean subprocess tests and ignored opt-in tracker tests. Needs one authoritative extension-lane smoke. |
| `design_or_proof_gap` | L149 `E__advanced__polyglot-distributed__2__L149.shape` | Snapshot after a Python call is not the same as proving resumability with extension manifests, foreign-frame barriers, and cross-node requirements. |
| `stale_disabled_candidate` | L208 `E__advanced__polyglot-distributed__3__L208.shape` | The snippet's comment says receiver-side `snapshot()` is a clean barrier today; current e2e source expects a receiver hash. Rewrite expected `snapstate` and smoke. |

### `advanced/transport-layer.mdx`

Disabled count: 7

Counts:

| Classification | Count |
|---|---:|
| `active_feature_gap` | 1 |
| `design_or_proof_gap` | 4 |
| `old_syntax_or_policy_rewrite` | 1 |
| `preview_or_out_of_scope` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `design_or_proof_gap` | L79 `E__advanced__transport-layer__0__L79.shape`; L401 `E__advanced__transport-layer__5__L401.shape` | `state::capture_call` and transport composition are not established as a runnable content-addressed call layer. `capture_call` currently surfaces at marshal-return. |
| `design_or_proof_gap` | L213 `E__advanced__transport-layer__1__L213.shape`; L347 `E__advanced__transport-layer__3__L347.shape` | Native TCP/memoized exports exist, but live framing/cache behavior tests are explicitly deferred. |
| `active_feature_gap` | L282 `E__advanced__transport-layer__2__L282.shape` | QUIC is feature-gated and not established as a default user-facing snippet surface. |
| `old_syntax_or_policy_rewrite` | L367 `E__advanced__transport-layer__4__L367.shape` | This is an output/tuple fragment, not standalone Shape. Replace with a real `memo_stats` inspection example. |
| `preview_or_out_of_scope` | L436 `E__advanced__transport-layer__6__L436.shape` | `@faas(cluster_with_memo_transport)` is a policy framework example, not current release surface. |

### `advanced/content-addressed-bytecode.mdx`

Disabled count: 11

Counts:

| Classification | Count |
|---|---:|
| `active_feature_gap` | 3 |
| `design_or_proof_gap` | 4 |
| `old_syntax_or_policy_rewrite` | 1 |
| `preview_or_out_of_scope` | 3 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `design_or_proof_gap` | L154 `E__advanced__content-addressed-bytecode__3__L154.shape` | `capture`/`capture_all` read live VM state internally but still surface at typed-object marshal-return. |
| `active_feature_gap` | L168 `E__advanced__content-addressed-bytecode__4__L168.shape` | `state::resume(vm)` is registered but not a complete public resume primitive. |
| `design_or_proof_gap` | L202 `E__advanced__content-addressed-bytecode__5__L202.shape` | `state.hash` is real, while `fn_hash`/`schema_hash` have partial content-metadata constraints. Needs split examples. |
| `active_feature_gap` | L227 `E__advanced__content-addressed-bytecode__6__L227.shape`; L255 `E__advanced__content-addressed-bytecode__7__L255.shape` | `diff`/`patch`, `serialize`, and `deserialize` are W17 residuals. |
| `preview_or_out_of_scope` | L264 `E__advanced__content-addressed-bytecode__8__L264.shape`; L321 `E__advanced__content-addressed-bytecode__10__L321.shape`; L396 `E__advanced__content-addressed-bytecode__13__L396.shape` | Store API, FaaS scheduler, and migratable scheduler examples are conceptual. |
| `old_syntax_or_policy_rewrite` | L282 `E__advanced__content-addressed-bytecode__9__L282.shape` | `__original__(args)` is the old replacement-body convention. Current forwarding must pass real parameters, not a hidden array. |
| `design_or_proof_gap` | L515 `E__advanced__content-addressed-bytecode__14__L515.shape`; L541 `E__advanced__content-addressed-bytecode__15__L541.shape` | Transport behavior and `state::caller`/`args`/`locals` are not yet proven runnable user snippets. |

### `advanced/developer-tools.mdx`

Disabled count: 5

Counts:

| Classification | Count |
|---|---:|
| `design_or_proof_gap` | 3 |
| `preview_or_out_of_scope` | 2 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `preview_or_out_of_scope` | L86 `E__advanced__developer-tools__0__L86.shape`; L320 `E__advanced__developer-tools__3__L320.shape` | Hot reload and blob prefetcher examples describe developer/tooling APIs not exposed as a current `std::debug` Shape module. |
| `design_or_proof_gap` | L137 `E__advanced__developer-tools__1__L137.shape`; L238 `E__advanced__developer-tools__2__L238.shape`; L462 `E__advanced__developer-tools__4__L462.shape` | Rust-side time-travel/proof support exists in places, but no established user-facing proof API or full distributed proof invariant is present. |

### `advanced/comptime.mdx`

Disabled count: 2

Counts:

| Classification | Count |
|---|---:|
| `active_feature_gap` | 1 |
| `preview_or_out_of_scope` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `preview_or_out_of_scope` | L76 `D__advanced__comptime__2__L76.shape` | Intentional negative example: runtime `marker` access inside `comptime` should not be flipped as a positive snippet. |
| `active_feature_gap` | L266 `D__advanced__comptime__6__L266.shape` | DuckDB schema inference mixes comptime FFI, `set param`, and generated return types. This is broader than current directive proof. |

### `advanced/comptime-annotations-cookbook.mdx`

Disabled count: 4

Counts:

| Classification | Count |
|---|---:|
| `active_feature_gap` | 2 |
| `design_or_proof_gap` | 1 |
| `preview_or_out_of_scope` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `active_feature_gap` | L31 `D__advanced__comptime-annotations-cookbook__0__L31.shape` | Same DuckDB/schema-inference gap as `advanced/comptime.mdx`. |
| `active_feature_gap` | L183 `D__advanced__comptime-annotations-cookbook__3__L183.shape` | `await_expr` host-routing needs async remote/Future semantics and annotation routing proof. |
| `preview_or_out_of_scope` | L308 `D__advanced__comptime-annotations-cookbook__9__L308.shape` | Stacked retry/circuit/fallback/host annotations are policy pseudocode until the async annotation stack is implemented. |
| `design_or_proof_gap` | L329 `D__advanced__comptime-annotations-cookbook__10__L329.shape` | Snapshot-step workflow uses real `snapshot()`, but a repeatable checkpoint/resume/step proof with store selection is not established by this snippet. |

### `advanced/comptime-llm-patterns.mdx`

Disabled count: 1

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `stale_disabled_candidate` | L170 `E__advanced__comptime-llm-patterns__4__L170.shape` | `extend (f"fn ...")` plus `string_lit` is now an explicitly tested comptime flagship path. Needs a self-contained wrapper before flipping. |

### `examples/comptime-codegen.mdx`

Disabled count: 1

Counts:

| Classification | Count |
|---|---:|
| `old_syntax_or_policy_rewrite` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `old_syntax_or_policy_rewrite` | L22 `C__examples__comptime-codegen__0__L22.shape` | Broad conceptual example mixes old runtime-hook syntax, source-string generation, untyped methods, `Result<Connection>`, and CSV/type-generation placeholders. Rewrite around current focused comptime surfaces. |

### `tooling/polyglot.mdx`

Disabled count: 4

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 1 |
| `active_feature_gap` | 1 |
| `design_or_proof_gap` | 1 |
| `preview_or_out_of_scope` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `stale_disabled_candidate` | L14 `D__tooling__polyglot__0__L14.shape` | Simple local Python scalar example likely belongs in the extension-gated FFI smoke lane. |
| `design_or_proof_gap` | L96 `D__tooling__polyglot__2__L96.shape` | Python returning `Vec<Element>` / typed object arrays needs explicit marshal proof beyond scalar/container tests. |
| `active_feature_gap` | L126 `D__tooling__polyglot__3__L126.shape` | Async Python plus external HTTP is not a book-flip candidate. |
| `preview_or_out_of_scope` | L186 `D__tooling__polyglot__7__L186.shape` | NumPy dependency examples are useful prose, but not default book-truth material without an extension/dependency fixture. |

### `tooling/python-extension.mdx`

Disabled count: 6

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 1 |
| `active_feature_gap` | 3 |
| `design_or_proof_gap` | 1 |
| `old_syntax_or_policy_rewrite` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `stale_disabled_candidate` | L68 `D__tooling__python-extension__1__L68.shape` | Scalar `fn python add` has dedicated FFI-tier coverage when extensions are built. |
| `design_or_proof_gap` | L117 `D__tooling__python-extension__3__L117.shape` | Typed object return should get a focused extension smoke before flipping. |
| `active_feature_gap` | L142 `D__tooling__python-extension__4__L142.shape`; L163 `D__tooling__python-extension__5__L163.shape`; L197 `D__tooling__python-extension__7__L197.shape` | Async Python fetch examples add async foreign runtime and external network dependencies. |
| `old_syntax_or_policy_rewrite` | L184 `D__tooling__python-extension__6__L184.shape` | Nonconforming returns are currently tested as catchable `TypeConformanceError`, not the older `MARSHAL_ERROR` wording. Rewrite expected prose/output. |

### `tooling/typescript-extension.mdx`

Disabled count: 5

Counts:

| Classification | Count |
|---|---:|
| `stale_disabled_candidate` | 2 |
| `active_feature_gap` | 1 |
| `design_or_proof_gap` | 1 |
| `old_syntax_or_policy_rewrite` | 1 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `stale_disabled_candidate` | L74 `D__tooling__typescript-extension__1__L74.shape`; L238 `D__tooling__typescript-extension__6__L238.shape` | Scalar `fn typescript add` has FFI-tier coverage. The TypeScript extension also ships a `typescript` module with `eval`/`import` helpers, but this needs extension-gated smoke. |
| `design_or_proof_gap` | L134 `D__tooling__typescript-extension__3__L134.shape` | Typed object return needs explicit marshal proof. |
| `old_syntax_or_policy_rewrite` | L163 `D__tooling__typescript-extension__4__L163.shape` | Error-channel wording should align with current conformance/error semantics before flipping. |
| `active_feature_gap` | L180 `D__tooling__typescript-extension__5__L180.shape` | Async TypeScript fetch adds async foreign runtime and external HTTP. |

### `advanced/security-permissions.mdx`

Disabled count: 9

Counts:

| Classification | Count |
|---|---:|
| `design_or_proof_gap` | 4 |
| `old_syntax_or_policy_rewrite` | 1 |
| `preview_or_out_of_scope` | 4 |

| Classification | Lines / snippet ids | Notes |
|---|---|---|
| `design_or_proof_gap` | L162 `E__advanced__security-permissions__4__L162.shape`; L387 `E__advanced__security-permissions__11__L387.shape`; L445 `E__advanced__security-permissions__13__L445.shape`; L502 `E__advanced__security-permissions__15__L502.shape` | Permission checks, load-time permission filtering, runtime network permission, and tiered security are real Rust/VM concepts, but the snippets are host/security proof sketches rather than self-contained Shape examples. |
| `preview_or_out_of_scope` | L333 `E__advanced__security-permissions__8__L333.shape`; L350 `E__advanced__security-permissions__9__L350.shape`; L364 `E__advanced__security-permissions__10__L364.shape`; L470 `E__advanced__security-permissions__14__L470.shape` | `PermissionGrant`, direct `ScopeConstraints` construction, and `ResourceUsage` are host-side concepts, not current Shape user APIs. |
| `old_syntax_or_policy_rewrite` | L418 `E__advanced__security-permissions__12__L418.shape` | Uses stale `io::write_file` teaching surface. Current permission examples should use real `std::core::file::write_text` or handle-based `io::write`. |

## Cross-Cutting Findings

1. `@remote` docs are the biggest stale-policy cluster.
   `@remote` should not be taught as a catchable `Result` surface. Use bare
   return/raising semantics for `@remote`, and teach `remote::call` for
   recoverable `Result<R, RemoteError>`.

2. `remote::__call` must be removed from the book.
   It is explicitly retired. User docs should name `remote::call`; internal
   dunder names should only appear in implementation/design docs.

3. `std::core::state` is a real module but not yet a green user chapter.
   `state.hash` is closest to flip-ready. `capture*`, `capture_call`,
   `serialize`, `deserialize`, `resume`, and introspection examples need W17
   marshal-return/state-resume work before they become truth-bearing snippets.

4. Transport is in a partial implementation/proof state.
   Native exports exist, including memoized transport, but live behavior tests
   are explicitly deferred. Keep transport examples disabled until a focused
   loopback proof lane lands.

5. Local polyglot examples are split between stale candidates and proof gaps.
   Scalar Python/TypeScript examples are likely current under the extension
   tier. Typed object returns, remote transfer, async foreign functions, and
   third-party dependency examples need dedicated extension-gated proofs.

6. Comptime source-string generation is current but should not become the
   long-term ergonomic endpoint.
   `extend (expr)` plus `string_lit` is tested and useful. The next design lane
   should still pursue typed fragments/quasiquote so docs do not overfit to
   source-string assembly.

7. Proof guards are clean but bounded.
   Source-only ignored-test classification and typed-opcode proof guards are
   valuable, but they do not replace supervisor-lane cargo projections, full
   Miri coverage, distributed e2e, or book-truth execution.

## Next Recommended Wave

### Implementation Lanes

1. `remote::call_async`
   - Add a public `remote::call_async(addr, fn, args...) -> Future<Result<R, RemoteError>>` surface.
   - Reuse the typed positional elaboration shape from `remote::call`.
   - Add scheduler/external-completion plumbing so `await remote::call_async(...)`
     resolves from remote completion, not by wrapping blocking `remote::call`.
   - Pin cancellation, timeout, and snapshot behavior for in-flight remote
     futures before enabling book examples.

2. Snapshot/state completion
   - Finish W17 marshal-return arms for `state.capture*`, `state.capture_call`,
     `state.serialize`, `state.deserialize`, `state.caller`, `state.args`, and
     `state.locals`.
   - Make `state.resume`/`resume_frame` a real user surface or remove from
     positive book examples.
   - Add remote receiver snapshot resume via a serve snapshot-store selector or
     resume RPC, then unignore the receiver-resume tracker.

3. Transport layer hardening
   - Align `transport.shape` signatures and docs with native exports
     (`Array<int>` vs `Vec<int>`, `IoHandle` return shape, memoized exports).
   - Add focused loopback TCP send/connect/recv tests, plus memoized cache hit,
     stats, and invalidate tests.
   - Decide whether QUIC is a release surface; if not, mark QUIC snippets as
     preview prose only.

4. Polyglot transfer and local extension coverage
   - Promote Python/TypeScript remote transfer into a non-ambiguous extension
     gate: either required in CI's FFI lane or clearly skipped with a separate
     proof artifact.
   - Add typed object return and typed object array return coverage for Python
     and TypeScript.
   - Split async foreign examples from local scalar docs; async examples need
     their own runtime/network story.

5. Comptime typed APIs
   - Close `set param` default metadata so omitted args and explicit overrides
     work at public call sites.
   - Close generated `replace module` re-verification against declared return
     types.
   - Design typed generated-code fragments/quasiquote so future docs can reduce
     dependence on source strings.
   - Defer `await_expr` policy annotations until remote futures and async
     annotation routing are real.

6. Permission/security docs
   - Rewrite Shape snippets to current `std::core::file`/`std::core::io`
     surfaces.
   - Separate host API sketches (`PermissionSet`, `ScopeConstraints`,
     `ResourceLimits`) from runnable Shape examples.
   - Add proof tests for permission derivation over content-addressed blobs,
     receiver refusal, scoped network/file constraints, and deterministic/FFI
     refusal.

### Proof/Test-Hardening Lanes

1. Distributed smoke matrix
   - Live `remote::execute`.
   - Live `remote::call` Ok/Err.
   - `@remote` bare-return behavior.
   - Receiver-side permission refusal.
   - Receiver-side snapshot hash.
   - Extension-gated Python/TypeScript transfer.

2. Snapshot proof matrix
   - Local hash/resume.
   - SIGINT snapshot/resume.
   - Remote receiver hash/resume from receiver store.
   - Foreign-frame barrier and "snapshot across foreign calls" distinction.
   - Cross-node resume compatibility: blobs, schemas, extensions, and
     permissions.

3. Comptime proof matrix
   - Keep WF-3D flagship generated free function/method/type_info/diagnostics
     green in VM and JIT.
   - Add active tests for `set param` default metadata and `replace module`
     re-verification before flipping cookbook examples.
   - Add negative-doc harness support before considering intentional-error
     snippets runnable.

4. Global proof boundaries
   - Refresh cargo-reported ignored-test projection only in the supervisor build
     lane.
   - Expand Miri from targeted probes only where it answers a concrete pointer
     or snapshot-carrier question.
   - Keep `check-typed-opcode-proof-coverage.py` at `unproven_gap = 0`, but do
     not treat it as a semantic proof for distributed, FFI, snapshot, or
     permission behavior.
