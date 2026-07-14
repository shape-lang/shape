# Wave 33C Real Async Current Gap

Date: 2026-07-10
Scope: static inspection only. No cargo, just, nextest, rustc, build, test, or
book-truth commands were run.

## Prior Wave Truth

AGENTS.md rows inspected: Wave-14C, Wave-15A, Wave-19A, Wave-22C, Wave-23C,
and Wave-24A.

The prior recommendation chain is now partly stale in a good way:

- Wave-14C correctly identified the first bounded lane as live-future snapshot
  refusal.
- Wave-15A closed that lane: snapshot capture rejects unresolved `Future(id)`
  handles from stack slots, module bindings, and closure upvalues, but still lets
  the future be awaited after the failed checkpoint attempt.
- Wave-19A closed ordered `join all` value materialization for homogeneous
  scalar and typed-object carriers, including distributed `remote::call_async`
  fan-in.
- Wave-22C recommended distributed cancellation for `remote::call_async`.
- Wave-23C partially landed call identity, receiver call registry, queued
  cancellation, running/not-preemptible reporting, and `CancelCall` wire
  messages, but caller-side promptness for running remote calls was still not
  proven.
- Wave-24A closed prompt caller-side cancellation for the plaintext TCP
  `remote::call_async` path by running it on the shared Tokio runtime with an
  abort handle. The row explicitly leaves TLS `remote::call_async` on the old
  blocking worker path.

## Working Now

`Future<T>` is a real type surface for async handles. `remote::call_async` is
typed as `Future<Result<R, RemoteError>>`, and plain async-let handles can be
annotated as `Future<int>` in the current async ShapeTest fixtures
(`tools/shape-test/tests/async_concurrency/future_handles.rs`).

`await` is real over `Future(id)` handles. `op_await` detects
`HeapKind::Future`, resolves pending background tasks through
`resolve_pending_async_task`, and otherwise falls back to the spawned-task
resolver (`crates/shape-vm/src/executor/async_ops/mod.rs:283-353`).

Native async module futures run concurrently when they are self-contained
`Send + 'static` futures. The VM owns a process-wide multi-threaded Tokio
runtime, and async module futures are spawned there while the interpreter
receives completion over an `mpsc` channel
(`crates/shape-vm/src/executor/async_runtime.rs:1-59`,
`crates/shape-vm/src/executor/task_scheduler.rs:61-82`).

Zero-argument user `async fn` calls with scalar returns can overlap through the
isolated-VM path. The comments are honest about the boundary: the normal
interpreter cannot suspend a mid-flight Shape frame, so this path runs a fresh VM
on a blocking worker and only marshals leaf scalar/unit returns back
(`crates/shape-vm/src/executor/async_runtime.rs:61-148`,
`crates/shape-vm/src/executor/async_ops/mod.rs:443-497`). The async-let timing
fixture covers two zero-arg scalar async functions overlapping
(`tools/shape-test/tests/async_concurrency/async_let.rs:157-229`).

`join all` now materializes ordered values for homogeneous supported carriers.
The runtime resolves child futures in source order and returns a typed array for
`int`, `number`, `bool`, and typed-object carriers; mixed or unsupported
carriers surface clear errors instead of returning the old TaskGroup placeholder
(`crates/shape-vm/src/executor/async_ops/mod.rs:644-658`,
`crates/shape-vm/src/executor/async_ops/mod.rs:760-815`). The local tests assert
indexable arrays and the distributed e2e asserts two `remote::call_async`
branches materialize ordered `Result<int>` values
(`tools/shape-test/tests/async_concurrency/join_strategies.rs:111-182`,
`bin/shape-cli/tests/distributed_async_e2e.rs:142-172`).

`join race`, `join any`, and `async scope` have real cancellation semantics for
local scheduler entries. Race/any poll in-flight background tasks for first
settlement/success and cancel non-winners; scope exit cancels pending children in
LIFO order (`crates/shape-vm/src/executor/async_ops/mod.rs:659-697`,
`crates/shape-vm/src/executor/async_ops/mod.rs:879-1008`). Scheduler cancellation
runs pending async cancellation hooks and aborts Tokio tasks when an abort handle
exists (`crates/shape-vm/src/executor/task_scheduler.rs:289-310`).

