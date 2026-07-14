# Wave 22C Real Async Next-Lane Scout

Date: 2026-07-09
Scope: static inspection only. No cargo, just, nextest, rustc, build, test,
benchmark, or book-truth commands were run.

## Current Truth

The VM now has real background execution for native async module functions. The
shared runtime is a process-wide multi-threaded Tokio runtime, and `TypedAsync`
module calls are spawned onto it immediately, returning `Future(id)` while the
interpreter keeps running (`crates/shape-vm/src/executor/async_runtime.rs:1-59`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:608-645`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:931-968`).

The scheduler has a concrete pending-async carrier: a completion channel plus a
Tokio abort handle. `await` resolves pending async tasks through that carrier and
caches the projected result; non-pending futures still use the spawned-task
resolver (`crates/shape-vm/src/executor/task_scheduler.rs:61-80`,
`crates/shape-vm/src/executor/async_ops/mod.rs:283-353`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:647-735`).

`Future<T>` is a real type surface for handles. Inference unwraps proven
`Future<T>` at `await`, `async let` binds `Future<T>`, and `join all` infers
`Array<T>` when all branch payloads match
(`crates/shape-runtime/src/type_system/inference/expressions.rs:2998-3055`,
`tools/shape-test/tests/async_concurrency/future_handles.rs:5-49`).

`join all` value materialization is now closed for homogeneous scalar carriers
and typed-object carriers. The runtime resolves children in source order and
builds a typed array; mixed or unsupported carriers surface a clear diagnostic
(`crates/shape-vm/src/executor/async_ops/mod.rs:644-658`,
`crates/shape-vm/src/executor/async_ops/mod.rs:760-815`,
`tools/shape-test/tests/async_concurrency/join_strategies.rs:111-182`).
The distributed proof row already covers two `remote::call_async` branches in
one `join all`, materializing ordered `Result<int, RemoteError>` values
(`bin/shape-cli/tests/distributed_async_e2e.rs:142-172`).

`join race`, `join any`, and `async scope` have local cancellation semantics.
Race/any cancel non-winners after selection; scope exit cancels pending children
in LIFO order (`crates/shape-vm/src/executor/async_ops/mod.rs:659-697`,
`crates/shape-vm/src/executor/async_ops/mod.rs:879-950`,
`crates/shape-vm/src/executor/async_ops/mod.rs:977-1008`,
`tools/shape-test/tests/async_concurrency/async_scope.rs:125-143`).
There does not appear to be a source-level explicit `cancel` API; current source
entry points are structured scope exit and join loser cancellation.

`remote::call_async` is a real caller-side future. The compiler rewrites it to
`__call_async_result`, types it as `Future<Result<R, RemoteError>>`, and refuses
callees whose declared return is already `Future<T>`
(`crates/shape-vm/src/compiler/expressions/function_calls.rs:5767-5783`,
`crates/shape-vm/src/compiler/expressions/function_calls.rs:6184-6429`).
The native body starts a background remote RPC and returns `TypedReturn::Future`
(`crates/shape-vm/src/executor/builtins/remote_builtins.rs:781-815`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1415-1443`,
`crates/shape-runtime/stdlib-src/core/remote.shape:110-115`).

Remote calls are still one request -> one response. `WireMessage` has `Call` and
`CallResponse`, but no cancel frame or remote-job handle
(`crates/shape-vm/src/remote.rs:277-317`). `RemoteCallRequest` carries function
identity, arguments, schemas, program hash, and blobs, but no per-call id
(`crates/shape-vm/src/remote.rs:50-108`). The serve path handles `Call` by
acquiring the semaphore and then running `handle_call` synchronously for that
connection (`bin/shape-cli/src/commands/serve_cmd.rs:534-556`,
`bin/shape-cli/src/commands/serve_cmd.rs:1011-1051`).

Snapshot capture rejects live futures before persistence. The guard consults
the scheduler status and refuses stack slots, module bindings, and closure
upvalues that contain `HeapKind::Future`
(`crates/shape-vm/src/executor/snapshot.rs:131-150`,
`crates/shape-vm/src/executor/snapshot.rs:184-220`,
`crates/shape-vm/src/executor/snapshot.rs:990-1065`). The snapshot wire format
has a `Future(u64)` arm, but the guard prevents relying on a bare local
scheduler id without persisted scheduler state
(`crates/shape-runtime/src/snapshot.rs:650-678`,
`crates/shape-runtime/src/snapshot.rs:1776-1793`).

`for await` is syntax over an ordinary iterable, not a stream protocol. The loop
compiler emits `Await` on each element when `is_async` is set
(`crates/shape-vm/src/compiler/loops.rs:300-308`,
`crates/shape-vm/src/compiler/loops.rs:511-514`), and the tests explicitly say
there is no real async stream protocol yet
(`tools/shape-test/tests/async_concurrency/for_await.rs:8-12`).

