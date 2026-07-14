# Wave 40M: Explicit Async Resource Scope

Date: 2026-07-10

## Decision

Add one explicit, structured construct for suspending cleanup:

```shape
async resource scope(cleanup: bounded(3.seconds)) {
    using session <- mesh.open(placement)?
    using stream <- session.subscribe(topic)?

    await consume(stream)
}
```

`async resource scope` owns both its child tasks and every value introduced by
`using`. On every contained `Completed`, `Failed`, or cooperative `Cancelled`
exit, the evaluator first quiesces children and then visits initialized
resources in reverse acquisition order. Each resource reaches exactly one
terminal state: evidence-backed `Closed` or explicit `Abandoned`. A close
failure, deadline, or cancellation never skips the remaining obligations.

This is stronger than an implicit `async Drop` method and stronger than
`try/catch/finally`:

- ordinary lexical `Drop` remains synchronous and non-failing;
- async cleanup occurs only for a value registered by `using` in an explicit
  async resource scope;
- the compiler generates every exit edge and the evaluator owns the cleanup
  cursor, so no optional source callback is responsible for completeness;
- ordinary cancellation is shielded only while the bounded cleanup driver is
  active; and
- process abort remains outside the promise. Remote durability comes from
  leases, fencing, idempotency, and transactions, not stronger syntax.

No compatibility constraint is useful here. Retire the current implicit
`DropCallAsync` model instead of preserving two meanings of async destruction.
The existing `async scope` should either become this total task/resource scope
or be removed; a weaker cancel-without-join construct must not share its name.

## Why Current Pieces Are Insufficient

Current `async scope` lowering emits only `AsyncScopeEnter`, the body, and
`AsyncScopeExit` (`compiler/expressions/advanced.rs:1012-1049`). Normal exit
cancels tracked tasks without joining them
(`executor/async_ops/mod.rs:977-1010`); abnormal exits can skip the opcode.
Current `DropCallAsync` has no persisted cleanup continuation, shield, or
deadline, and a genuine suspension is contained as a failed drop. Native JIT
does not have matching user-drop landing pads. These mechanisms cannot be
extended piecemeal into a total lifecycle.

The accepted evaluator outcome remains:

```text
Evaluation<R> =
    Completed(R)
  | Suspended(Suspension)
  | Failed(RuntimeFailure)
  | Cancelled(Cancellation)
  | Faulted(EngineFault)
```

`Evaluation<R>` is host/evaluator-only. An async resource scope does not change
the source return type `R` and does not make `Evaluation<R>` a Shape type.

## Source And Trait Surface

The canonical form is a scope, not a destructor attached to every local:

```shape
async fn export(destination: Placement<Archive>, rows: Array<Row>) -> Receipt {
    async resource scope(cleanup: bounded(5.seconds)) {
        using upload <- archive.open_upload(destination)?
        await upload.write(rows)?
        await upload.finish()
    }
}
```

`using x <- expression` awaits acquisition, atomically registers the resulting
affine owner, and only then exposes `x`. A single-resource
`async using x <- expression { body }` may be syntax sugar for the scope above.
An ordinary `let` cannot own a type with a pending async obligation. Such a
value must be consumed immediately or installed in a resource scope.

The semantic protocol is:

```text
trait DropSafe {
    fn drop(self: own) -> Unit
        where effects = NoSuspend + NoFail + NoReentry
}

affine trait ScopedAsyncDrop: DropSafe {
    type Completion: DropSafe
    type Incomplete: DropSafe

    async fn close(
        resource: &mut Retired<Self>,
        context: &CloseContext,
    ) -> CloseOutcome<Self::Completion, Self::Incomplete>
        where effects = CleanupOnly
}

enum CloseOutcome<C, I> {
    Closed(C),
    Incomplete(I),
}

struct CloseContext {
    reason: CloseReason,
    deadline: Deadline,
    deadline_cancellation: CleanupCancellation,
    events: CleanupEventSink,
}

enum CloseReason {
    Completed,
    Failed,
    Cancelled,
    ExplicitClose,
}
```

`Retired<R>` is an evaluator-owned affine token. Before invoking `close`, the
scope irreversibly revokes all source access to `R`; `close` borrows the token,
so a returned failure, cancellation, or contained close fault cannot lose its
owner. After `Closed`, the runtime records the completion evidence and performs
the synchronous structural drop. After `Incomplete`, close failure, or deadline
expiry, it records abandonment and performs the same structural drop.

`CleanupOnly` may suspend and perform the resource's bounded protocol. It may
not invoke the scope body or annotation continuation, detach work, acquire a
new obligation into the closing scope, extend the deadline, or move the retired
owner. Native/provider implementations must obey cancellation and deadline
conformance; non-cooperative in-process code is not eligible for this trait.

