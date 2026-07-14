# Wave 36C Distributed/Snapshot/Polyglot Deep-Test Gap

Date: 2026-07-10
Role: Wave-36C distributed/snapshot/polyglot deep-test gap scout

## Scope

Static inspection only. I did not run cargo, just, nextest, rustc, build, test,
or book-truth commands. I wrote only this report.

Current verified book baseline is the Wave-35 state supplied by the supervisor:
707 Shape snippets total, 557 runnable, 150 disabled, and a full release-binary
book gate pass of 557/557. The local Wave-36A disabled inventory matches that
baseline and records 6 fixture snippets, 6 expected-output snippets, and 6
expected-fail snippets (`docs/cluster-audits/wave36-disabled-current-triage.md`).

Primary sources inspected:

- `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs`
- `bin/shape-cli/tests/distributed_matrix_e2e.rs`
- `bin/shape-cli/tests/distributed_extern_c_snapshot_e2e.rs`
- `bin/shape-cli/tests/distributed_dynamic_snapshot_e2e.rs`
- `bin/shape-cli/tests/distributed_composition_e2e.rs`
- `bin/shape-cli/tests/distributed_proof_matrix_e2e.rs`
- `bin/shape-cli/tests/distributed_async_e2e.rs`
- `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`
- `crates/shape-vm/src/remote.rs`
- `crates/shape-vm/src/executor/builtins/remote_builtins.rs`
- sibling book pages for remote, execution-server, content-addressed bytecode,
  resumability, and polyglot-distributed
- prior audit reports, especially Waves 31, 33, 34, and 36A

## Bottom Line

The core composition is no longer merely claimed. There is real CLI e2e coverage
for:

- local CLI snapshot hash -> `shape --resume` with a selected snapshot store
- live `shape serve` remote execution over plain TCP
- `remote::call` named functions, scalar argument/return shape, closure capture,
  recoverable transport errors, and TLS user surface
- `@remote` receiver-side snapshots and receiver-store resume
- extern-C remote transfer and strict-node FFI refusal
- extern-C combined `@remote` + receiver snapshot + resume
- Python and TypeScript remote transfer and receiver snapshot/resume when dynamic
  extension shared libraries are available
- TLS receiver-store isolation and TLS dynamic Python/TypeScript combined
  snapshot/resume when extension shared libraries are available
- async `remote::call_async` success/error/snapshot/fan-in/live-future barrier,
  plus ignored serialized cancellation proofs for TCP and TLS

The remaining risk is not "there are no tests". The risk is sharper:

- Python/TypeScript positive composition tests self-skip when extension `.so`
  files are absent. They are deep when enabled, but not a guaranteed default
  cargo gate.
- Book truth covers extern-C transfer and extern-C combined receiver resume, but
  still leaves Python/TypeScript extension rows manual/external.
- Closure/capture coverage is strong for `remote::call`, but the `@remote`
  value-type/capture matrix in the book is still disabled and not pinned by a
  cargo e2e matrix.
- Content-addressed hash/resupply logic is well unit-tested in `remote.rs`, but
  real-socket serve e2e coverage does not yet force missing-blob resupply or
  persistent blob-cache behavior.
- Async cancellation is deeply probed, including TLS, but all cancellation tests
  are `#[ignore]` timing-sensitive supervisor-lane tests rather than default
  tests.

## Coverage Matrix