JIT async remains VM-only. Preflight marks async/event opcodes as unsupported
until kinded Future/TaskGroup lowering exists, and the async FFI still contains
strict-typing surfaces such as `jit_join_init` returning `TAG_NULL` and
`jit_cancel_task` being a `todo!`
(`crates/shape-jit/src/compiler/accessors.rs:586-610`,
`crates/shape-jit/src/compiler/accessors.rs:1194-1255`,
`crates/shape-jit/src/ffi/async_ops.rs:111-216`).

Book note: the async book page still says `join all` returns a `TaskGroup`
summary in v0.3.3 (`../shape-web/book/book-site/src/content/docs/fundamentals/async.mdx:55-73`).
That is stale relative to the current VM and e2e tests.

## Recommended First Lane

First implementation lane: distributed cancellation MVP for `remote::call_async`.

Make cancellation of a caller-side `Future<Result<R, RemoteError>>` trigger a
real remote-call cancellation path for work that has not yet entered an
uninterruptible receiver VM frame, and make caller-side cancellation return
promptly for remote loser futures. This is the smallest distributed slice
because it reuses the existing local cancellation entry points: `async scope`
exit and `join race`/`join any` loser cancellation already call
`TaskScheduler::cancel`. The gap is that a remote async task currently has no
remote call identity or cancellation hook.

Strict boundary for the MVP:

- Add a per-call cancellation identity for `remote::call_async` tasks.
- Add a scheduler-side cancellation hook for pending async tasks.
- On cancellation, abort/stop the caller-side wait promptly and send a
  best-effort cancel request keyed by the call identity.
- On the receiver, acknowledge and drop queued/not-yet-started calls by id.
- Do not claim mid-bytecode receiver preemption in this lane. If a Shape VM
  frame is already executing, return/record "already running" or "not
  cancellable yet" honestly.
- Do not expose remote `Future<T>` return values or durable remote future
  handles to Shape code.
- Do not make pending futures snapshot-resumable.

Required tests:

- `remote_call_async_scope_cancel_returns_promptly`: start a slow remote call
  inside `async scope` without awaiting it; scope exit must not wait for the
  slow remote call wall time.
- `remote_call_async_race_cancels_slow_loser`: race a fast remote call against
  a slow remote call; the fast result returns promptly and the slow loser runs
  the remote cancellation hook.
- `remote_call_async_cancel_queued_call_is_receiver_visible`: with receiver
  concurrency saturated, queue a second call, cancel it from the caller, and
  assert the receiver reports/records the queued call as cancelled rather than
  executing its side effect.
- `remote_call_async_cancel_running_call_is_honest`: if the receiver has already
  entered a VM frame, the cancel response must be an explicit "already running /
  not preemptible" outcome, not a false success.

Owned files for the first lane:

- `crates/shape-vm/src/executor/task_scheduler.rs`: extend `PendingAsyncTask`
  with an optional cancellation hook or cancellation metadata.
- `crates/shape-vm/src/executor/async_ops/mod.rs`: keep cancellation call sites
  unchanged if possible; adjust only if hook result/status needs to be surfaced.
- `crates/shape-vm/src/executor/builtins/remote_builtins.rs`: generate and
  attach the remote call cancellation identity/hook when registering
  `remote::call_async`.
- `crates/shape-vm/src/remote.rs`: add the wire message(s), call id type, and
  receiver result vocabulary for cancellation.
- `bin/shape-cli/src/commands/serve_cmd.rs`: dispatch cancellation messages and
  maintain the active/queued call registry for the serve process.
- `bin/shape-cli/tests/distributed_async_e2e.rs` or a new focused
  `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`: e2e proof rows.
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`: optional
  helper-only changes for starting constrained/slow receivers; do not grow the
  capped `distributed_snapshot_polyglot_e2e.rs`.

## Ranked Slices After The First Lane

1. Distributed cancellation MVP, as above.

Why first: it is directly distributed, uses existing cancellation points, and
does not require user coroutine frames, durable future identity, stream
backpressure, or JIT support. It also closes an honesty gap: today local
cancellation can cancel the caller's scheduler entry, but the remote side has no
cancellable job concept.

2. Native async signature honesty and stdlib expansion.

Current native async support is real for `Send + 'static` futures, but the
registration contract intentionally excludes borrowing `ModuleContext` across
await (`crates/shape-runtime/src/marshal.rs:2161-2176`). `time::sleep` is wired
through the variadic async helper (`crates/shape-runtime/src/stdlib_time.rs:64-87`);
HTTP is wired through fixed-arity async helpers; async file I/O is explicitly
deferred (`crates/shape-runtime/src/stdlib_io/async_file_ops.rs:1-22`).

Next tests: `let h: Future<unit> = time::sleep(0.0)` type-checks if the native
async signature is meant to expose a Future handle directly; async file
`read_file_async` / `exists_async` are registered and overlap in `join all`;
permission/schema-sensitive async native functions fail before spawning rather
than borrowing `ModuleContext` across await.