There is no default no-op `ScopedAsyncDrop`. A type with no async close uses
ordinary `Drop`; a type with an async obligation must provide both close
outcomes and a `DropSafe` local-release fallback. The fallback only retires
local ownership. It cannot claim that a peer closed, a transaction rolled back,
or an effect did not execute.

Every cleanup budget is finite. At scope exit the runtime creates an absolute
monotonic deadline bounded by the scope budget, every ancestor unwind deadline,
and a host ceiling. There is no `unbounded` spelling. This prevents nested
scopes from extending a root cancellation indefinitely.

## Ownership And Compile-Time Completeness

The compiler gives each scope and resource an unforgeable brand:

```text
affine ResourceScope<'s>
affine ScopedResource<'s, R: ScopedAsyncDrop>
```

`ScopedResource` is not cloneable, returnable, globally storable, capturable by
a detached task, or movable to another scope. Methods receive scoped borrows;
the ledger, not the visible handle, owns `R`. A future extension may permit an
atomic move to an enclosing scope, but absence is safer than a transfer that
can temporarily leave no owner.

Compilation succeeds only when all of these hold:

1. the construct is inside an async callable and has a finite cleanup budget;
2. every `using` result is affine and implements `ScopedAsyncDrop + DropSafe`;
3. acquisition registration dominates every use and happens before the value
   becomes visible, including partial-initialization and `?` paths;
4. no loan survives resource retirement, scope exit, or transfer to an
   uncontained child;
5. every `return`, `break`, `continue`, `Failed`, and cancellation edge enters
   the generated scope epilogue exactly once;
6. all handler and close branches are exhaustive and `CloseOutcome` is
   `must_use`;
7. child work borrowing a scoped resource is structured and cooperatively
   cancellable; foreign or blocking work must be isolated behind a killable
   host boundary; and
8. VM bytecode and JIT MIR carry the same `AsyncResourceScopePlan`, obligation
   sites, and landing edges.

Dynamic acquisition in loops is valid. Registration receives a monotonically
ordered `ResourceId`, so runtime acquisition order, not source variable name or
static block order, determines cleanup. A failed acquisition creates no live
entry; if a provider creates an owner before returning an error, its host
adapter must hand that owner to the pending registration guard first.

## Evaluator State Machine

The evaluator, not source code, owns these states:

```text
ScopeState<R> =
    Open { ledger, children }
  | SuspendedBody { ledger, children, continuation }
  | ExitSelected { primary, ledger, children }
  | Quiescing { primary, ledger, child_cursor, deadline }
  | Closing { primary, ledger, reverse_cursor, deadline, report }
  | SuspendedClose { primary, ledger, reverse_cursor, deadline, report, wait }
  | Terminal { evaluation, report }

ResourceState<R> =
    Registered(R)
  | Retired(Retired<R>)
  | Closing(Retired<R>)
  | Closed(CompletionEvidence)
  | Abandoned(AbandonmentEvidence)
```

The central invariant is:

```text
each initialized ResourceId:
    one owner while nonterminal
    one retirement transition
    one terminal Closed or Abandoned record
    zero uses after retirement
```

An exit runs as follows:

1. Capture the body exit as `Completed`, `Failed`, or `Cancelled`, freeze new
   acquisitions, and latch later cancellation.
2. Revoke child admission, request cancellation of unfinished children, and
   join cooperative children. Child scopes finish before their parent.
3. Walk the ledger in reverse acquisition order. Retire one owner, await its
   close under the effective deadline, record `Closed` or `Abandoned`, then
   continue regardless of ordinary close failure.
4. Structurally release the completed or abandoned owner and all ordinary
   `DropSafe` values in the same precomputed unwind plan.
5. Publish the projected evaluation only after every ledger entry is terminal.

Suspension is nonterminal. Body suspension moves the whole open scope into the
continuation and performs no cleanup. Close suspension persists the reverse
cursor, current retired owner, absolute deadline, cancellation latch, and
partial report. Resume moves the same owner back; it never recreates a resource
or restarts an already completed close.

An affine `Suspension` must be resumed or consumed by
`cancel_and_drain(deadline).await`. Dropping a host continuation without driving
it can perform only synchronous abandonment; it cannot truthfully promise
async close. Shape-owned suspended tasks remain registered with their parent,
whose total cancellation path drives this operation.

A contained body `Faulted` outcome is deliberately outside user async-close
execution. If the ledger remains trustworthy, the containment owner marks each
entry `Abandoned(Faulted)` in reverse order and runs only compiler-known
structural or trusted host release. If the ledger itself may be corrupt, only
the enclosing native/process owner may reclaim it. The total async-close claim
therefore names `Completed`, `Failed`, and cooperative `Cancelled`, rather than
pretending an engine fault is an ordinary unwind.