| Axis | Strong existing coverage | Partial / missing deep test |
|---|---|---|
| Plain TCP vs TLS | Plain TCP has broad e2e coverage through `start_serve`: `remote::execute`, `remote::call`, `@remote` receiver snapshot, extern-C transfer, selected receiver store, Python/TypeScript dynamic transfer, and async success/fan-in. TLS has user-surface `remote::call`, missing-CA and wrong-name refusals, receiver-store isolation, TLS dynamic Python/TypeScript combined resume, TLS async join-all receiver-store proof, and ignored TLS cancellation variants. Sources: `distributed_snapshot_polyglot_e2e.rs:61`, `:85`, `:125`, `:231`, `:267`, `:272`, `:397`, `:403`, `:444`; `distributed_matrix_e2e.rs:10`, `:38`, `:67`, `:73`; `distributed_composition_e2e.rs:8`, `:22`; `distributed_proof_matrix_e2e.rs:10`. | No book-truth TLS fixture. No non-extension TLS extern-C combined e2e analogous to `remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store`. TLS dynamic tests skip without `.so` files. |
| `remote::call` vs `@remote` | `remote::call` is covered for Ok/Err, named functions, int preservation, two-arg calls, number closure capture, TLS, and async. `@remote` is covered for execution-server scalar call, receiver snapshot, extern-C transfer, Python/TypeScript dynamic wrapper transfer, receiver snapshot/resume, and TLS dynamic combined resume. Sources: `distributed_snapshot_polyglot_e2e.rs:86`, `:157`, `:181`, `:205`, `:231`, `:267`, `:272`, `:403`, `:444`; `execution-server.mdx:130`; `distributed_async_e2e.rs:7`, `:37`, `:70`, `:110`, `:143`, `:175`. | The disabled book value-type matrix at `stdlib/core/remote.mdx:77` claims integer/array/string/closure examples but is not truth-gated. There is no focused `@remote` cargo matrix for string, array/object, module-global capture, and refusal semantics. |
| Named function vs closure/captures | Named functions are covered in CLI and unit layers: `remote_call_two_argument_function_over_shape_serve`, `remote_call_int_argument_and_return_preserves_value_kind`, `remote_named_function_executes_end_to_end`, and `build_call_request_by_id_round_trips_named_function`. Closure capture has a live CLI e2e through `remote::call` and unit coverage for immutable capture execution, mutable capture refusal, and missing kind-track refusal. Sources: `distributed_snapshot_polyglot_e2e.rs:157`, `:181`, `:205`; `remote.rs:3958`, `:3976`, `:4022`, `:4073`, `:4178`. | Closure coverage is mostly `remote::call`, not the `@remote` annotation path. Mutable/reference/resource/nested closure refusals are unit-level, not live-server CLI e2e. |
| Local snapshot/resume vs receiver snapshot/resume | Local CLI selected-store roundtrip is covered by `snapshot_hash_resume_cli_roundtrip`. Receiver snapshot hash and selected receiver-store resume are covered by `remote_snapshot_hash_is_saved_in_selected_receiver_store`, `remote_snapshot_hash_can_be_resumed_from_receiver_store`, and store-isolation tests for TCP/TLS. Sources: `distributed_snapshot_polyglot_e2e.rs:9`, `:403`, `:444`; `distributed_matrix_e2e.rs:67`, `:73`. | The book's generic resumability rows remain disabled stale/fixture candidates (`advanced/resumability.mdx:20`, `:100`). Local Python/TypeScript snapshot->resume across foreign calls is still a manual/book row (`advanced/polyglot-distributed.mdx:149`) rather than a guaranteed cargo/book fixture. |
| Selected snapshot store vs default store | Selected stores are strong: local CLI roundtrip uses `--snapshot-store`; receiver selected-store tests prove `snapshot info` succeeds in receiver store and fails in caller store; dynamic/TLS composition tests resume from the selected receiver store. Sources: `distributed_snapshot_polyglot_e2e.rs:9`, `:403`, `:444`; `distributed_matrix_e2e.rs:67`, `:73`; `distributed_dynamic_snapshot_e2e.rs:8`, `:22`; `distributed_composition_e2e.rs:8`, `:22`; `distributed_proof_matrix_e2e.rs:10`. | Default receiver-store hash production is covered (`remote_snapshot_returns_receiver_hash_over_remote_call`, `remote_call_async_receiver_snapshot_returns_hash`) but not as deeply as selected-store visibility/resume. |
| Extern-C vs Python vs TypeScript transfer | Extern-C transfer is always-on and tests strict FFI refusal: `remote_extern_c_transfer_executes_and_strict_node_refuses_ffi`. Python/TypeScript transfer tests run VM and JIT modes and assert serve logs `foreign_entries=1` when `.so` files exist. Sources: `distributed_snapshot_polyglot_e2e.rs:231`, `:267`, `:272`; `support/distributed_snapshot_polyglot.rs:335`. | Python/TypeScript positive transfer tests self-skip if `libshape_ext_python.so` / `libshape_ext_typescript.so` is unavailable. Book truth has only the extern-C transfer row (`advanced/polyglot-distributed.mdx:74`), while the Python/TypeScript book examples remain manual/external (`advanced/polyglot-distributed.mdx:149`, `stdlib/core/remote.mdx:107`). |
| Dynamic receiver runtime opt-in / strict-node refusal | Extern-C strict sandbox refusal is covered. Python/TypeScript no-opt-in refusal is covered even without extensions by asserting the receiver rejects at `--ffi-languages` before runtime lookup. Positive dynamic runtime opt-in is covered when `.so` files exist. Sources: `distributed_snapshot_polyglot_e2e.rs:231`, `:267`, `:272`; `distributed_matrix_e2e.rs:79`, `:91`. | Positive dynamic tests are not guaranteed default coverage. There is no book fixture for dynamic language opt-in/refusal. |
| Content-addressed function resupply and hash visibility | Unit coverage in `remote.rs` is strong: deterministic program hash, by-hash minimal blobs, missing entry refusal, blob cache insert/LRU/filter, blob negotiation, known-blob stripping, receiver permission by hash, hash mismatch rejection, missing dependency surfaced with missing blob hashes, retry-once resupply, and no retry on non-missing errors. Sources: `remote.rs:2899`, `:2963`, `:3017`, `:3033`, `:3045`, `:3087`, `:3119`, `:3461`, `:3483`, `:3553`, `:3781`, `:3819`, `:3851`, `:3876`, `:4108`, `:4154`. Public hash visibility is book-runnable through `state::hash` / `state::fn_hash` examples (`content-addressed-bytecode.mdx:202`). | No live `shape serve` e2e forces receiver missing-blob resupply or persistent blob-cache negotiation over a real socket. The disabled content-addressed rows for arbitrary state transfer/storage remain active gaps (`content-addressed-bytecode.mdx:154`, `:168`, `:264`, `:541`). |
| Cancellation / async interaction | `remote::call_async` e2e covers await success, transport Err as inner Result, receiver snapshot hash, two-call composition, ordered `join all`, and live-future snapshot barrier followed by successful await. Ignored serialized cancellation tests cover TCP and TLS scope exit, race loser, queued cancellation visibility, running-call honesty, and TLS blackhole handshake cancellation. Sources: `distributed_async_e2e.rs:7`, `:37`, `:70`, `:110`, `:143`, `:175`; `distributed_async_cancellation_e2e.rs:349`, `:362`, `:375`, `:417`, `:430`, `:443`, `:456`, `:469`, `:482`. | Cancellation proof is supervisor-only ignored coverage, not default. Missing larger semantics remain: remote callees returning `Future<T>`, pending-future snapshot/resume, remote future identity, streams, and JIT async lowering. |
| Book fixture vs cargo e2e | Book truth now covers six distributed fixture snippets: extern-C remote transfer, extern-C combined receiver resume, `remote::execute`, `remote::ping`, `remote::call`, and execution-server `@remote`. Sources: `polyglot-distributed.mdx:74`, `:213`; `remote.mdx:136`, `:160`, `:181`; `execution-server.mdx:130`. Cargo e2e coverage is broader than book truth. | Current external/manual distributed rows still include `advanced/polyglot-distributed.mdx:149`, `stdlib/core/remote.mdx:42`, `:77`, `:107`, `:220`, `content-addressed-bytecode.mdx:515`, `module-distribution.mdx:563`, `wire-protocol.mdx:90`, and `stdlib/core/transport.mdx:61`, `:95` (`wave36-disabled-current-triage.md`). |

