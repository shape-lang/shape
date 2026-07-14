# Wave-31B distributed book-fixture next slice

Date: 2026-07-10

Mode: static/file inspection only. I did not run cargo, just, nextest, rustc,
build, test suites, the snippet extractor, or the book-truth gate.

## Question

Design the smallest deterministic book-truth fixture lane that can make some
currently disabled distributed snippets runnable without weakening the gate.

Scoped sources inspected:

- `../shape-web/book/book-site/scripts/extract-shape-snippets.mjs`
- `../shape-web/book/book-site/scripts/run-book-truth-gate.mjs`
- scoped distributed book pages under `../shape-web/book/book-site/src/content/docs/**`
- `bin/shape-cli/tests/distributed_*`
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`
- `bin/shape-cli/src/commands/serve_cmd.rs`
- narrow public-surface confirmation in `crates/shape-runtime/stdlib-src/core/{remote,transport}.shape`
  and `crates/shape-vm/src/executor/builtins/transport_builtins.rs`

## Current gate shape

The extractor emits one `.shape` file per `shape` fence and a manifest entry
with `runnable`, `deferred`, `cite`, `expected`, page, line, and path fields.
It already recovers raw fence metadata to parse `runnable=deferred` and
`expected="..."`, but it does not preserve arbitrary fixture metadata.
Source: `extract-shape-snippets.mjs:178-265`.

The runner executes each runnable snippet as a standalone file under both
`shape run --mode vm` and `shape run --mode jit`, compares stdout byte-for-byte,
and optionally compares both outputs to exact `expected`. It does not allocate
ports, start background servers, rewrite fixture placeholders, isolate XDG/config
dirs, set snapshot stores, create TLS material, load extensions, or run
multi-phase resume flows. Source: `run-book-truth-gate.mjs:88-214`.

This means live distributed rows are disabled for a good reason today: making
them runnable requires orchestration outside a single `shape run` invocation.

## Existing deterministic e2e harness evidence

The Rust distributed helper already demonstrates the process pattern the book
gate needs to copy in JavaScript:

- `IsolatedEnv` creates temporary `XDG_DATA_HOME` and `SHAPE_CONFIG_DIR`
  roots. Source: `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs:31-64`.
- `start_serve*` allocates a loopback port, starts `shape serve --address
  127.0.0.1:<port> --sandbox <level> --max-concurrent <n>`, captures stderr,
  polls readiness with `TcpStream::connect`, and kills the child on drop.
  Source: `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs:170-309`.
- The same helper has optional selected receiver snapshot stores, TLS cert
  generation, extension-dir symlinks, and `--ffi-languages`, but those are
  extra lanes, not needed for the first fixture. Source:
  `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs:182-284`.
- Existing e2e tests prove the runtime surfaces the first lane would rely on:
  `remote::execute` over `shape serve`, recoverable `remote::call`, scalar
  argument/return, closure capture, extern-C `@remote` transfer, receiver
  snapshot hashes, TLS, and async calls. Sources:
  `distributed_snapshot_polyglot_e2e.rs:61-264`,
  `distributed_async_e2e.rs:6-172`,
  `distributed_extern_c_snapshot_e2e.rs:7-58`.

`shape serve` itself supports the required loopback behavior:

- Non-loopback binds refuse to start without TLS and auth, while loopback is
  the intended local fixture path. Source: `serve_cmd.rs:281-350`.
- It loads extension runtimes once at startup, derives security from sandbox
  and FFI language opt-ins, binds the requested address, logs readiness, then
  handles framed `Ping`, `Execute`, `Call`, `CancelCall`, blob negotiation, and
  sidecar messages. Sources: `serve_cmd.rs:353-450`, `serve_cmd.rs:551-811`.
- Reported capabilities include `execute`, `validate`, `call`, `call-cancel`,
  and `blob-negotiation`. Source: `serve_cmd.rs:841-855`.

## Recommended first lane

Add a **plain loopback serve fixture** to the book-truth gate.

Smallest useful contract:

1. Add extractor support for a narrow fence flag, for example:
   `fixture=serve serve-sandbox=none`.
2. Preserve fixture metadata in manifest entries. Do not change default
   behavior for ordinary runnable snippets.
3. In the runner, when a snippet has `fixture=serve`, run each mode in an
   isolated fixture invocation:
   - create a temp dir
   - set `XDG_DATA_HOME` and `SHAPE_CONFIG_DIR` under it for both server and client
   - allocate `127.0.0.1:0`, then start `shape serve --address <addr>
     --sandbox none --max-concurrent 4`
   - poll readiness with a short deadline and include serve stderr tail on
     harness failure
   - write a temporary snippet with `__BOOK_SERVE_ADDR__` replaced by `<addr>`
   - run the normal `shape run --mode <vm|jit> <temp-snippet>` path
   - kill the serve child in `finally`
4. Require exact `expected="..."` on every newly flipped first-lane fixture
   whose output is stable. Do not introduce regex expectations in this lane.
5. Keep the gate sequential. The serve child will inherit the caller's cgroup
   when the supervisor runs the gate under `systemd-run`; the fixture script
   should not spawn its own cgroup wrapper.

Why this is the smallest lane:

- It reuses the current gate's central invariant: VM stdout equals JIT stdout,
  optionally equals exact expected stdout.
- It avoids dynamic hashes, resume, TLS certs, extension availability, auth,
  negative-port races, and protocol-byte construction.
- It unlocks real live `shape serve` rows rather than weakening them into local
  examples.

## Disabled row inventory

Read-only manifest inventory for the scoped pages lists 14 disabled Shape rows:

| Row | Current blocker | Classification |
|---|---|---|
| `advanced/polyglot-distributed.mdx:74` (`E__advanced__polyglot-distributed__1__L74.shape`) | Live loopback `shape serve --sandbox none`; fixed port must become fixture placeholder. | Fixture/harness-only for first lane. |
| `advanced/polyglot-distributed.mdx:149` (`E__advanced__polyglot-distributed__2__L149.shape`) | Dynamic Python extension plus selected snapshot store plus two-phase resume. | Fixture/external/multi-phase; keep disabled in first lane. |
| `advanced/polyglot-distributed.mdx:212` (`E__advanced__polyglot-distributed__3__L212.shape`) | Live receiver, selected receiver snapshot store, dynamic hash capture, resume command. | Fixture/multi-phase; not first lane. |
| `advanced/wire-protocol.mdx:90` (`E__advanced__wire-protocol__0__L90.shape`) | Conceptual bytes example with undefined `large_dataset`; no Shape-level construction of a valid `shape serve` request frame in the snippet. | Book/protocol implementation gap for book-truth; keep prose. |
| `stdlib/core/remote.mdx:41` (`B__stdlib__core__remote__1__L41.shape`) | Illustrative `worker:9527`, missing standalone import, array-map example. | Book rewrite plus fixture; not first flip as-is. |
| `stdlib/core/remote.mdx:76` (`B__stdlib__core__remote__2__L76.shape`) | Live serve fixture; several value-type examples in one fence. | Fixture/harness-only after splitting or tightening outputs. |
| `stdlib/core/remote.mdx:106` (`B__stdlib__core__remote__3__L106.shape`) | Dynamic Python extension, NumPy dependency, receiver opt-in. | External/manual fixture; keep disabled/prose. |
| `stdlib/core/remote.mdx:134` (`B__stdlib__core__remote__4__L134.shape`) | Live serve fixture and address placeholder. | Fixture/harness-only for first lane. |
| `stdlib/core/remote.mdx:154` (`B__stdlib__core__remote__5__L154.shape`) | Live serve fixture and address placeholder. | Fixture/harness-only for first lane if rewritten to print stable `PING_OK`. |
| `stdlib/core/remote.mdx:171` (`B__stdlib__core__remote__6__L171.shape`) | Live serve fixture and address placeholder. | Fixture/harness-only for first lane after standalone import/output rewrite. |
| `stdlib/core/remote.mdx:204` (`B__stdlib__core__remote__7__L204.shape`) | Needs deterministically reserved unused port. | Fixture/harness-only, but separate `dead-port` lane. |
| `stdlib/core/transport.mdx:61` (`B__stdlib__core__transport__3__L61.shape`) | Undefined `request_bytes`; needs a peer and valid payload protocol. | Book/protocol gap; not unlocked by plain serve. |
| `stdlib/core/transport.mdx:95` (`B__stdlib__core__transport__4__L95.shape`) | Non-loopback address, undefined `payload`, persistent protocol peer. | Book/protocol gap; not unlocked by plain serve. |
| `tooling/execution-server.mdx:130` (`E__tooling__execution-server__0__L130.shape`) | Live serve fixture and address placeholder. | Fixture/harness-only for first lane after stable print/assert rewrite. |

## First-lane unlock set

The first fixture lane should target the rows that need only a plain loopback
serve node and stable standalone book snippets:

- `advanced/polyglot-distributed.mdx:74`: extern-C `@remote` transfer. Replace
  the fixed `127.0.0.1:9702` with `__BOOK_SERVE_ADDR__`, keep
  `--sandbox none`, and set exact expected stdout `REMOTE_C_ABS=42\n`.
- `stdlib/core/remote.mdx:134`: `remote::execute`. Replace `localhost:9527`
  with `__BOOK_SERVE_ADDR__`, keep the example small, and assert stable output
  such as `REMOTE_EXEC_OK`.
- `stdlib/core/remote.mdx:154`: `remote::ping`. Replace `localhost:9527` with
  `__BOOK_SERVE_ADDR__`; print a constant on `Ok` so exact expected output can
  prove the server was live without baking the current version string into the
  book.
- `stdlib/core/remote.mdx:171`: `remote::call`. Add the standalone import,
  use `__BOOK_SERVE_ADDR__`, match `Ok(v)`, and exact expected stdout such as
  `REMOTE_CALL_SQUARE=49\n`.
- `tooling/execution-server.mdx:130`: `@remote` Call. Use
  `__BOOK_SERVE_ADDR__` and print/assert a stable value. This row can either
  keep the array example if a future worker confirms it, or be narrowed to the
  same scalar `mul(6, 7)` shape used by existing distributed e2e coverage.

Second batch in the same general lane, after the first five are stable:

- `stdlib/core/remote.mdx:76`: split the multi-value-type fence or add stable
  printed markers per value type. Keeping all four cases in one fence makes
  failure triage noisy and should not be the first flip.
- `stdlib/core/remote.mdx:41`: rewrite from illustrative `worker:9527` prose to
  a concrete loopback placeholder example, or leave as prose and rely on the
  more precise rows above.

## Rows to keep disabled/prose

Keep these disabled even after plain loopback serve fixtures exist:

- `advanced/polyglot-distributed.mdx:149`: requires extension discovery and a
  resume phase. The existing Rust tests skip cleanly when `.so` files are not
  present; the book gate should not make extension availability a global
  requirement in the first lane.
- `advanced/polyglot-distributed.mdx:212`: this is the real
  distributed + snapshot + polyglot composition proof, but it needs receiver
  snapshot-store selection, hash capture, and `shape --resume <hash>`. It
  should be a later `serve-snapshot-resume` fixture, not folded into the first
  serve fixture.
- `stdlib/core/remote.mdx:106`: depends on Python/NumPy and receiver language
  opt-in. Leave to an extension-fixture lane.
- `stdlib/core/remote.mdx:204`: needs a deterministic unused-port fixture. It
  should not use `127.0.0.1:1`.
- `stdlib/core/transport.mdx:61` and `stdlib/core/transport.mdx:95`: public
  transport builtins exist, but the book snippets are not standalone and do not
  construct valid protocol bytes for `shape serve`. They need a protocol/echo
  fixture or a public Shape-side wire-message builder.
- `advanced/wire-protocol.mdx:90`: conceptual compression example with
  undefined data and protocol bytes; keep prose until a real wire fixture exists.

## Later composition lane

After the plain serve fixture proves stable, the smallest composition-specific
next lane is `fixture=serve-snapshot-resume` for
`advanced/polyglot-distributed.mdx:212`.

Required additions:

- start `shape serve --sandbox none --snapshot-store <receiver-store>`
- run the client snippet with `__BOOK_SERVE_ADDR__`
- capture `REMOTE_C_SNAPSHOT=HASH:<hash>` from stdout
- assert the hash is hex-like
- run `shape --snapshot-store <receiver-store> --mode vm --resume <hash>`
- assert exact resume stdout contains or equals `RESUMED:43`
- keep this VM-authoritative unless a future gate design defines a deterministic
  dual-mode resume comparison

Do not combine this with dynamic Python/TypeScript extension rows at first.
Extern C gives the composition proof without extension availability.

## Future worker ownership

First lane owner should touch only:

- `../shape-web/book/book-site/scripts/extract-shape-snippets.mjs`
- `../shape-web/book/book-site/scripts/run-book-truth-gate.mjs`
- `../shape-web/book/book-site/scripts/MANIFEST_SCHEMA.md`
- `../shape-web/book/book-site/scripts/run-book-truth-gate.test.mjs` or a new
  focused fixture-unit test file for metadata parsing and child cleanup
- `../shape-web/book/book-site/src/content/docs/stdlib/core/remote.mdx`
- `../shape-web/book/book-site/src/content/docs/tooling/execution-server.mdx`
- `../shape-web/book/book-site/src/content/docs/advanced/polyglot-distributed.mdx`

Runtime proof tests to mirror, not necessarily edit:

- `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs`
- `bin/shape-cli/tests/distributed_async_e2e.rs`
- `bin/shape-cli/tests/distributed_extern_c_snapshot_e2e.rs`
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`

No first-lane production-code ownership is expected. In particular,
`bin/shape-cli/src/commands/serve_cmd.rs` already exposes the required plain
loopback server behavior.

## Closeout

Recommended next slice: implement `fixture=serve` with dynamic loopback port
substitution and exact expected outputs, then flip the five first-lane rows
listed above. This creates real book-truth coverage for live `shape serve`
without broadening the gate to dynamic hashes, extensions, TLS, or protocol-byte
fixtures.
