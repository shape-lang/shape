# Wave 40N: Two-Phase Async Drop Design

Date: 2026-07-10

Scope: interface design over the accepted Wave-40 failure, lifecycle, cleanup,
delivery, and remoting-provider reports plus the current ownership, async,
executor, and snapshot surfaces. This is a clean-break design, not a claim that
the current runtime implements it.

## Recommendation

Adopt automatic language-level `AsyncDrop` as one **total evaluator lifecycle**
with two mandatory phases:

1. **Synchronous retirement:** revoke source access and move every initialized
   exiting resource into guarded runtime ownership, without failure/suspension.
2. **Awaited close:** under a bounded shield, reach the declared close
   postcondition or release local mechanics with explicit abandonment evidence.

Both sweeps use reverse acquisition/adoption order. Every obligation reaches
exactly one `Closed(evidence)` or `Abandoned(evidence)`. Error, timeout, or
depleted budget cannot skip later entries: an unstarted graceful close records
`CloseNotStarted` and performs emergency release, never successful drop.

This is not `try`/`catch`/`finally`. Source code cannot omit, catch, replace, or
manually sequence the lifecycle callbacks. The compiler builds one ownership
plan, and one evaluator unwind machine consumes it on every contained
`Completed`, `Failed`, and cooperative `Cancelled` exit. VM and JIT must use the
same plan and machine.

No language protocol can run after process abort or host loss. Leases,
transactions, durable deduplication, and host containment remain the only
honest backstops for effects outside the process.

## Alternatives Rejected

Reject a single `async fn drop(self)`: suspension strands the only capability
without an emergency guard. Also reject explicit close plus sync fallback as the
primary contract: forgotten close and abnormal exits remain partial.

## Current Gap

Current cleanup is compiler-inserted control flow. Scope and early-exit lowering
append `DropCall`/`DropCallAsync` opcodes, but arbitrary runtime failure and
exception truncation do not walk those source scopes
(`compiler/helpers.rs:5813-5967,6030-6165`). A genuinely suspending
`drop_async` becomes a contained drop error, not an awaited continuation
(`executor/trait_object_ops.rs:680-875`). Cancellation aborts without joining,
JIT has no equivalent user-cleanup landing path, and snapshots reject reachable
futures because scheduler state is absent (`executor/task_scheduler.rs:289-310`,
`executor/async_ops/mod.rs:977-1010`, `executor/snapshot.rs:132-154`). Another
opcode or `finally` block would retain these holes.

## Semantic Outcome Boundary

Retain the accepted host-only `Evaluation<R>` variants: `Completed`,
`Suspended`, `Failed`, `Cancelled`, and `Faulted`. Cleanup operates between a
terminal primary outcome and host projection:

```text
UnwindResult<R> {
    primary: Completed(R) | Failed(RuntimeFailure) | Cancelled(Cancellation),
    cleanup: CleanupReport,
}
```

`Suspended` retains its ledger. `Faulted` uses the restricted containment path
below. A domain `Result::Err` remains a `Completed` value.

## Ownership State Machine

Each successful resource construction creates one unforgeable `ObligationId`
and one affine owner:

```text
Uninitialized -> Live<T> -> Moved<T>
Live<T> -> Retired<State, Guard>
        -> Closing<State, Guard> <-> CloseSuspended<State, Guard>
        -> Closed<CloseEvidence> | Abandoned<AbandonmentEvidence>
```

Only `Live` is source-usable. `Moved` leaves no obligation at the old location;
all later states are evaluator-only. The invariant is:

```text
entered nonterminal obligation: exactly one owner
move or suspension edge:        owner transferred, never copied or dropped
terminal edge:                  one Closed or Abandoned record
closed cleanup scope:           no live owner and no missing terminal record
```

Registration follows full construction and guard creation. On construction
failure, initialized fields remain separate obligations. A move transfers the
ID and dependencies; cross-scope adoption adds ordering without cloning it.

An `AsyncDrop` type cannot derive `Clone` or enter a shared cell. Explicit
`duplicate()` must create an independent resource and obligation. Borrows cannot
outlive the owner or cross an untracked task boundary.

## Trait And Source Interface