## Existing Test Inventory

### Default or ordinary CLI e2e coverage

- `snapshot_hash_resume_cli_roundtrip`
  (`bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs:9`): local
  `snapshot()` hash, selected store, `snapshot info`, and `shape --resume`.
- `remote_execute_user_surface_over_shape_serve`
  (`distributed_snapshot_polyglot_e2e.rs:62`): live `remote::execute`.
- `remote_call_user_result_ok_and_transport_err_over_shape_serve`
  (`distributed_snapshot_polyglot_e2e.rs:86`): live Ok plus dead-port Err.
- `remote_snapshot_returns_receiver_hash_over_remote_call`
  (`distributed_snapshot_polyglot_e2e.rs:125`): `@remote` receiver-side
  `snapshot()` returns a hash.
- `remote_call_two_argument_function_over_shape_serve`,
  `remote_call_int_argument_and_return_preserves_value_kind`, and
  `remote_call_number_closure_capture_over_shape_serve`
  (`distributed_snapshot_polyglot_e2e.rs:157`, `:181`, `:205`).
- `remote_extern_c_transfer_executes_and_strict_node_refuses_ffi`
  (`distributed_snapshot_polyglot_e2e.rs:231`): extern-C server execution and
  strict receiver refusal.
- `tls_remote_call_user_surface_over_shape_serve`
  (`distributed_snapshot_polyglot_e2e.rs:397`): trusted TLS succeeds, plaintext
  to a TLS server fails.