Owned files: `crates/shape-runtime/src/marshal.rs`,
`crates/shape-runtime/src/typed_module_exports.rs`,
`crates/shape-runtime/src/module_exports.rs`,
`crates/shape-runtime/src/stdlib_io/async_file_ops.rs`,
`crates/shape-vm/src/compiler/expressions/function_calls.rs`, and focused
async/native tests.

3. User async continuations.

Current user async functions are not general coroutines. Zero-arg scalar
`async fn` calls can be deferred into isolated VMs, but arg-bearing calls,
heap-shaped returns, captures, and module-global-dependent bodies keep the eager
path (`crates/shape-vm/src/compiler/expressions/advanced.rs:847-999`,
`crates/shape-vm/src/executor/async_runtime.rs:61-148`). The executor comments
still mark mid-frame suspension across spawned callable frames as out of scope
(`crates/shape-vm/src/executor/async_ops/mod.rs:78-84`).

Next tests: two arg-bearing user async calls with `await time::sleep` overlap;
heap return values cross the task boundary; captured values and module globals
keep the caller's semantics; suspension inside a spawned user async frame is not
lost.

Owned files: async call lowering in `crates/shape-vm/src/compiler/expressions`,
executor continuation/call-frame state in `crates/shape-vm/src/executor/**`,
and focused `tools/shape-test/tests/async_concurrency/**` rows.

4. Remote future identity.

The compiler deliberately rejects `remote::call_async` callees that return
`Future<T>` (`crates/shape-vm/src/compiler/expressions/function_calls.rs:6282-6294`).
Supporting this means inventing a cross-node future identity and await/poll
protocol, not just changing a type annotation.

Next tests: a remote callee starts a local async operation and returns a remote
future handle; the caller awaits it later; cancellation of that handle reaches
the receiver; disconnect/error cases map to `RemoteError` variants without
message parsing.

Owned files: `crates/shape-vm/src/remote.rs`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs`,
`crates/shape-runtime/stdlib-src/core/remote.shape`,
`bin/shape-cli/src/commands/serve_cmd.rs`, and distributed async e2e tests.

5. Pending-future snapshot/resume.

Snapshot capture currently refuses all live Future handles with a clear barrier
because a local scheduler id is not enough to restore work
(`crates/shape-vm/src/executor/snapshot.rs:131-150`). This should wait until at
least local continuation state and remote future identity are designed.

Next tests: snapshot with a pending native async task resumes and awaits the
same result; snapshot with a pending remote async call either resumes via a
durable remote identity or refuses with a precise reconnect/cannot-resume
diagnostic; completed futures either snapshot as payloads or are refused
consistently.

Owned files: `crates/shape-vm/src/executor/task_scheduler.rs`,
`crates/shape-vm/src/executor/snapshot.rs`,
`crates/shape-vm/src/executor/resume.rs`,
`crates/shape-runtime/src/snapshot.rs`, and snapshot/resume tests.

6. Streams and real `for await`.

`for await` currently awaits elements from a normal collection; there is no
`Stream<T>` carrier, async iterator state, backpressure, or remote stream
session (`tools/shape-test/tests/async_concurrency/for_await.rs:8-12`). This is
larger than cancellation because it needs a new long-lived protocol and resource
lifetime story.

Next tests: `for await` over a native stream yields values as they arrive;
breaking the loop cancels the producer; remote stream disconnects surface a
typed error; backpressure prevents unbounded buffering.

Owned files: parser/AST only if syntax changes, loop lowering,
`crates/shape-vm/src/executor/async_ops/mod.rs`, stream carrier types in
`shape-value`/`shape-runtime`, remote wire protocol, and async tests.

7. JIT async.

Async opcodes are intentionally VM-only today, and JIT async FFI contains
strict-typing surfaces (`crates/shape-jit/src/compiler/accessors.rs:586-610`,
`crates/shape-jit/src/ffi/async_ops.rs:111-216`). This is a performance/parity
lane after interpreter semantics settle.

Next tests: JIT preflight no longer rejects a minimal `await` program; VM/JIT
parity for `async let`, `join all`, `race`, `any`, and `async scope`; fallback
still triggers for unsupported stream/snapshot async surfaces.

Owned files: `crates/shape-jit/src/compiler/accessors.rs`,
`crates/shape-jit/src/ffi/async_ops.rs`, async opcode lowering/FFI symbol
plumbing, and VM/JIT parity tests.

## Closeout

Recommended first implementation lane: distributed cancellation MVP for
`remote::call_async`, bounded to call identity, caller promptness, queued-call
receiver cancellation, and honest non-preemption once a receiver VM frame is
already running.

Files changed by this scout:

- `docs/cluster-audits/wave22-real-async-next-lane.md`