The semantic trait is deliberately split around an opaque retired state:

```text
affine trait AsyncDrop {
    type Retired
    type CloseEvidence: sealed CleanupEvidence

    total sync fn retire(self: own, exit: ExitCause) -> Self::Retired
        effects(NoSuspend, NoFail, NoReentry)

    async fn close(state: &mut Self::Retired, context: CloseContext)
        -> CloseAttempt<Self::CloseEvidence>
        effects(CancelSafe, TrackedTasksOnly)

    total sync fn release(
        state: own Self::Retired,
        disposition: CloseDisposition<Self::CloseEvidence>,
    ) -> LocalReleaseEvidence
        effects(NoSuspend, NoFail, NoReentry)
}

enum CloseAttempt<E> {
    Closed(E),
    Incomplete(CloseFailure),
}

enum CloseDisposition<E> {
    Graceful(E),
    Abandon(CleanupIncomplete),
}
```

`ExitCause` exposes only `Completed`, `Failed`, or `Cancelled`, never `R` or an
owned failure token. `CloseContext` carries the obligation ID, absolute host
deadline, latched cancellation, and scoped services; it cannot extend policy.

`retire` consumes the public resource, revokes normal operations, and leaves
only the cleanup capability. It retains mechanics needed by `close`; retirement
need not close a socket or descriptor prematurely.

`close` receives a loan, not ownership, of the retired state. When its future
terminates or is cooperatively cancelled and joined, the loan ends and the
evaluator can always call `release`. `release` disposes local mechanics on both
success and failure. On an incomplete close it is the emergency-release path;
it must never claim that a peer, transaction, or remote effect was closed.

Until Shape can prove these effects, `retire` and `release` are limited to
compiler-derived code and trusted host/provider implementations. A close that
cannot be cancelled and joined must run behind a killable host boundary.

```shape
impl AsyncDrop for Client {
    type Retired = RetiredClient
    type CloseEvidence = ClientCloseReceipt

    total sync method retire(self: own, exit: ExitCause) -> RetiredClient
        { RetiredClient::revoke_and_take(self) }
    async method close(state: &mut RetiredClient, ctx: CloseContext)
        { state.finish_protocol(ctx) }
    total sync method release(state: own RetiredClient, disposition)
        { state.release_local(disposition) }
}
```

Owning an unresolved `AsyncDrop` value adds the `AsyncCleanup` effect. A sync
scope must move that value out on every path or fails compilation; it cannot
block secretly. `await close(client)` is optional early-close syntax that
consumes the value through the same ledger and two phases. Source cannot call
`retire`, `close`, or `release` independently.

## Cleanup Evidence

```text
CleanupEntry =
    Closed { obligation, evidence: CloseEvidence, local: LocalReleaseEvidence }
  | Abandoned { obligation, evidence: AbandonmentEvidence }

AbandonmentEvidence {
    reason: CloseNotStarted | DeadlineExceeded | CloseFailed |
            CloseCancelled | QuiescenceFailed | ContainedFault,
    phase: CleanupPhase,
    local: Released | ContainedWorkerTerminated | ReleaseUnproven,
    external: DefinitelyInactive | Rejected | OutcomeUnknown,
    operation: Option<OperationId>,
    lease_expires_at: Option<Instant>,
    cause: CleanupCause,
    provenance: EvidenceProvenance,
}
```

Cause, local release, and external certainty are independent. `Released` means
only local mechanics retired. `OutcomeUnknown` cannot be upgraded by a message
string, socket close, cancellation request, or call ID. Stronger evidence must
be authenticated and bound to operation, principal, provider generation, and
target fingerprint; it excludes secrets and physical addresses.

`CloseEvidence` states the declared postcondition: for example local flush and
close, authenticated lease release, or only local session close when no peer
authority is owned. Its sealed trait prevents stronger user-minted claims.

## Total Compiler Plan

The compiler lowers scopes, obligations, moves, dependencies, and terminal
edges to one immutable `CleanupPlan`, not inline drop calls.

Every successful initialization/adoption appends an obligation to the active
scope. Every move, return, closure capture, and suspension transfers that exact
ID. Every contained terminal CFG edge enters `BeginUnwind(plan, primary)`.
There is no direct return, branch escape, exception truncation, JIT negative
signal, or cancellation projection that bypasses the evaluator unwind machine.

