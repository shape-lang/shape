# Wave 40K: Async And External Cleanup Guarantees

Date: 2026-07-10

## Decision

Shape needs three distinct cleanup mechanisms, not one increasingly powerful
`Drop` hook:

1. **Affine synchronous `Drop`** for deterministic, non-suspending release of
   process-local ownership.
2. **Explicit `AsyncClose`** for cleanup that can wait, fail, or require peer
   acknowledgement. It runs under a bounded cancellation shield and returns a
   typed outcome.
3. **Leases, idempotency/deduplication, and transactions** for remote ownership
   or externally visible effects that must survive caller failure.

Cancellation is a request to stop. It is not destruction, task termination,
remote non-execution, rollback, or cleanup completion. A structured scope may
claim that no cooperative local child remains only after cancellation has been
observed and every child has been joined. A deadline can bound that wait, but
deadline expiry weakens the result to explicit abandonment or outcome unknown.

No in-process API can guarantee that cleanup code runs after `SIGKILL`, abort,
segfault, OOM kill, power loss, or host loss. Local kernel resources may be
reclaimed by the OS; remote resources and effects require a durable protocol.
Unsafe or non-cooperative work needs host containment in a killable process,
not a stronger destructor promise.

## Canonical Terms

- **Destruction**: synchronous consumption of one owned value at a language or
  runtime boundary. It cannot suspend.
- **Cleanup**: any work needed to stop using a resource. It may be fallible,
  asynchronous, remote, or only eventually effective.
- **Cancellation requested**: the owner has signalled intent to stop.
- **Cancellation observed**: the task reached a cancellation point and began
  termination/unwind.
- **Terminated**: task code can no longer execute.
- **Joined**: the owner observed termination and collected its terminal state.
- **Local release**: caller-side memory, handles, sessions, or sockets were
  retired. It says nothing about peer state.
- **Acknowledged close**: the relevant owner confirmed the close operation.
- **Cleanup incomplete**: the runtime stopped waiting before confirmation.
- **Outcome unknown**: an external request may have taken effect, but no
  authoritative terminal evidence was observed.
- **Cancellation shield**: a bounded region that defers ordinary cancellation
  while cleanup runs. It is not immunity from deadlines, engine faults, or
  process termination.
- **Abandonment**: local ownership is retired while external cleanup remains
  unconfirmed. This must be observable, never silently called `closed`.

## Current State

### Synchronous destruction

The compiler emits user `DropCall` instructions at normal lexical exits and
explicit source exits (`return`, `break`, `continue`, and `?` propagation), in
reverse declaration order. The focused Drop tests assert block/function/loop
ordering and exactly-once escape handling
(`crates/shape-vm/src/compiler/helpers.rs:5813-6400`,
`crates/shape-vm/src/executor/tests/auto_drop.rs`). The interpreter invokes the
registered user method and releases the receiver share
(`crates/shape-vm/src/executor/trait_object_ops.rs:680-875`).

This is not yet a general unwind guarantee. An unhandled runtime `VMError` is
returned directly by the dispatch loop. Exception handling and nested-call
unwind truncate typed slots and release refcounts, but do not execute pending
user `Drop` methods (`executor/dispatch.rs:260-310`,
`executor/exceptions/mod.rs:132-175`, `executor/call_convention.rs:977-1025`).
`VirtualMachine::Drop` likewise releases live typed carriers, not source-level
drop bodies (`executor/mod.rs:685-875`). A runtime failure, interrupt, or engine
fault can therefore skip user cleanup even while memory shares are reclaimed.

A failing user drop body is contained: its frames are unwound, a warning is
recorded, later drops continue, and the scope's original return survives
(`trait_object_ops.rs:827-875`, `vm_impl/output.rs:35-72`). This is useful
failure isolation, but it also means `Drop` cannot communicate reliable close
failure to ordinary source control flow.

### Async drop

The compiler distinguishes `DropCallAsync` and rejects an async-only drop in a
sync function (`compiler/statements.rs:7370-7410`). The executor selects
`drop_async`, then drives it with the same call-depth loop used for sync calls
(`trait_object_ops.rs:700-840`). There is no persisted cleanup continuation,
cleanup deadline, or cancellation shield.

An `await` of a registered async module task blocks the interpreter on
`std::sync::mpsc::Receiver::recv` until completion
(`executor/vm_impl/modules.rs:656-680`). A true VM suspension propagates as an
error and is contained as a failed drop. Thus current `DropCallAsync` can block
to completion for supported futures, but it is not a general suspendable,
cancel-safe async destruction protocol. The book claim that async drops are
simply "awaited" is stronger than the implementation.

`compile_async_scope` emits the body, including its lexical drops, before
`AsyncScopeExit`, so supported drop bodies precede child cancellation
(`compiler/expressions/advanced.rs:1012-1049`). There is no bound on that wait.