`remote::call_async` is a real distributed caller-side future. The compiler
rewrites it to `__call_async_result`, types it as
`Future<Result<R, RemoteError>>`, and the native body registers a future and
returns `TypedReturn::Future(task_id)`
(`crates/shape-vm/src/compiler/expressions/function_calls.rs:6209-6420`,
`crates/shape-runtime/stdlib-src/core/remote.shape:110-115`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1769-1884`).
The distributed e2e covers success, transport failure as the inner `Err`,
receiver-side snapshot, manual composition, `join all` fan-in, and live-future
snapshot refusal followed by a successful await
(`bin/shape-cli/tests/distributed_async_e2e.rs:6-223`).

Distributed cancellation now exists for plaintext TCP. A sender-assigned
`RemoteCallId` is carried in `RemoteCallRequest`, and `CancelCall` /
`CancelCallResponse` are first-class wire messages
(`crates/shape-vm/src/remote.rs:56-163`,
`crates/shape-vm/src/remote.rs:330-342`). The serve process tracks queued,
running, and finished call ids. Queued calls can be cancelled before receiver
execution; already-running VM frames report `AlreadyRunning` with an explicit
"not preemptible" message (`bin/shape-cli/src/commands/serve_cmd.rs:193-279`,
`bin/shape-cli/src/commands/serve_cmd.rs:640-719`,
`bin/shape-cli/src/commands/serve_cmd.rs:1216-1256`). The TCP sender path uses a
Tokio `TcpStream` future with an abort handle, so caller-side scope/race
cancellation can return promptly (`crates/shape-vm/src/executor/builtins/remote_builtins.rs:919-981`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1234-1387`).

Live-future snapshot refusal is intentional and precise. The snapshot guard
consults `TaskScheduler::future_snapshot_status` and rejects `HeapKind::Future`
slots with `FUTURE_SNAPSHOT_BARRIER` and "resumable futures are not implemented
yet" (`crates/shape-vm/src/executor/snapshot.rs:131-151`,
`crates/shape-vm/src/executor/snapshot.rs:184-220`).

JIT behavior is honest fallback, not async lowering. Async opcodes are marked
VM-only in JIT preflight (`Await`, `SpawnTask`, `JoinInit`, `JoinAwait`,
`CancelTask`, `AsyncScopeEnter`, `AsyncScopeExit`, etc.), and the JIT executor
routes unsupported programs through the documented `[jit-fallback]` interpreter
path (`crates/shape-jit/src/compiler/accessors.rs:245-264`,
`crates/shape-jit/src/compiler/accessors.rs:592-610`,
`crates/shape-jit/src/executor.rs:51-62`).

## Missing Now

Native async signatures are still not context-aware. Current native async support
works for self-contained `Send + 'static` futures, but it does not model a native
async function that borrows `ModuleContext` or permission/schema state across an
await point. That keeps async stdlib expansion, especially permission-sensitive
I/O, constrained.

User async continuations are still not general coroutines. The isolated-VM path
is useful but bounded to zero-argument scalar/unit returns and no shared heap.
Argument-bearing user async functions, shared-heap returns, captured state that
must preserve caller semantics, module-global-dependent bodies, and true
mid-frame suspension/resumption remain missing
(`crates/shape-vm/src/executor/async_runtime.rs:85-101`,
`tools/shape-test/tests/async_concurrency/async_let.rs:102-130`).

Remote functions returning `Future<T>` are deliberately refused. The compiler
rejects `remote::call_async` callees whose declared return is already a future,
because Shape does not yet have durable cross-node future identity, remote
poll/await, or remote cancellation for a callee-created future
(`crates/shape-vm/src/compiler/expressions/function_calls.rs:6282-6294`,
`crates/shape-vm/src/compiler/expressions/function_calls.rs:6423-6430`).

Pending-future snapshot/resume is not implemented. The snapshot format has
future-shaped vocabulary elsewhere in the system, but the VM correctly refuses
live scheduler ids because there is no persisted scheduler state, remote future
identity, or continuation state to restore.

Streams and real `for await` are missing. `for await` currently parses and runs
over ordinary collections; there is no `Stream<T>` carrier, async iterator
state, backpressure, producer cancellation, or remote stream session
(`tools/shape-test/tests/async_concurrency/for_await.rs:8-12`,
`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/async.mdx:153-167`).

`join settle` still returns a TaskGroup summary rather than settled result
objects. The current runtime branch still resolves children, drops successful
shares, and pushes a `TaskGroupData { kind: 3 }`; the test expects
`[TaskGroup:Settle(2)]`
(`crates/shape-vm/src/executor/async_ops/mod.rs:698-720`,
`tools/shape-test/tests/async_concurrency/join_strategies.rs:273-289`).