Compilation rejects a plan when any initialized owner lacks a terminal transfer
or cleanup edge, when a moved value remains usable, when a loan can outlive
quiescence, when async cleanup escapes a sync context, or when `retire`/`release`
effects are unproven. Dynamic registration has the same runtime ledger and
cannot be represented by nullable callback slots.

VM bytecode and JIT MIR reference the same immutable plan IDs. Native code exits
through a shared evaluator trampoline carrying `Evaluation`, live-owner facts,
and the plan cursor. It does not replay in the interpreter after side effects.
Parity is defined by identical lifecycle traces and evidence, not merely equal
final output.

## Evaluator Algorithm

### 0. Quiesce borrowers

Close the scope to new work, request cancellation of contained children, and
join every child that can hold a resource loan. This is bounded by the cleanup
policy. A non-cooperative in-process task may not borrow an `AsyncDrop` owner;
unsafe blocking work must be contained so the host can terminate and join it.
If neither join nor containment succeeds, record a cleanup invariant fault and
`ReleaseUnproven` rather than race release. Remote work remains independent.

### 1. Retirement sweep

Latch the primary outcome and ordinary cancellation. Traverse live obligations
in exited scopes in reverse acquisition order, calling `retire` without
suspension and installing each state plus guard in the cleanup ledger. After
the sweep, no source operation can use an exited resource; a slow first close
cannot leave older resources source-live.

A violated `total sync` contract is an `EngineFault(CleanupInvariant)`. The host
continues compiler-derived structural retirement/release for the remaining
entries where its metadata is trustworthy; it never fabricates graceful-close
evidence.

### 2. Close sweep

Traverse retired entries in the same reverse order. For each entry:

1. allocate a deadline bounded by the scope ceiling and resource cap;
2. enter a cancellation shield and await `close`;
3. on success, validate evidence and call `release(Graceful(evidence))`;
4. on returned failure, evaluator failure, or deadline, cancel and join the
   close future, then call `release(Abandon(incomplete))`; and
5. append the terminal report before moving to the next entry.

Ordinary cancellation remains latched and is delivered after cleanup. The
shield has a host-enforced absolute deadline and cannot be extended by source,
an annotation, or a provider. If the total budget expires, every remaining
entry still runs emergency release and records `CloseNotStarted`; none
disappears from the report. A close failure never short-circuits later entries.

For resources `a`, then `b`, then `c`, the trace is:

```text
quiesce
retire(c), retire(b), retire(a)
await close(c), release(c)
await close(b), release(b)
await close(a), release(a)
project(primary, cleanup_report)
```

Close bodies run in tracked child cleanup scopes. Resources acquired by a close
body are cleaned before that close attempt terminates. Reentry into the retired
resource, spawning untracked work, or transferring its guard is forbidden.

## Outcome And Failure Precedence

The primary outcome is frozen before retirement:

| Primary | Cleanup report | Final evaluator projection |
|---|---|---|
| `Completed(R)` | every required close is `Closed` | `Completed(R)` |
| `Completed(R)` | any required close is `Abandoned` | `Failed(CleanupFailureBundle)`; computed `R` is disposed, not returned |
| `Failed(primary)` | any report | preserve `primary`; attach ordered cleanup failures as suppressed evidence |
| `Cancelled(primary)` | any report | preserve cancellation; attach ordered cleanup failures and abandonment |

Owned obligations inside a computed `R` remain evaluator-held until cleanup
succeeds. If success becomes cleanup failure, those obligations rejoin unwind
instead of transferring to the caller.

A cleanup failure cannot replace an existing failure or cancellation, and an
observer cannot replace either. Multiple cleanup failures are accumulated in
cleanup order. A failed close is not recursively handled by the same resource
scope. Only after that scope reaches `Closed`/`Abandoned` may an outer total
annotation policy observe the composed failure.

Best-effort work does not implement `AsyncDrop`. If cleanup must not affect a
successful result, it is telemetry or a separately supervised service with an
explicit ownership transfer. `detach(resource, supervisor)` succeeds only when
the supervisor durably accepts the obligation and returns a transfer receipt;
discarding a future is not detachment.