## Cancellation And Failure Precedence

Ordinary cancellation is latched while `Quiescing` or `Closing` runs. Cleanup
receives a separate token that fires only at the effective hard deadline. The
shield admits no new body work and cannot be extended by a close implementation.
When the deadline fires, the runtime cancels the current close wait, marks its
owner and all unvisited owners `Abandoned`, performs their safe local release in
reverse order, and publishes the terminal evaluation.

This is bounded cleanup, not a claim that every close succeeds. A provider
close that ignores its deadline must run out of process; killing that process
can establish local containment but not peer cleanup.

The projection rule is deterministic:

| Primary body exit | Ordinary cleanup issues | Final evaluation |
|---|---|---|
| `Completed(r)` | none | `Completed(r)` |
| `Completed(r)` | one or more | `Failed(RuntimeFailure::Cleanup(report))` |
| `Failed(f)` | any | `Failed(f.with_suppressed_cleanup(report))` |
| `Cancelled(c)` | any | `Cancelled(c.with_cleanup(report))` |
| any | cleanup invariant/engine fault | `Faulted(CleanupFault { primary, report })` |

The body value `r` remains owned until cleanup completes; if cleanup becomes
the primary failure, it is structurally dropped rather than published. A close
method's ordinary `Failed` is converted to a cleanup issue, the current owner is
abandoned, and later resources still run. A close `Faulted` stops further user
close code but still structurally abandons every remaining ledger entry.

Cleanup failure is never routed back into the same layer that is already
closing. An enclosing annotation layer may receive
`RuntimeFailure::Cleanup` under the accepted failure-hook algebra; explicit
recovery cannot alter the immutable cleanup report or upgrade provider
evidence. There is no automatic cleanup retry. Any retry requires the
resource's typed idempotency/deduplication capability and a new bounded attempt.

Cancellation arriving after a body failure does not erase that primary
failure. Cancellation arriving before a completed result is published remains
latched for the enclosing evaluator. Host lifecycle events always retain both
the primary exit and the ordered cleanup report.

## Nested Scopes And Annotation State

Scopes are strict stacks. For resources `a` then `b`, cleanup is `b`, then `a`.
An inner scope reaches `Terminal` before its enclosing ledger advances. A child
task's resources close before resources borrowed from its parent. Parent
resources are not retired until all cooperative borrowers have joined.

Callable annotations keep the accepted affine `HookState<S: DropSafe>` model.
An annotation that needs async resources declares an explicit layer scope and
receives a branded capability through its existing phase context:

```text
annotation observed(...) resources(cleanup: bounded(2.seconds))

BeforeContext<Sig> {
    target: Callable<Sig>,
    resources: &mut ResourceScope<'layer>,
    ...
}
```

The ledger remains the owner. `HookState` may hold only a branded scoped
capability or inert identity, never the raw provider/session owner. The state
and ledger move together through `Proceed`, `Return`, `Retry`, backoff, and
suspension.
On terminal exit, hooks stop, state access is revoked, `HookState` is dropped
exactly once, and the layer's resource scope is drained before the next outer
phase runs.

Resources acquired for one invocation attempt belong to an inner attempt
scope and close before `FailureDecision::Retry` starts another attempt.
Resources intentionally shared across attempts belong to the annotation layer
scope and survive retry with its original owner. The type checker rejects an
ambiguous handle whose lifetime does not say which policy applies.

For outer annotation `A` and inner annotation `B`, successful order is:

```text
A.before -> B.before -> body -> B.after -> drop(B state)
         -> close(B resources) -> A.after -> drop(A state)
         -> close(A resources)
```

Failure propagation and cancellation use the same inner-to-outer resource
order. Optional final observers run only after their layer is terminal and
cannot own or repair cleanup.

## Provider And Remote Resources

The generic scope knows nothing about addresses, discovery, transport, auth,
codec, or protocol. It accepts an affine resource from any capability provider.
A remote example uses the accepted opaque provider-typed placement:

```shape
async fn fetch<P: RemotingProvider>(
    placement: Placement<P>, key: Key,
) -> Record {
    async resource scope(cleanup: bounded(3.seconds)) {
        using session <- remote::open_session(placement)?
        await session.invoke(fetch_record, key)?
    }
}
```

`P` owns discovery, route/address encoding, transport, authentication, codec,
negotiation, deadline plumbing, cancellation mechanics, and provider telemetry.
The host retains signature validation, attempt identity, execution certainty,
idempotency authority, and cleanup evidence rules. A session completion may
prove peer acknowledgement; an abandoned session reports local release plus
`OutcomeUnknown`, lease status, or another provider-validated fact. Neither the
scope nor a generic hook parses a provider string to infer certainty.

