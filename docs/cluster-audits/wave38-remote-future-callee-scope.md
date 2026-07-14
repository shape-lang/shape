# Wave 38C Remote-Callee Future Scope

Date: 2026-07-10
Role: Wave-38C real async remote-callee `Future<T>` scope scout

## Scope

Static inspection only. I did not run cargo, just, nextest, rustc, build,
tests, or book-truth commands. I did not edit production code or `AGENTS.md`.

Primary sources inspected:

- `AGENTS.md` resource/build policy
- `crates/shape-vm/src/compiler/expressions/function_calls.rs`
- `crates/shape-vm/src/compiler/expressions/advanced.rs`
- `crates/shape-vm/src/compiler/expressions/misc.rs`
- `crates/shape-vm/src/executor/async_ops/mod.rs`
- `crates/shape-vm/src/executor/call_convention.rs`
- `crates/shape-vm/src/executor/task_scheduler.rs`
- `crates/shape-vm/src/executor/vm_impl/modules.rs`
- `crates/shape-vm/src/executor/async_runtime.rs`
- `crates/shape-vm/src/executor/snapshot.rs`
- `crates/shape-vm/src/executor/builtins/remote_builtins.rs`
- `crates/shape-vm/src/remote.rs`
- `crates/shape-runtime/src/snapshot.rs`
- `crates/shape-runtime/stdlib-src/core/remote.shape`
- `bin/shape-cli/src/commands/serve_cmd.rs`
- `bin/shape-cli/tests/distributed_async_e2e.rs`
- `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`
- `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/stdlib/core/remote.mdx`
- `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/async.mdx`
- prior audits, especially Waves 22, 33, and 36

## Bottom Line

The smallest honest lane is not a durable cross-node future identity. It is a
receiver-side materialization lane:

1. Let a remote callee return a local `Future<T>` handle.
2. Detect that handle at the receiver host boundary.
3. Await/resolve it inside the receiver VM before serializing the response.
4. Send the materialized `T` over the existing one-request/one-response wire
   protocol.

That supports useful source programs without inventing remote polling,
cross-node scheduler ids, or pending-future snapshot/resume. Durable remote
future handles remain a separate, larger protocol lane.

## Current Shape

`remote::call_async` is a caller-side future only. The compiler lowers it to
`__call_async_result` and types it as `Future<Result<R, RemoteError>>`; the
native body starts the network round trip on the shared runtime and returns
`TypedReturn::Future(task_id)` (`function_calls.rs:6209-6430`,
`remote_builtins.rs:1703-1817`, `remote.shape:110-115`).

The compiler explicitly rejects `remote::call_async` when the callee's declared
return type is already `Future<T>` (`function_calls.rs:6282-6294`). Synchronous
`remote::call` does not have the same explicit guard, but it would currently be
typed as `Result<Future<T>, RemoteError>` and then fail at the receiver return
marshal boundary.

The receiver currently executes the callee once and serializes the returned
slot directly (`remote.rs:968-1369`). If that slot is `Ptr(HeapKind::Future)`,
`slot_to_serializable` refuses it with the same rationale used for snapshots:
a future is an inline scheduler id, not a restorable or transferable value
(`snapshot.rs:1796-1803`). The `SerializableVMValue::Future(u64)` enum arm
exists, but the codec intentionally does not write a live future carrier.

The VM already has the necessary local resolution pieces. `op_await` resolves
`Future(id)` by using `resolve_pending_async_task` for in-flight async module
tasks and `resolve_spawned_task` for callable/pre-completed tasks
(`async_ops/mod.rs:283-356`). `join all`, `race`, and `any` reuse the same
blocking/non-blocking task-resolution concepts (`async_ops/mod.rs:640-840`).

Cancellation is already honest for the existing remote call identity, not for
callee-created futures. `RemoteCallId` is explicitly a transport correlation
token, not a remotely awaitable future id (`remote.rs:125-130`). `shape serve`
can cancel queued calls, but once a call is marked running it reports
`AlreadyRunning` and "not preemptible" (`serve_cmd.rs:204-285`,
`serve_cmd.rs:640-719`, `serve_cmd.rs:1237-1256`). A first receiver-side
materialization lane should keep that behavior.

Snapshot semantics are also already clear. Live `Future(id)` handles block VM
snapshot capture with `FUTURE_SNAPSHOT_BARRIER` because scheduler state is not
persisted (`snapshot.rs:132-151`, `task_scheduler.rs:205-225`). Materializing a
callee future before the remote response means no future handle crosses the
wire; pending-future snapshot/resume remains out of scope.

## Ranked Implementation Plan