- `remote_snapshot_hash_is_saved_in_selected_receiver_store` and
  `remote_snapshot_hash_can_be_resumed_from_receiver_store`
  (`distributed_snapshot_polyglot_e2e.rs:403`, `:444`).
- `tls_remote_call_refuses_missing_ca_trust_root` and
  `tls_remote_call_refuses_mismatched_server_name`
  (`distributed_matrix_e2e.rs:10`, `:38`).
- `plaintext_remote_snapshot_uses_receiver_store_not_caller_store` and
  `tls_remote_snapshot_uses_receiver_store_not_caller_store`
  (`distributed_matrix_e2e.rs:67`, `:73`).
- `remote_python_call_refuses_receiver_without_language_opt_in` and
  `remote_typescript_call_refuses_receiver_without_language_opt_in`
  (`distributed_matrix_e2e.rs:79`, `:91`).
- `remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store`
  (`distributed_extern_c_snapshot_e2e.rs:8`): extern-C combined cell,
  receiver selected store, resume prints `RESUMED:43`.
- `tls_remote_call_async_join_all_snapshots_land_in_receiver_store_only`
  (`distributed_proof_matrix_e2e.rs:10`): TLS async fan-in returns two
  distinct receiver-owned snapshot hashes and excludes caller store.
- `distributed_async_e2e.rs` covers async success, transport Err, receiver
  snapshot, two-call composition, ordered join-all, and live-future snapshot
  refusal (`:7`, `:37`, `:70`, `:110`, `:143`, `:175`).

### Extension-available but self-skipping coverage

These are real e2es when `language_ext_so(language)` finds the runtime `.so`,
but they print `SKIP ...` and return early otherwise:

- `remote_python_transfer_self_skips_without_extension_and_refuses_without_opt_in`
  (`distributed_snapshot_polyglot_e2e.rs:267`)
- `remote_typescript_transfer_self_skips_without_extension_and_refuses_without_opt_in`
  (`distributed_snapshot_polyglot_e2e.rs:272`)
- `remote_python_snapshot_hash_can_be_resumed_from_receiver_store`
  (`distributed_dynamic_snapshot_e2e.rs:8`)
- `remote_typescript_snapshot_hash_can_be_resumed_from_receiver_store`
  (`distributed_dynamic_snapshot_e2e.rs:22`)
- `tls_remote_python_snapshot_hash_can_be_resumed_from_selected_receiver_store`
  (`distributed_composition_e2e.rs:8`)
- `tls_remote_typescript_snapshot_hash_can_be_resumed_from_selected_receiver_store`
  (`distributed_composition_e2e.rs:22`)

When enabled, these tests are strong: they use a selected receiver store, pass
`--extension` on resume, assert resumed values (`RESUMED:120`, `RESUMED:30`,
`RESUMED:218`, `RESUMED:42`), and assert the serve log saw a foreign stub.