The provider-facing specialization is mechanical rather than a new language
policy:

```text
impl<P: RemotingProvider> ScopedAsyncDrop for RemoteSession<P> {
    type Completion = SessionCloseReport<P>
    type Incomplete = SessionCloseIncomplete<P>

    async fn close(resource: &mut Retired<Self>, context: &CloseContext)
        -> CloseOutcome<Self::Completion, Self::Incomplete>
}
```

The provider supplies the two evidence types and close mechanics. The host
validates their fixed semantic facts and decides only `Closed` versus
`Abandoned`; neither side invents a destination encoding in the generic API.

`@remote<P>(Placement<P>)` stays signature-transparent. Its implementation may
open a per-attempt session in an explicit attempt scope; the annotation API
still returns exactly `R`, while dispatch or cleanup machinery can produce a
non-returning `Failed`. A placement is inert and may live in hook state. A live
session owner may not; a scope-branded session capability may, because its
ledger owner moves with the same annotation lifecycle.

Provider generations and pools use the same protocol at their owning host
scope: stop admission, drain sessions, close by deadline, then report unresolved
owners. Remote leases, fenced epochs, transaction recovery, and deduplication
remain mandatory when cleanup must survive process loss.

## Snapshot And Process Boundaries

In-memory suspension preserves the exact scope owner and cleanup cursor. Durable
snapshot is stricter:

- an open scope containing a live task, session, socket, credential lease, or
  other opaque resource is a named `SnapshotBarrier` by default;
- a close already in progress is always a barrier in the first design;
- snapshot never invokes close as a hidden side effect;
- explicit `quiesce(deadline)` may close resources before capture; and
- a provider-supported durable resource must be consumed into a typed rebind
  token containing provider identity, logical lease/operation identity, epoch,
  and expiry, never a route, address, socket, credential, or runtime pointer.

Restore validates the provider/configuration and creates exactly one fresh
scope registration from that token. Missing or incompatible providers fail
restore; another provider never interprets the payload. `HookState` additionally
requires `SnapshotSafe`, and retry/attempt state must satisfy its own certainty
rules.

No language lifecycle can run after `SIGKILL`, abort, OOM kill, segfault, power
loss, or host disappearance. The OS may reclaim local descriptors. External
cleanup requires leases, fencing, durable transactions, idempotent status
queries, or compensation. This scope guarantees that a healthy contained
evaluator never skips a registered obligation; it does not guarantee that the
process remains available to drive it.

## VM/JIT Contract And Proof Boundary

The compiler should lower the construct once to an
`AsyncResourceScopePlan`: registration sites, affine brands, child ownership,
all terminal landing edges, cleanup budget, and reverse ledger metadata. The VM
stores the live ledger outside the operand stack. JIT code returns every normal
or negative signal to the same evaluator cleanup driver; it must not re-execute
the body or maintain a second close implementation. Pre-execution fallback at a
scope boundary is acceptable during rollout only when it preserves the same
plan and happens before observable effects.

Required proofs are:

1. compiler ownership tests for partial acquisition, move/borrow rejection,
   every CFG exit, reverse dynamic order, finite budgets, and no implicit async
   drop;
2. evaluator fault injection before and after registration, at every close
   suspension, during cancellation, and after each terminal record, proving one
   owner and one `Closed`/`Abandoned` result per `ResourceId`;
3. nested child, annotation, retry, and suspension/resume traces proving exact
   order and state identity;
4. VM/JIT semantic trace equality for completion, runtime failure, cooperative
   cancellation, close failure, timeout, and contained close fault;
5. snapshot refusal for every live phase plus one explicit quiesce/rebind proof;
   and
6. provider fixtures for acknowledged close, dropped acknowledgement,
   cancellation, lease expiry, idempotent release, and process containment.

## Tradeoffs

The design is deliberately less convenient than universal implicit async drop.
Callers must choose a scope and finite budget, async resources cannot escape,
and a normal body result is not published until reverse cleanup finishes.
Affine brands and a runtime ledger add compiler, MIR, JIT, snapshot, and host
adapter work.

Those costs buy visible suspension, bounded cancellation, deterministic order,
single ownership, and one semantic implementation across normal and abnormal
exits. `try/finally` is terser but cannot prove that a callback exists, runs once,
survives its own failure, or resumes at the correct cleanup cursor. Universal
async drop hides awaits on ordinary scope exits and contaminates synchronous
code. Explicit structured scope is the narrowest surface that can honestly make
the requested guarantee.

This scout changed only this report and ran no cargo, just, test, or build
command.

## Changed File

`docs/cluster-audits/wave40-async-drop-design-explicit-scope.md`