### Task cancellation

`TaskScheduler::cancel` runs an optional hook, removes the pending entry,
calls `AbortHandle::abort`, and marks the task cancelled; it does not await
termination (`executor/task_scheduler.rs:289-310`). Scope exit and race/any
cancel children in LIFO order and immediately continue
(`executor/async_ops/mod.rs:660-700,977-1010`). The focused scope test explicitly
requires a pending one-second child not to delay scope exit
(`tools/shape-test/tests/async_concurrency/async_scope.rs:126-143`).

Regular Tokio futures are dropped when their abort is observed, which runs
synchronous Rust destructors for state owned by that future. No async cleanup
can run after the future is dropped. Isolated user async functions are launched
with `spawn_blocking` (`async_ops/mod.rs:450-480`); Tokio 1.50 documents that a
started blocking task cannot be aborted. The current handle may therefore be
marked cancelled while the isolated VM keeps executing. `TaskScheduler::Drop`
has the same abort-without-join behavior, and external receivers have no
preemption handle (`task_scheduler.rs:531-561`).

The current strongest honest statement is **prompt local cancellation request**,
not "no task outlives its scope" and not cleanup completion.

### Remote cancellation and effects

`remote::call_async` aborts the caller-side network future and launches a
fire-and-forget `CancelCall`; its response is ignored
(`executor/builtins/remote_builtins.rs:931-975,1042-1055`). The receiver can
suppress queued work, but `Running` is explicitly non-preemptible
(`bin/shape-cli/src/commands/serve_cmd.rs:231-288,640-730`). The real-socket
tests distinguish `AcceptedQueued` from `AlreadyRunning`, while also proving
scope exit returns promptly (`distributed_async_cancellation_e2e.rs:127-348`).

`RemoteCallId` is a cancellation correlation token, not a lease,
idempotency key, transaction ID, or result-replay identity. After possible
submission, cancellation or connection loss can leave the call's outcome
unknown. Remote effects may continue after the local future is gone.

### Resources and FFI

`IoHandleData` shares an `Arc<Mutex<Option<IoResource>>>`; explicit `close()`
removes the resource under the mutex, and final Rust destruction releases an
unclosed OS handle (`crates/shape-value/src/heap_value.rs:335-520`). Because
handles are shared, lexical drop of one alias does not determine physical close
time. A dropped child-process handle also does not promise the child was killed
or waited; those are separate operations.

Plugin wrappers call synchronous vtable `drop` functions from Rust `Drop`, and
copy/free returned buffers through provider-owned free functions
(`shape-runtime/src/plugins/language_runtime.rs:20-40,340-365`). The extension
macro catches Rust panics at vtable shells, but a C segfault, abort, or stuck
call cannot be contained or cancelled in process. Foreign calls and provider
drop callbacks have no asynchronous close acknowledgement or deadline.

Compiled foreign-handle disposal is explicit rather than intrinsic to
`CompiledForeignFunction`: the JIT bridge calls it from its own `Drop`, while
no matching VM-handle disposal call was found. The loader also deliberately
keeps language-runtime libraries mapped for process-lifetime atexit safety
(`shape-jit/src/foreign_bridge.rs:27-42`, `plugins/loader.rs:564-595`).

### Provider sessions and snapshots

Wave-40H proposes provider-owned `RemoteSession` values with async `cancel` and
`close`, plus provider `shutdown(deadline)`. Those signatures currently return
no close/shutdown report, so they cannot distinguish peer acknowledgement,
local-only release, timeout, or unknown outcome. Production still has only a
global `WireTransportProvider`, not this session lifecycle
(`executor/builtins/transport_provider.rs`).

VM snapshots do not persist scheduler state. Every reachable `Future` is a
barrier with its scheduler status; its work must be made quiescent and the
handle consumed before capture (`executor/task_scheduler.rs:205-225`,
`executor/snapshot.rs:132-154`). `IoHandle`, live channels and iterators, and
foreign frames also refuse capture rather than fabricating restored resources
(`shape-runtime/src/snapshot.rs:1875-1910,2325-2360`,
`executor/snapshot.rs:840-890`). Live sessions, cancellation handles, and
in-progress cleanup must follow the same rule.

`state.capture_all()` is a separate metadata surface: it projects frame
metadata and supported module bindings into a typed object, while per-frame
args and executable local IP remain absent. It carries no task registry,
cleanup stack, provider session, or lease-renewal state
(`state_builtins/introspection.rs:795-823`,
`executor/vm_state_snapshot.rs:83-131`). It is neither proof of quiescence nor
a lifecycle checkpoint and must not weaken the snapshot barriers above.

## Strongest Honest Guarantee Matrix