## Suspension, Cancellation, And Snapshot

Body suspension stores the live ownership ledger in the continuation without
running cleanup. Cleanup suspension stores the frozen primary outcome, retired
states, guards, current close cursor, accumulated evidence, shield state, and
absolute deadline. In-memory resume restores the same owners and continues at
the next cleanup transition. Cancelling or abandoning a continuation invokes
the unwind driver; dropping raw future bits is not cleanup.

The first executable version must make every live `AsyncDrop` obligation,
borrower, retired state, or in-progress close a snapshot barrier. Current VM
snapshots do not persist scheduler state, and `state.capture_all()` is metadata,
not a lifecycle checkpoint. Snapshot refusal names the obligation and phase.

A later opt-in snapshot contract may persist only a logical, versioned close
operation with one owner, stable provider/type identity, idempotency semantics,
and revalidation on restore. It never serializes a socket, task, credential,
route, provider pointer, close future, or emergency guard. If these invariants
are unavailable, callers must `quiesce(deadline)` before capture.

## Annotation Composition

Callable annotations use the same lifecycle, not `finally`. Async resources in
annotation state register ordinary obligations. Body/attempt resources close
before `after` or `on_failure`; handler resources close before handler return;
nested layers unwind in reverse entry order. Cancellation bypasses recovery but
still runs cleanup.

An outer recovery annotation receives a read-only `CleanupReport`. It may retry
or recover only after every prior attempt obligation is terminal.
`OutcomeUnknown` or an abandoned external resource requires an explicit dedup lease,
transaction status, or lease/fencing gate. Generic recovery cannot erase
abandonment evidence or infer retry safety from a cleanup error string.

A built-in declarative annotation may narrow cleanup budgets:

```shape
@cleanup(deadline: 2.seconds, per_obligation: 500.milliseconds)
@recover(resilient_lookup)
@remote(on: placements.analytics)
async fn lookup(key: Key) -> Record { ... }
```

It compiles to constants in `CleanupPlan`; it is not a hook, cannot extend the
host ceiling, and cannot select `ignore`. Transparent `@remote` keeps its source
signature. Annotation authors cannot define `finally`, call retirement methods,
or intercept the emergency guard.

## Provider And Remote Resources

An opaque `RemoteDestination<P>` or `Placement<P>` is inert configuration, not
a live resource and not an address string. A call-owned `RemoteSession<P>`,
lease-renewal owner, transaction participant, or provider generation pin is an
affine resource and uses the two-phase protocol.

For a session, `retire` revokes exchange and moves the generation pin, session,
and attempt recorder into a host carrier. `close` uses provider mechanics under
host deadline, cancellation, auth, protocol, codec, and observability services;
the host validates acknowledgement and certainty. `release` closes local state
and unpins code only after close terminates or its contained worker is killed.

Connection pooling is an explicit affine transfer from the call to the provider
pool. The call scope does not drop a session the pool owns; provider shutdown
later unwinds the pool ledger and reports unresolved sessions. Reload keeps an
old provider generation pinned until every obligation is closed or abandoned.

`RemoteDispatch` completes call-owned cleanup before projection. Transparent
`@remote` returns `R` only after required cleanup; otherwise it produces
non-returning `Failed(RemoteCleanup)`, never fallback or transparent retry.
`remote::call` may project the same facts into typed `RemoteError`.
`remote::call_async` cancellation remains local plus best-effort remote request
until authenticated evidence proves more.

Remote authority needs a process-loss protocol in addition to async drop. Use a
lease with expiry/fencing for locks and reservations, durable deduplication for
safe repeat, and transactions for atomic effects. Local session release does
not roll back a call, cancel running receiver code, or prove a remote lock was
released.

## Fault And Process-Loss Boundary

For contained `EngineFault`, do not run arbitrary Shape close code in a damaged
VM. Native containment uses trusted ledger metadata for safe host release and
records `Abandoned(ContainedFault)`. An isolated cleanup executor may await only
validated retired state independent of that VM.