TLS cancellation is still a gap. TLS `remote::call_async` uses the older
blocking worker path with no abort handle. Comments in the implementation state
that the caller-side Future is cancelled immediately, but a TLS socket already
blocked in the worker is not interruptible until the receiver replies or the read
timeout fires. TLS cancellation messages also use a request/response
`wire_roundtrip` instead of the plaintext TCP one-way cancel send
(`crates/shape-vm/src/executor/builtins/remote_builtins.rs:943-965`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1220-1229`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1245-1256`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1420-1524`).

JIT async lowering remains missing. The current JIT contract is explicit
VM-only fallback for async/event opcodes; there is no native lowering for future
handles, task scheduler interaction, join groups, cancellation, or async scopes.

The sibling async book page is stale on `join all`: it still says individual
branch values cannot be unpacked and that `join all` returns a TaskGroup summary
in v0.3.3, which contradicts the current runtime/tests after Wave-19A
(`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/async.mdx:57-73`).
The `join settle` and `for await` caveats remain truthful.

## Next Smallest Distributed Lane

Recommended next lane: TLS `remote::call_async` cancellation parity.

Bounded goal: make TLS `remote::call_async` match the current plaintext TCP
cancellation contract without inventing durable remote futures, pending-future
snapshot/resume, user coroutine suspension, or stream protocols.

Concrete behavior:

- TLS `remote::call_async` should register a pending async task with an abortable
  handle, not a detached blocking worker with `abort: None`.
- Cancelling a TLS remote future through `async scope` exit or `join race` /
  `join any` loser cancellation should release caller resources promptly and
  should not leave a long-lived blocked TLS worker waiting for the receiver or
  read timeout.
- TLS `CancelCall` should be one-way/best-effort like plaintext TCP cancellation,
  not a cancellation hook that can itself block on a response round-trip.
- Receiver semantics should stay honest: queued TLS calls may be skipped before
  VM execution, and already-running receiver VM frames still report
  `AlreadyRunning` / not preemptible. Do not claim mid-frame receiver preemption.

Owned files:

- `crates/shape-vm/src/executor/builtins/remote_builtins.rs`: primary work.
  Split the TLS async path from the blocking `tls_roundtrip` helper, add a
  cancellable/abortable TLS round-trip or async TLS transport, and make TLS
  cancel-send one-way.
- `crates/shape-vm/src/remote.rs`: only if the existing `CancelCall` vocabulary
  needs a TLS-specific proof hook. Prefer no protocol change.
- `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`: add TLS variants
  for scope cancellation, race loser cancellation, queued cancellation, and
  running-call honesty. Keep them ignored/timing-sensitive if they use the same
  supervisor-only lane as the current cancellation proofs.
- `bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs`: helper-only if
  existing TLS serve startup helpers are not enough.
- `crates/shape-vm/Cargo.toml` / `Cargo.lock`: only if the implementation chooses
  an async TLS dependency such as `tokio-rustls`; avoid dependency churn if a
  cancellable nonblocking rustls loop is practical.

First tests:

1. `remote_call_async_tls_scope_cancel_returns_promptly`: start a slow TLS remote
   call inside `async scope`, exit without awaiting it, assert elapsed time is
   materially less than an awaited slow control, and assert the serve log records
   `CancelCall`.
2. `remote_call_async_tls_race_cancels_slow_loser`: race a fast TLS remote call
   against a slow TLS remote call, assert the fast result returns promptly, and
   assert the slow loser sends cancellation.
3. `remote_call_async_tls_cancel_queued_call_is_receiver_visible`: with receiver
   concurrency saturated, cancel a queued TLS call and assert the receiver reports
   `AcceptedQueued` before execution.
4. `remote_call_async_tls_cancel_running_call_is_honest`: cancel after the
   receiver VM frame is already running and assert `AlreadyRunning` plus the
   not-preemptible message.

Why this lane is next: Waves 19, 23, and 24 already delivered the small TCP
distributed async wins: fan-in values and caller-side cancellation. TLS parity is
the smallest remaining distributed-computing improvement because it reuses the
existing `RemoteCallId`, `CancelCall`, scheduler hook, serve registry, and proof
shape. The larger gaps are important, but each requires a new semantic object:
cross-node future identity, persisted scheduler/continuation state, stream
sessions, or JIT task lowering.

## Larger Follow-On Order

After TLS cancellation parity, the next genuinely distributed feature is remote
future identity for callees that return `Future<T>`. That should be designed as a
separate protocol lane with explicit remote future creation, await/poll,
cancellation, disconnect/error mapping, and lifetime cleanup.

Pending-future snapshot/resume should wait until either local continuation state
or remote future identity exists; otherwise the system would only persist an
unrestorable scheduler id.

Native async signature expansion and user async continuations matter for local
real async, but they are not the smallest distributed-computing lane.

JIT async lowering should remain behind interpreter semantics. The current
fallback is honest and safer than a partial JIT async path that cannot preserve
kinded future/task-group behavior.

## Closeout

Files changed by this scout:

- `docs/cluster-audits/wave33-real-async-current-gap.md`
