# Wave 18C Real Async / Distributed Futures Next Lane

Date: 2026-07-09
Scope: static inspection only. No cargo, just, rustc, build, test, benchmark,
or book-truth commands were run.

## Current Working Surface

Async module futures have real background execution. The VM owns a process-wide
multi-threaded Tokio runtime, and module futures that are `Send + 'static` run
there while the interpreter receives completion over an `mpsc` channel
(`crates/shape-vm/src/executor/async_runtime.rs:1-59`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:610-645`).

`Future(id)` handles are first-class runtime carriers. `await` checks whether a
future is a pending background task and blocks on/project-caches it; otherwise it
falls back to synchronous spawned-task resolution
(`crates/shape-vm/src/executor/async_ops/mod.rs:280-349`,
`crates/shape-vm/src/executor/vm_impl/modules.rs:647-735`).

`async let` has two useful paths today. Async module calls pass through their
already-running `Future(id)` handles. Zero-argument user `async fn` calls with
declared scalar returns can be deferred into isolated VMs on the blocking pool;
argument-bearing calls and heap-shaped returns stay on the eager path
(`crates/shape-vm/src/compiler/expressions/advanced.rs:825-1010`,
`crates/shape-vm/src/executor/async_ops/mod.rs:389-541`,
`crates/shape-vm/src/executor/async_runtime.rs:61-148`).

Structured cancellation exists for local scheduler entries. `PendingAsyncTask`
carries an abort handle, `cancel` removes/aborts pending async tasks, and
`async scope` cancels still-pending children on exit
(`crates/shape-vm/src/executor/task_scheduler.rs:61-90`,
`crates/shape-vm/src/executor/async_ops/mod.rs:834-898`).

`join race` and `join any` are value-producing and poll in-flight async tasks for
first completion/success. They cancel non-winners after selecting a winner
(`crates/shape-vm/src/executor/async_ops/mod.rs:660-698`,
`crates/shape-vm/src/executor/async_ops/mod.rs:761-831`).

Distributed async is already more than a typed wrapper. The compiler lowers
`remote::call_async` to `__call_async_result`, types it as
`Future<Result<R, RemoteError>>`, and refuses remote callees that themselves
return `Future<T>` (`crates/shape-vm/src/compiler/expressions/function_calls.rs:6152-6375`).
The native body registers a background remote RPC task and returns
`TypedReturn::Future(task_id)` (`crates/shape-vm/src/executor/builtins/remote_builtins.rs:780-815`,
`crates/shape-vm/src/executor/builtins/remote_builtins.rs:1329-1444`).

The distributed async e2e file covers: awaiting one remote async call, transport
failure as the inner `Err`, receiver-side snapshot returning a hash, two remote
async calls composed by manual awaits, and snapshot-time rejection of a live
remote future followed by a successful await
(`bin/shape-cli/tests/distributed_async_e2e.rs:6-191`).

Snapshot safety for futures is explicit. Snapshot capture rejects live
`Future(id)` handles on stack/module bindings with a diagnostic telling the user
to await/cancel/move the snapshot point because resumable futures are not
implemented (`crates/shape-vm/src/executor/snapshot.rs:131-220`). The scheduler
has the status API that powers this refusal
(`crates/shape-vm/src/executor/task_scheduler.rs:181-203`).

Remote capture refuses futures/task groups rather than pretending they have
cross-node identity (`crates/shape-vm/src/remote.rs:1328-1345`,
`crates/shape-vm/src/remote.rs:1468-1485`).

Prior audits agree that the first snapshot-safety lane is closed and leave real
async gaps around native async signatures, continuation scheduling, join value
materialization, distributed cancellation, remote futures, pending-future
snapshot/resume, and JIT lowering
(`docs/cluster-audits/wave14-current-completeness-snapshot.md:212-232`).

## Missing For Real Async

- Native async module signatures are still constrained to self-contained
  `Send + 'static` futures. Anything that needs to borrow `ModuleContext` across
  await points is not modeled yet.
- User async continuations are not real coroutines. Apart from the zero-arg
  scalar isolated-VM path, user async bodies still run eagerly or synchronously,
  and `VMError::Suspended` across spawned callable frames remains out of scope.
- `join all` and `join settle` do not materialize values. They currently resolve
  children, drop successful result shares, and push a `TaskGroup` placeholder
  (`crates/shape-vm/src/executor/async_ops/mod.rs:640-659`,
  `crates/shape-vm/src/executor/async_ops/mod.rs:699-721`). Tests assert those
  placeholders today (`tools/shape-test/tests/async_concurrency/join_strategies.rs:16-43`,
  `tools/shape-test/tests/async_concurrency/join_strategies.rs:113-170`,
  `tools/shape-test/tests/async_concurrency/join_strategies.rs:257-278`).
- Cancellation/error propagation is local. Race/any abort local losers, and
  remote async maps RPC failures into `Result<R, RemoteError>`, but there is no
  remote cancellation protocol and no value-preserving `join all` error story.
- Pending futures cannot be snapshotted or resumed. They are intentionally
  rejected at capture time.
- Remote functions returning futures are explicitly refused at compile time.
- JIT async lowering is not a real async implementation lane yet; async remains
  a VM/interpreter semantics surface with fallback expectations.

## Chosen Next Lane

Implement value materialization for `await join all { ... }`, with
`remote::call_async` as the first distributed proof.

This lane should make `join all` return an actual ordered `Array<T>` (or
`Array<Result<R, RemoteError>>` for remote async calls) for homogeneous,
representable child result kinds instead of returning `[TaskGroup:All(n)]`.
Keep `join settle` as a placeholder in this lane.

Bound the first implementation to:

- homogeneous scalar payloads (`int`, `number`, `bool`) and existing typed-object
  carriers such as canonical `Result` objects;
- ordered results matching source branch order;
- loud refusal for mixed or unsupported element carriers rather than falling
  back to the `TaskGroup` print placeholder.

Owned files:

- Primary runtime: `crates/shape-vm/src/executor/async_ops/mod.rs`.
- Optional extraction helper if needed to keep the large file from growing:
  `crates/shape-vm/src/executor/async_join_values.rs` plus the local module
  declaration.
- Focused tests: `tools/shape-test/tests/async_concurrency/join_strategies.rs`
  and `bin/shape-cli/tests/distributed_async_e2e.rs`.

Avoid owning `remote_builtins.rs` for this lane. The remote async task starts,
completes, and projects through existing `PendingAsyncTask` machinery; the gap
is fan-in materialization after those futures complete.

First tests:

1. Replace or add a local scalar `join all` assertion:
   `let xs = await join all { 1 + 2, 3 + 4 }; print(xs[0] + xs[1])` prints `10`.
2. Keep the existing overlap timing test but update its semantic assertion so
   the result is materialized, not `[TaskGroup:All(2)]`.
3. Add distributed fan-in:
   two `remote::call_async` branches inside one `await join all { ... }`, then
   match `results[0]` and `results[1]` as `Result<int, RemoteError>` and print
   `57`.
4. Add a distributed mixed outcome row if the carrier is immediately practical:
   one dead-address `remote::call_async` and one live call in `join all` should
   yield an array where one element matches `Err(_)` and the other `Ok(v)`.

Exit criteria:

- Existing manual-await distributed async behavior remains valid.
- `join race` and `join any` keep returning a single payload and keep cancelling
  losers.
- `join settle` remains explicitly out of scope rather than half-materialized.
- Unsupported `join all` element shapes produce a clear diagnostic.

## Why This Comes Before The Other Async Gaps

Distributed computing needs fan-out/fan-in before it needs durable remote
coroutines. `remote::call_async` can already start independent remote work; the
missing user-visible semantic is collecting those results as values without
manual sequential awaits.

This lane reuses existing machinery: pending async tasks, remote RPC background
execution, result projection, scheduler caching, and ordered task ids from
`JoinInit`. It does not require a new wire protocol, VM frame suspension model,
snapshot format, or JIT lowering strategy.

Native async signatures and continuation scheduling are deeper because they
challenge the VM's current `!Sync` and no-mid-frame-suspend constraints.
Snapshot/resume of pending futures is deeper because it needs durable future
identity and restoration semantics. Remote futures returned by callees need a
cross-node future identity/protocol design. Distributed cancellation should come
after value materialization because cancellation semantics need to be specified
against a real fan-in operator. JIT lowering should follow once the interpreter
semantics are value-complete.

## Files Changed By This Scout

- `docs/cluster-audits/wave18-real-async-distributed-next-lane.md`