After `SIGKILL`, abort, segfault, OOM kill, power loss, or host disappearance,
neither phase is guaranteed to run and the dead process cannot emit truthful
abandonment evidence. The OS may reclaim process-local descriptors. A supervisor
may observe worker death and kill a contained child. Neither action proves
remote non-execution, peer acknowledgement, child-process termination, flush,
rollback, or external release. Only durable service records, leases,
transactions, and idempotent status queries can resolve those states.

## Required Invariants And Misuse Prevention

1. Every successful initialization/adoption registers one obligation before
   source use; partial initialization registers only completed fields.
2. Ownership is affine across locals, fields, closures, tasks, annotations,
   suspension, return, and provider transfer.
3. Every contained `Completed`, `Failed`, and cooperative `Cancelled` edge enters
   the same evaluator unwind machine.
4. Retirement covers every live obligation before any close suspension; close
   and release preserve reverse acquisition order.
5. Every obligation records exactly one `Closed` or `Abandoned` terminal entry,
   including close-not-started deadline cases.
6. `retire` and `release` are non-suspending, non-failing, non-reentrant, and
   available to trusted containment without executing arbitrary cleanup code.
7. Async close is tracked, cancellation-safe, bounded, and joined before its
   retired state is released. Non-cooperative work is process-contained.
8. Primary failure/cancellation is never replaced; cleanup failures are ordered
   evidence, and cleanup cannot fabricate `R`.
9. Providers cannot mint execution certainty, peer acknowledgement, or stronger
   evidence than authenticated host validation permits.
10. VM/JIT, normal/failure/cancellation, and in-memory suspend/resume consume the
    same plan and produce the same lifecycle trace.
11. Snapshots refuse live cleanup state unless a separate stable logical
    checkpoint contract proves one owner and replay safety.
12. No API claims destructor execution, close acknowledgement, rollback, or
    abandonment recording after process loss.

## Examples

Nested resources close in reverse order on failure without source `finally`:

```shape
async fn copy(source: Source, sink: Sink) -> CopyStats {
    let connection = await Client::connect(source)
    let input = await connection.open_stream()
    let output = await sink.create_stream()
    await transfer(input, output) // Failed still enters total unwind
}

// quiesce; retire output, input, connection
// await close(output), await close(input), await close(connection)
```

Early close uses the same state machine and consumes ownership:

```shape
async fn publish(batch: Batch) -> Receipt {
    let transaction = await begin_transaction()
    let receipt = await transaction.commit(batch)
    await close(transaction) // same retire/close/release protocol
    receipt
}
```

If commit acknowledgement is lost, `close(transaction)` cannot relabel the
transaction as rolled back. Its evidence carries `OutcomeUnknown`, operation
identity, and any transaction-status or lease recovery handle.

## Tradeoffs

Costs are real: ownership adds `AsyncCleanup`; scope exit may wait; the evaluator
retains retired states, guards, evidence, and an unwind cursor; providers need
cancellation-safe close/release; and JIT exits rejoin a shared host machine.

The two-sweep order can keep cleanup-only parent mechanics alive after source
retirement, so providers must separate public capability from close capability.
Deadlines can still produce abandonment, and a depleted budget can prevent a
graceful close attempt. Those limitations are visible rather than hidden.

In return, no initialized affine resource silently falls off a contained
success, failure, or cooperative-cancellation edge: it closes with evidence or
retires locally with explicit abandonment.

## Proof Boundary

Compiler proofs cover partial initialization, moves, returns, closures, handler
state, retry, every exit, sync rejection, ordering, and plan completeness.

Evaluator fault injection covers every state transition and asserts two-sweep
order, one terminal record per obligation, continued cleanup, primary outcome
precedence, shield expiry, and no unjoined borrower/close future.

VM/JIT proofs compare lifecycle traces. Snapshot proofs either round-trip one
logical owner under a future explicit contract or refuse before persistence.
Provider fixtures cover acknowledged close, dropped acknowledgement,
outcome-unknown release, pool transfer, reload pinning, lease expiry, and
contained non-cooperative close. Process fixtures prove only supervisor/OS
containment and durable external recovery, never in-process destructor
execution after death.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited, and no cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-async-drop-design-two-phase.md`