| Boundary | Strongest promise while the runtime remains healthy | Never implied |
|---|---|---|
| Normal lexical exit | Each initialized affine obligation invoked once, in reverse acquisition order, before exit completes | External close succeeded |
| Ordinary runtime failure | Same deterministic unwind only after an evaluator cleanup stack exists | Cleanup after corrupted engine state |
| Cooperative cancellation | Request, observation, LIFO unwind, termination, then join | Preemption at an arbitrary instruction |
| Bounded async close | One close operation is awaited until acknowledgement or deadline | Success when deadline expires |
| Remote cancellation | Authenticated queued acknowledgement can prove non-execution | Running work stopped or effects rolled back |
| Remote lease | Authority eventually expires after renewal stops, within the stated lease assumptions | Immediate revocation during partition |
| Idempotent/dedup call | Safe repeat or duplicate suppression within declared scope/window | Exactly-once arbitrary effects |
| Durable transaction | Commit/abort recovery according to the participant protocol | Atomicity for non-participants |
| Contained engine/FFI fault | Supervisor survives and OS reclaims contained worker resources | In-process user destructors ran |
| Process/host loss | Only OS and durable external protocols act | Any local cleanup code executes |

## Recommended Lifecycle APIs

### Affine `Drop`

Use affine `Drop` when cleanup is process-local, synchronous, bounded, and safe
to perform during unwind:

```text
affine trait Drop {
    fn drop(self: own) -> void
}
```

The compiler registers an obligation only after successful initialization,
moves it with the value, and consumes it before invoking the body so re-entry
cannot double-drop. Obligations unwind in reverse acquisition order on normal
exit, `Failed`, and cooperative `Cancelled`, while VM invariants are intact.
JIT and VM must consume the same cleanup plan.

`Drop` must not await, initiate untracked work, perform unbounded I/O, or be the
only way to report a fallible close. Failures are diagnostics aggregated while
remaining drops continue. Shared resources cannot promise lexical destruction;
use an affine owner plus borrowed views, or an explicit idempotent shared
`close()` state machine.

### Explicit async close

Cleanup that can suspend uses an explicit consuming protocol:

```text
trait AsyncClose {
    type Completion;
    type Incomplete;

    async fn close(self: own, context: CloseContext)
        -> CloseOutcome<Self::Completion, Self::Incomplete>
}

struct CloseContext {
    reason: CloseReason,
    deadline: Deadline,
}

enum CloseOutcome<Completion, Incomplete> {
    Completed(Completion),
    AlreadyCompleted(Completion),
    Incomplete(Incomplete),
}
```

The generic lifecycle contract does not encode remoting. Each resource defines
the evidence that satisfies `Completion` and the local/external/recovery state
carried by `Incomplete`; provider sessions specialize those types below.
An `async using` construct may elaborate to this protocol, but only if its
cleanup result is handled or its API declares an infallible bounded close.
Automatic hidden async `Drop` should not be the public guarantee. During close,
the resource is `Closing` and unusable; completion moves it to `Closed`, while a
deadline-expired incomplete outcome never returns a reusable open handle.

### Cancellation scopes and shields

A structured scope uses two phases: request cancellation for every child, then
join every cooperative child. Cleanup runs from an evaluator-owned stack, not
from arbitrary future destruction. Scope exit may be bounded:

```text
cancel_and_join(deadline) -> ScopeCloseReport {
    terminated: Array<TaskId>,
    abandoned: Array<{ task: TaskId, reason: NonTerminationReason }>,
    remote: Array<RemoteCancellationState>,
}
```

Use a shield only around the minimal close/commit/rollback critical region.
The original cancellation remains latched and is delivered immediately after
cleanup. Every shield has an independent hard deadline; unbounded shields turn
cancellation into a hang. A non-cooperative blocking/foreign task cannot be
made safe by a shield and must run in a killable host process if hard
termination matters.

## Remote And Provider Contract

Provider sessions are affine. Their local `Drop` closes local descriptors and
records abandonment only; it never claims peer cleanup. Strengthen Wave-40H's
SPI to return evidence:

```text
async fn RemoteSession::close(self, CloseContext) -> SessionCloseReport
async fn RemotingProvider::shutdown(deadline) -> ProviderShutdownReport

struct SessionCloseReport {
    completion: SessionCloseCompletion,
    local: LocalReleaseState,
    peer: PeerCloseState,
    lease: Option<LeaseStatus>,
}

enum SessionCloseCompletion {
    Completed,
    DeadlineExceeded,
}

enum PeerCloseState {
    NotSubmitted,
    Rejected(CloseFailure),
    Acknowledged,
    OutcomeUnknown { operation_id: OperationId },
}
```

Provider reload pins old generations until sessions are closed or explicitly
abandoned. Shutdown stops new resolution, cancels pre-submit work, requests
in-flight cancellation, drains until the deadline, closes local sessions, and
reports every unresolved lease/effect. Unauthenticated provider status strings
cannot upgrade certainty; authenticated protocol evidence is authoritative.