1. **Materialized remote-callee future payloads**

   Own:
   - `crates/shape-vm/src/compiler/expressions/function_calls.rs`
   - `crates/shape-vm/src/executor/call_convention.rs` or a narrow executor
     helper module
   - `crates/shape-vm/src/remote.rs`
   - `bin/shape-cli/tests/distributed_async_e2e.rs`
   - optionally `crates/shape-runtime/stdlib-src/core/remote.shape` comments
     and the sibling remote book page after code lands

   Compiler work:
   - Replace the `remote::call_async` `Future<T>` rejection with a helper that
     unwraps one `Future<T>` layer in the callee return annotation for remote
     result typing.
   - Apply that unwrapped payload type to both surfaces:
     `remote::call(...) -> Result<T, RemoteError>` and
     `remote::call_async(...) -> Future<Result<T, RemoteError>>`.
   - Keep non-future callees unchanged.

   Runtime work:
   - Add a small `VirtualMachine` helper such as
     `resolve_future_handle_blocking(task_id) -> KindedSlot` inside executor
     territory, reusing the same choice as `op_await`: pending async task first,
     otherwise spawned/cached task.
   - In `run_remote_call`, after `execute_function_by_id_at_host_boundary`,
     if the raw result kind is `Ptr(HeapKind::Future)`, verify the callee's
     declared ABI return kind was also `Ptr(HeapKind::Future)`, resolve the
     local future, and serialize the resolved payload slot.
   - For non-future returns, keep the existing ABI return-kind cross-check and
     `slot_to_serializable` path.
   - Do not add wire messages, remote polling, or a remote future handle.

   First tests:
   - `remote_call_async_callee_returned_future_materializes_payload`: remote
     callee returns `Future<int>` from an inner `async let`; caller awaits
     `remote::call_async` and sees `Ok(42)`.
   - `remote_call_callee_returned_future_materializes_payload`: synchronous
     `remote::call` blocks until the receiver future resolves and returns
     `Ok(42)`.
   - `remote_call_async_join_all_callee_returned_futures`: two remote calls
     whose callees return `Future<int>` still materialize ordered
     `Array<Result<int>>` values through current `join all`.
   - A negative row where the future body errors should map to
     `RemoteError::RuntimeError` through the existing remote failure mapping.

2. **Cancellation precision for receiver-side materialization**

   Own:
   - `bin/shape-cli/src/commands/serve_cmd.rs`
   - `crates/shape-vm/src/remote.rs`
   - `crates/shape-vm/src/executor/task_scheduler.rs`
   - `bin/shape-cli/tests/distributed_async_cancellation_e2e.rs`

   This is not required for the first materialization slice. The existing
   honest behavior is enough: queued calls can be cancelled; running calls,
   including calls currently waiting for a callee-created future, are
   `AlreadyRunning` and not preemptible.

   A later improvement could thread a receiver cancellation token into
   `run_remote_call` and let a callee-future wait abort pending async tasks
   before response. That would change the meaning of "running" and should have
   its own proof rows.

3. **Durable remote future identity**

   Own:
   - `crates/shape-vm/src/remote.rs`
   - `crates/shape-vm/src/executor/builtins/remote_builtins.rs`
   - `crates/shape-runtime/stdlib-src/core/remote.shape`
   - `bin/shape-cli/src/commands/serve_cmd.rs`
   - distributed async/cancellation tests

   This is a larger protocol design, not the first lane. It would need a
   remote future id, await/poll/cancel messages, receiver-side lifetime and
   cleanup, disconnect semantics, error mapping, and an explicit snapshot
   policy. Do not conflate it with materialized response semantics.

4. **Pending-future snapshot/resume**

   Own:
   - `crates/shape-vm/src/executor/task_scheduler.rs`
   - `crates/shape-vm/src/executor/snapshot.rs`
   - `crates/shape-vm/src/executor/resume.rs`
   - `crates/shape-runtime/src/snapshot.rs`

   Leave this blocked. The first lane resolves the receiver's future before
   serialization; it does not make local or remote pending futures restorable.
   Existing caller-side `remote::call_async` live-future snapshot refusal should
   remain unchanged.

5. **Streams, real `for await`, `join settle`, and JIT async**

   These remain separate lanes. `for await` still iterates ordinary collections
   rather than a real stream protocol; `join settle` still returns the
   `TaskGroup` placeholder; JIT async remains honest VM fallback. None should
   be pulled into the remote-callee future materialization slice.

## Book Impact

I did not find a currently disabled book row that exactly waits on remote
callees returning `Future<T>`. After the code lands, add or flip a narrow
`stdlib/core/remote.mdx` snippet showing:

```shape
use std::core::remote
use std::core::time

async fn delayed_value() -> int {
  await time::sleep(10.0)
  42
}

async fn remote_future() -> Future<int> {
  async let value = delayed_value()
  value
}

let result = remote::call("__BOOK_SERVE_ADDR__", remote_future)
match result {
  Ok(v) => print(f"REMOTE_FUTURE={v}")
  Err(_) => print("REMOTE_FUTURE_ERR")
}
```

That row should use the existing `fixture=serve` harness after the code is
ready. Do not use it to claim durable remote future identity.

## Supervisor Verification Commands

Run only under the single global cargo/build/test lane. First focused commands:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 \
  cargo test -p shape-cli --test distributed_async_e2e \
  remote_call_async_callee_returned_future_materializes_payload -- --exact --nocapture
```

```bash
systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 \
  cargo test -p shape-cli --test distributed_async_e2e \
  remote_call_callee_returned_future_materializes_payload -- --exact --nocapture
```

Then run the whole distributed async file:

```bash
systemd-run --user --wait --collect --pipe \
  -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 \
  cargo test -p shape-cli --test distributed_async_e2e -- --test-threads=1
```

If a book row is added, the supervisor should run extraction plus the relevant
book-truth slice using the established book-site cgroup policy.