### Ignored supervisor-lane coverage

- `sigint_saves_snapshot_and_plain_resume_continues`
  (`distributed_snapshot_polyglot_e2e.rs:336`) is timing-sensitive and ignored.
- `distributed_async_cancellation_e2e.rs` has nine ignored serialized proof
  tests for TCP/TLS cancellation and TLS blackhole handshake cancellation
  (`:349`, `:362`, `:375`, `:417`, `:430`, `:443`, `:456`, `:469`, `:482`).

The Wave-35 AGENTS evidence says the full ignored cancellation file passed 9/0
under the supervisor cgroup lane, but these tests remain outside default cargo
execution.

## Book Truth Surface

Book fixture rows that are currently runnable:

- `advanced/polyglot-distributed.mdx:74`: extern-C `@remote` transfer,
  `fixture=serve`, exact `REMOTE_C_ABS=42`.
- `advanced/polyglot-distributed.mdx:213`: extern-C combined receiver
  snapshot/resume, `fixture=serve-snapshot-resume`, exact `RESUMED:43`.
- `stdlib/core/remote.mdx:136`: `remote::execute`, `fixture=serve`, exact
  `REMOTE_EXEC_OK=Int(42)`.
- `stdlib/core/remote.mdx:160`: `remote::ping`, `fixture=serve`, exact
  `PING_OK`.
- `stdlib/core/remote.mdx:181`: `remote::call`, `fixture=serve`, exact
  `REMOTE_CALL_SQUARE=49`.
- `tooling/execution-server.mdx:130`: execution-server `@remote`, exact
  `EXEC_SERVER_REMOTE_MUL=42`.

Distributed/book rows still external or manual:

- `advanced/polyglot-distributed.mdx:149`: Python snapshot->resume across
  foreign calls; needs extension loading and two-process resume.
- `stdlib/core/remote.mdx:42`: illustrative `@remote("worker:9527")` row.
- `stdlib/core/remote.mdx:77`: value-type matrix for integer, array, string,
  closures; needs a controlled receiver fixture and current truth pinning.
- `stdlib/core/remote.mdx:107`: remote Python matrix-multiply; needs extension,
  NumPy, receiver opt-in.
- `stdlib/core/remote.mdx:220`: dead endpoint Result example; needs reserved
  unused port fixture.
- `advanced/content-addressed-bytecode.mdx:515`: live TCP transport example.
- `advanced/module-distribution.mdx:563`: packaged module/bundle fixture.
- `advanced/wire-protocol.mdx:90`, `stdlib/core/transport.mdx:61`, and
  `stdlib/core/transport.mdx:95`: framed peer/protocol fixtures.

## Smallest Next Deep-Test Lanes

### Lane 1: Make dynamic Python/TypeScript composition non-silent in CI

Goal: keep the existing skip-friendly behavior for local developers, but add a
required-extension mode for CI/supervisor runs so Python/TypeScript composition
cannot silently disappear from the deep-test signal.

Suggested files:

- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`
- `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs`
- `bin/shape-cli/tests/distributed_dynamic_snapshot_e2e.rs`
- `bin/shape-cli/tests/distributed_composition_e2e.rs`

Suggested tests / helper names:

- `require_language_ext_so(language)` in the support helper. It should panic
  with setup instructions when `SHAPE_REQUIRE_FFI_EXT=1` and the `.so` is absent.
- `remote_python_transfer_executes_on_receiver_when_extensions_required`
- `remote_typescript_transfer_executes_on_receiver_when_extensions_required`
- `remote_python_snapshot_resume_required_extension_roundtrip`
- `remote_typescript_snapshot_resume_required_extension_roundtrip`
- `tls_remote_python_snapshot_resume_required_extension_roundtrip`
- `tls_remote_typescript_snapshot_resume_required_extension_roundtrip`

This can initially wrap the existing helper bodies. The important change is
classification: extension-backed composition should be "required and failed" in
the dedicated lane, not "skipped and invisible".

### Lane 2: Add extension-aware book fixtures

Goal: close the gap between cargo e2e truth and book truth for dynamic
Python/TypeScript examples.

Suggested sibling book files:

- `../shape-web/book/book-site/scripts/extract-shape-snippets.mjs`
- `../shape-web/book/book-site/scripts/run-book-truth-gate.mjs`
- `../shape-web/book/book-site/scripts/MANIFEST_SCHEMA.md`
- `../shape-web/book/book-site/scripts/serve-fixture.test.mjs`
- `../shape-web/book/book-site/src/content/docs/advanced/polyglot-distributed.mdx`
- `../shape-web/book/book-site/src/content/docs/stdlib/core/remote.mdx`

Suggested fixture/test names:

- `fixture=serve-extension`
- `fixture=serve-extension-snapshot-resume`
- Node tests named `serve_extension_fixture_loads_language_runtime`,
  `serve_extension_fixture_refuses_missing_language_opt_in`, and
  `serve_extension_snapshot_resume_fixture_resumes_selected_store`.

First rows to target:

- `advanced/polyglot-distributed.mdx:149` for Python snapshot->resume across
  foreign calls, or split it into Python and TypeScript fixture rows.
- A narrowed replacement for `stdlib/core/remote.mdx:107` that avoids NumPy and
  proves scalar `fn python` / `fn typescript` remote execution with explicit
  receiver opt-in.

Do not make extension availability a global book-gate prerequisite. The fixture
should either run only in an explicit extension fixture mode or classify the row
as skipped-with-reason outside that mode.

### Lane 3: Pin the `@remote` value/capture and real-socket blob-resupply matrix

Goal: turn the remaining `@remote` and content-addressed distribution claims
into live-server probes, especially where current coverage is unit-only or
book-disabled.

Suggested files:

- `bin/shape-cli/tests/distributed_matrix_e2e.rs`
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`
- optional, only if a protocol helper is needed:
  `bin/shape-cli/tests/distributed_resupply_e2e.rs`

Suggested tests:

- `remote_annotation_string_argument_over_shape_serve`
- `remote_annotation_array_argument_truthfully_pinned`
- `remote_annotation_object_argument_truthfully_pinned`
- `remote_annotation_module_global_capture_over_shape_serve`
- `remote_annotation_closure_capture_refusal_is_user_visible`
- `remote_call_missing_blob_resupply_over_shape_serve`
- `remote_call_repeated_function_uses_receiver_blob_cache_over_shape_serve`

Acceptance should be explicit: if arrays/objects/module globals are supported,
assert the returned value; if they are intentionally unsupported, assert a clean
compile/runtime refusal and update the book claim. The resupply/cache tests may
need a lower-level wire helper because the public `remote::call` path normally
ships a complete minimal blob closure.

### Lane 4: Make async cancellation proof cheaper to run

Goal: preserve the current strong ignored tests but add at least one
deterministic default-gate smoke that proves cancellation hooks are wired
without relying on timing thresholds.

Suggested file:

- `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`

Suggested tests:

- `remote_call_async_cancel_queued_call_records_cancelcall_smoke`
- `tls_remote_call_async_cancel_queued_call_records_cancelcall_smoke`

Keep the full promptness/race/running-honesty suite ignored and serialized, but
add a short queued-call log assertion that can run normally if it is stable.

## Recommended Priority

1. Lane 1: required-extension CI mode for dynamic Python/TypeScript composition.
   This converts the largest remaining "deep when present, invisible when
   absent" risk into an explicit gate.
2. Lane 3: `@remote` value/capture plus real-socket resupply/cache matrix. This
   pins the still-disabled public `remote.mdx` value claims and raises
   content-addressed resupply from unit-only to live-server evidence.
3. Lane 2: extension-aware book fixtures. This closes the documentation truth
   gap after the dynamic tests are non-silent.

Lane 4 is valuable, but the current ignored cancellation suite is already deep
and recently verified by the supervisor; it is less urgent than making
Python/TypeScript composition and the disabled `@remote` value matrix non-silent.