Use mechanisms by responsibility:

- **Lease**: remote locks, workers, subscriptions, reservations, and session
  authority that must expire after caller crash. Persist TTL/epoch/owner and
  renew explicitly. A lease does not undo completed effects.
- **Idempotency/deduplication**: retry close, release, cancel, or mutation after
  an unknown reply. Bind key to principal, target, fingerprint, epoch, and
  retention. A correlation ID alone is insufficient.
- **Transaction**: effects requiring atomic commit/rollback. Commit and abort
  need durable participant records and status query after uncertain replies.
- **Compensation**: non-transactional effects needing reversal. It is a new
  fallible effect, not guaranteed rollback.
- **Host containment**: unsafe extensions, C, non-cooperative blocking work,
  or suspect providers. Process kill provides local containment; leases and
  transactions still own external recovery.

## Snapshot And Recovery Rules

A snapshot is legal only when cleanup state is quiescent or represented by a
durable, provider-neutral recovery identity. Never serialize a live future,
thread, socket, provider session, credential, cancellation token, or in-memory
lease renewal task.

Do not remove a cancelled task from the runtime registry until it is joined;
otherwise a snapshot can appear quiescent while detached work still runs.
In-progress `AsyncClose` is a barrier unless its operation ID and state machine
are durably replayable and idempotent. A restorable lease stores logical owner,
epoch, expiry, and provider identity, then revalidates/reopens through the
provider; it never restores a session pointer.

Snapshot should not silently close resources as a side effect. Provide an
explicit `quiesce(deadline) -> QuiesceReport`; capture proceeds only after the
report proves no live obligations, or returns a named `SnapshotBarrier` listing
the blocking task/resource/cleanup state.

## Fault And Crash Boundary

`Failed` and cooperative `Cancelled` may run user cleanup while evaluator
invariants are trustworthy. `Faulted(EngineFault)` must not blindly execute
arbitrary Shape destructors in a potentially corrupted VM. It should preserve
host-owned guards, terminate the contained worker, and report which cleanup
obligations became abandoned. A caught extension or marshal panic is a fault,
not proof that extension cleanup ran.

Process isolation is the only hard boundary for segfaults, aborts, stuck C
calls, and started `spawn_blocking` work. Even isolation guarantees only parent
survival and OS-scoped reclamation. Child processes may continue, remote calls
may complete, files may contain partial writes, and services may retain state.
Lease expiry, transaction recovery, durable outboxes, and idempotent status
queries are the corresponding crash mechanisms.

## Impossible Guarantees And Visible APIs

Never promise:

1. A destructor runs after process or machine loss.
2. Arbitrary in-process native/blocking work can be safely preempted.
3. Cancellation proves a submitted remote call did not execute.
4. A finite async-close deadline guarantees peer acknowledgement.
5. A local drop rolls back externally visible effects.
6. Exactly-once effects without transactional integration at every effect.
7. Immediate lease expiry during partition, clock failure, or lease-service
   outage.
8. Snapshot/resume of opaque live resources by serializing their handles.

APIs make uncertainty visible with `CancelRequested`, `CancelObserved`,
`Terminated`, `Joined`, `CleanupIncomplete`, `OutcomeUnknown`,
`LocalReleased`, `PeerAcknowledged`, `LeaseExpiresAt`, `SnapshotBarrier`, and
`Faulted`. Avoid boolean `cancelled`/`closed` results that collapse those
states.

## Focused Proof Boundary

Static/compiler proofs should cover affine move/consume, partial initialization,
reverse acquisition order, no duplicate cleanup edge, sync-only `Drop`, and
VM/JIT plan equality. Runtime proofs should inject cancellation at every
instruction around acquire/use/close, verify cancel-and-join, deadline expiry,
drop-error aggregation, and no hidden cleanup task.

Deterministic async/provider fixtures should cover cooperative future abort,
started non-preemptible blocking work, close acknowledgement, dropped reply,
lease expiry/renewal loss, duplicate release, transaction status recovery,
provider shutdown with unresolved sessions, and snapshot barriers before and
after quiescence. Process fixtures should separately prove panic containment,
worker abort/segfault isolation, OS-local reclamation, and explicitly surviving
external effects.

## Bottom Line

Deterministic destruction is attainable only at boundaries the running
evaluator controls. Suspending cleanup needs an explicit awaited protocol;
remote cleanup needs durable ownership/effect semantics; engine and process
loss need containment plus recovery protocols. Treating all four as `Drop` or
"cancellation" would make the strongest claims precisely where Shape has the
least control.

This scout changed only this report and ran no cargo, just, test, or build
command.

## Changed File

`docs/cluster-audits/wave40-async-cleanup-guarantees.md`
