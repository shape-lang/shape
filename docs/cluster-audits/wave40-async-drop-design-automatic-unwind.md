# Wave-40L Automatic AsyncDrop And Evaluator Unwind

Date: 2026-07-10

Scope: interface design only, assuming accepted Wave-40 outcome, affine
hook-state, opaque placement, and provider evidence terms. Only this report was
edited; no cargo, just, test, or build command was run.

## Decision

Add automatic language-level `AsyncDrop`, but make its suspension visible in a
callable effect and make the evaluator, not emitted exit snippets, own one total
cleanup ledger.

An initialized affine `AsyncDrop` value registers exactly one obligation. Moving
the value moves that obligation. Every healthy in-process `Completed`, `Failed`,
or cooperative `Cancelled` exit seals the primary outcome, cancels and joins
cooperative children, then visits all still-owned obligations in reverse dynamic
acquisition order. Each obligation ends as completed, incomplete, failed, or
explicitly abandoned; none silently disappears. Cleanup may suspend and resume
under a finite cancellation shield.

This is not `try`/`catch`/`finally`:

- runtime failures do not become catchable source values;
- cleanup is generated from ownership, so no handler can be omitted;
- a cleanup failure never skips later cleanup or recursively enters
  `on_failure`;
- suspension retains the exact ledger owner and cursor; and
- VM and JIT consume one compiler-authored plan.

The guarantee is invocation and accounting, not successful external close.
Deadline expiry records abandonment and runs trusted local retirement glue.
`Faulted(EngineFault)` runs no arbitrary Shape cleanup code unless containment
proves the ledger and interpreter are still trustworthy. Process abort, host
loss, power loss, and non-cooperative native code remain outside the guarantee.

## Why This Shape

Implicit async destruction hides suspension and still misses abnormal exits.
Blocking a sync evaluator can deadlock and cannot bound foreign work. Requiring
only `async using` makes close visible but is not automatic. The selected design
uses an explicit `AsyncCleanup` effect, automatic affine registration, and a
total evaluator ledger; an explicit block only narrows lifetime or deadline.

## Current Gap

Current Shape rejects async-only Drop in a sync function
(`crates/shape-vm/src/compiler/statements.rs:7379-7401`), then drives
`drop_async` with the ordinary nested-call loop
(`crates/shape-vm/src/executor/trait_object_ops.rs:700-840`). A real suspension
escapes as an error (`crates/shape-vm/src/executor/dispatch.rs:581-651`).

Ordinary `VMError` return and exception truncation can bypass user Drop, and the
JIT rejects user-Drop programs because native lowering only releases refcounts
(`docs/cluster-audits/wave40-shape-destruction-unwind-audit.md:131-163`). The
new model must replace those paths rather than extend `DropCallAsync`.

## Source Interface

The spelling below is normative in meaning and illustrative in concrete parser
syntax.

```shape
effect AsyncCleanup

affine trait AsyncDrop {
    type Evidence: CleanupEvidence = Unit
    async cleanup method drop(
        self: own,
        context: &CleanupContext,
    ) -> CleanupDisposition<Self::Evidence>
}

enum CleanupDisposition<E: CleanupEvidence> {
    Complete(E)
    Incomplete(CleanupIncomplete)
}

struct CleanupContext { reason: CleanupReason, deadline: Deadline, token: CleanupToken }
enum CleanupReason { Explicit, Completed, Failed { code }, Cancelled { kind } }
```

`CleanupContext` and `CleanupToken` are evaluator-created, read-only
capabilities. They contain no `R`, mutable `RuntimeFailure`, credential, raw
provider handle, or retry permit. The token lets cleanup-safe host operations
observe the hard deadline; it cannot clear the original cancellation.

`CleanupEvidence` is non-resource-owning, diagnostic-safe, and redaction-aware.
Local resources normally use `Unit`. Provider resources may return typed,
host-validated evidence such as local release, peer acknowledgement, operation
identity, outcome unknown, or lease expiry. Evidence cannot claim more certainty
than the host's attempt recorder proves.

A type implements at most one semantic destructor, `Drop` or `AsyncDrop`.
Every type also has hidden compiler/host **retire glue** that only releases local
storage and exact kinded shares. It cannot suspend, call user code, or claim
external cleanup. Retire glue is used after normal async cleanup and as the
bounded abandonment backstop; it never recursively invokes `AsyncDrop`.

The cleanup method is effect-restricted:

- it may await only cleanup-safe, deadline-aware operations;
- it may not detach tasks, block in-process native code, snapshot, re-enter the
  owner, invoke the primary target, or leak an owned field;
- temporary affine resources remain ledger-tracked inside its cleanup frame;
- during automatic unwind, a `RuntimeFailure` becomes a cleanup record; and
- a panic or invariant violation becomes `EngineFault`, not `Incomplete`.

## Callable And Scope Effects

A function that can own an armed `AsyncDrop` obligation on any CFG edge has the
`AsyncCleanup` effect. Public signatures state it explicitly:

```shape
async cleanup(max: 2.seconds) fn read_batch(placement: Placement<ComputeProvider>)
    -> Batch ! { Suspend, AsyncCleanup }
```

The declared completion type remains `Batch`; `AsyncCleanup` is an effect, not
`Result<Batch, CleanupError>`. Calling the function is awaitable. The optional
`max` narrows the caller/host budget and can never extend it.

An ordinary sync function cannot acquire an `AsyncDrop` owner, receive one as an
owned parameter, or cross a scope edge while owning one. An ordinary `async fn`
may localize the capability:

```shape
let value = await cleanup(deadline: 500.milliseconds) {
    let own session = await provider.open(placement)
    await session.read()
}
```

The block creates a cleanup-capable sub-scope and does not replace automatic
registration. A sync context has no block-on escape hatch. Representative
diagnostics are:

```text
E_ASYNC_DROP_SYNC_EXIT: `Session` may require suspending cleanup here
E_ASYNC_CLEANUP_EFFECT: call requires `await` and an async context
E_ASYNC_CLEANUP_ESCAPE: owned obligation escapes a non-cleanup-capable scope
```

Private closures may have their effect inferred, but exported functions,
trait methods, callable annotations, and higher-order bounds carry it in their
type. Applying an annotation whose state can `AsyncDrop` adds the effect; it
cannot silently preserve a sync callable type.

Explicit early cleanup uses the same protocol and disarms the automatic entry:

```shape
let disposition = await drop session
```

The expression consumes `session` and returns its typed disposition. Code whose
correctness depends on close acknowledgement, commit, or rollback must use this
explicit form and handle `Incomplete`; automatic cleanup is the total fallback,
not a transaction API. An unexpected evaluator failure during explicit drop is
the body's primary `Failed`; the consumed owner is still retired and disarmed.

## Affine Ownership Rules

1. Register only after successful initialization; partial construction registers completed fields independently.
2. Each obligation has an unforgeable ID, owner, plan, and dynamic acquisition sequence.
3. Move, return, capture, collection insertion, task transfer, and hook-state transfer move the same ID; copy/clone reject.
4. Borrowed views do not own cleanup; a borrowing child must join before owner cleanup.
5. Returning an owner provisionally transfers its ID; the caller receives it after callee cleanup.
6. Non-fault cleanup problems stay in evaluator metadata and do not suppress `R`; critical close uses explicit `await drop`.
7. Shared resources need one affine close owner or an explicit idempotent shared close state machine.
8. An owner cycle rejects unless ownership moves into an explicit affine cycle-breaking owner.

Static checking proves identity and transfer. The runtime ledger validates the
same IDs at VM/JIT, suspension, and provider boundaries so malformed metadata
cannot double-run or lose cleanup.

## Compiled Plan And Runtime State

The compiler emits one cleanup plan from typed ownership MIR:

```text
CleanupPlan { effect_fingerprint, regions, register/move/transfer edges,
              async_drop_entries, retire_glue }
CleanupLedgerEntry { id, acquired_sequence, owner,
                     state: Armed | Running | Suspended | Complete
                          | Incomplete | Failed | Abandoned | Retired }
```

The ledger is a dynamic stack because branches, loops, factories, and retries
make acquisition order runtime data. Compiler region facts determine which IDs
remain owned on an edge; dynamic acquisition sequence determines unwind order.

Every contained terminal path enters one evaluator operation:

```text
settle(primary):
    prevent new work; request cancellation and join cooperative children
    fold structured-child outcomes, then seal the primary
    while an owned obligation remains, newest first:
        run/resume AsyncDrop under the inherited shield
        append one cleanup record
        run trusted retire glue
    publish primary plus CleanupAggregate
```

There is no direct return/error/cancel instruction that bypasses `settle`.
Source `return`, `break`, `continue`, and `?` become ordinary edges into the same
region exit. An unhandled runtime failure requests evaluator unwind instead of
returning directly from dispatch. A catch/finally opcode is neither needed nor
allowed to own cleanup correctness.

## Primary And Cleanup Outcomes

During unwind the evaluator carries:

```text
Primary<R> = Completed(R) | Failed(RuntimeFailure) | Cancelled(Cancellation)

CleanupRecord =
    Complete { obligation, evidence }
  | Incomplete { obligation, detail }
  | Failed { obligation, failure }
  | Abandoned { obligation, reason, local_release, external_certainty }

CleanupAggregate = ordered Array<CleanupRecord>
```

`Evaluation<R>` remains evaluator-only and gains cleanup metadata on contained
terminal variants; the familiar shorthand omits an all-complete aggregate:

```text
Completed { value: R, cleanup: CleanupAggregate }
Failed { primary: RuntimeFailure, cleanup: CleanupAggregate }
Cancelled { primary: Cancellation, cleanup: CleanupAggregate }
Suspended(BodyContinuation | CleanupContinuation)
Faulted { fault: EngineFault, interrupted_primary, cleanup }
```

Primary precedence is fixed:

1. Body/annotation `Completed`, `Failed`, or `Cancelled` is sealed before
   cleanup and is never replaced by an ordinary cleanup failure.
2. Cleanup failures and incomplete/abandoned states append in execution order;
   all later obligations still run.
3. Cleanup failures never enter callable `on_failure`, never trigger retry, and
   never fabricate or suppress `R`.
4. An engine fault dominates because evaluator integrity is no longer known;
   `Faulted` retains the interrupted primary and records every obligation that
   could only be structurally retired or abandoned.
5. Host projections must expose non-complete cleanup. A root host may choose a
   nonzero operational status for `Completed(R)` plus incomplete cleanup, but it
   must preserve the primary/cleanup distinction in structured output.

This primary-wins rule matches synchronous Drop error containment and makes
ownership of a returned affine `R` unambiguous. It also makes the tradeoff of
automatic cleanup explicit: source code cannot branch on an automatic cleanup
record; use explicit `await drop` when it must.

## Cancellation Shield And Deadlines

The first cooperative cancellation is latched as the primary outcome. Cleanup
runs in a shield that defers further ordinary cancellation but not its absolute
deadline, instruction/fuel limits, engine faults, or process termination.

The effective deadline is the minimum of host policy, caller budget, enclosing
scope deadline, function `max`, and any resource-specific earlier limit. Nested
cleanup never resets it. Providers can partition remaining time but cannot
extend it.

At deadline:

- the active cleanup coroutine receives deadline cancellation at a required
  safepoint;
- its obligation records `Incomplete` or `Abandoned` with honest certainty;
- every remaining ledger entry is visited and explicitly abandoned rather than
  omitted; and
- trusted non-suspending retire glue drains local ownership.

The deadline bounds waiting and user cleanup execution, not the finite
O(number-of-obligations) trusted ledger drain. AsyncDrop code must be
cooperative and instruction-metered. Blocking/foreign cleanup is rejected
unless it runs behind a killable process boundary; an in-process C call cannot
be made bounded by type syntax.

## Suspension, Resume, And Snapshot

Cleanup suspension stores one affine continuation:

```text
CleanupContinuation<R> {
    sealed_primary,
    ledger,
    reverse_cursor,
    active_obligation,
    cleanup_coroutine_frame,
    absolute_deadline,
    aggregate,
}
```

In-memory resume continues the same coroutine and obligation ID. It never reruns
the body, a completed cleanup prefix, annotation `before`, or remote submission.
Cancellation arriving while suspended is latched; the shield resumes only to
finish or abandon cleanup.

Durable snapshot is fail-closed. Any armed provider/FFI/OS resource or active
cleanup continuation produces a typed `SnapshotBarrier` naming obligation,
state, and remediation. This extends the current Future barrier
(`crates/shape-vm/src/executor/snapshot.rs:132-150`). Snapshot does not silently
close resources.

A future opt-in durable cleanup protocol would need an idempotent recovery key,
provider identity/version, lease epoch/expiry, serializable owner state, and a
proof that resume cannot duplicate close. It may never serialize a socket,
session, credential, cancellation token, raw pointer, or provider object.
Opaque destinations may be restored and rediscovered; live sessions may not.

## Nested Order And Hook State

Dynamic entry/acquisition order is authoritative. For outer layer `A`, inner
layer `B`, and body resources `x` then `y`, success is:

```text
A.before -> B.before -> body
-> cleanup(y) -> cleanup(x)
-> B.after -> cleanup(B.state)
-> A.after -> cleanup(A.state)
```

On body failure, body cleanup completes before `B.on_failure` receives the
primary `RuntimeFailure`. Propagation cleans `B.state` before entering
`A.on_failure`. Recovery rejoins the success path. Cooperative cancellation
bypasses `after` and `on_failure`, joins children, then cleans body, B state, and
A state deepest/newest first.

`HookState<S>` may contain `AsyncDrop` only when the annotation plan has the
`AsyncCleanup` effect. Its obligation moves unchanged through `before`, target
suspension, `on_failure`, backoff, and retry. A retry fully settles the failed
attempt's inner ledger before starting the next attempt; the retrying layer's
state remains armed, while fresh inner layers receive fresh IDs.

Replacing async-droppable hook state is itself an async cleanup operation: old
state is moved to a nested obligation, cleaned, and retired before replacement
commits. Cleanup records are observation data only. They are not delivered to
`on_failure`, cannot authorize unknown-outcome retry, and cannot be relabelled as
target non-execution.

## Provider And Remote Resources

Provider/session mechanics remain below the generic language trait. A provider
may implement:

```shape
impl<P: RemotingProvider> AsyncDrop for RemoteSession<P> {
    type Evidence = SessionCloseEvidence

    async cleanup method drop(self: own, context: &CleanupContext)
        -> CleanupDisposition<SessionCloseEvidence> {
        provider.close(self, context.deadline, context.token).await
    }
}
```

`RemoteSession<P>` is affine. `RemoteDestination<P>` and `Placement<P>` are
opaque typed capabilities, not address strings and not live cleanup owners.
Discovery, routing, destination encoding, transport, auth, codec, protocol
negotiation, deadlines, cancellation, and observability stay behind provider
interfaces (`docs/cluster-audits/wave40-remoting-provider-interface.md`).

The host validates provider evidence and derives local release, peer
acknowledgement, and execution certainty. Outcome unknown remains unknown.
Automatic cleanup never retries an unknown close or effect without an explicit
idempotency/dedup permit. Provider generations are owners acquired before their
sessions, so reverse order closes sessions before provider shutdown.

Process-loss safety still comes from leases, fencing, durable transaction
recovery, and dedup/result replay. AsyncDrop can request and observe close only
while its evaluator remains alive.

## Ergonomic Examples

### Automatic success, failure, and cancellation

```shape
async cleanup(max: 2.seconds) fn load(
    placement: Placement<AnalyticsProvider>,
    key: string,
) -> Batch {
    let own session = await analytics.open(placement)
    let own stream = await session.stream(key)
    return await stream.collect()
}
```

On every contained body return or runtime failure, `stream` cleans before
`session`. Cooperative cancellation first stops and joins child work, then uses
the same order. The call does not return or fail outward until cleanup settles or
is explicitly abandoned at the deadline.

### Explicit close evidence

```shape
async cleanup fn publish(
    placement: Placement<EventProvider>, event: Event,
) -> Result<Receipt, CleanupIncomplete> {
    let own session = await events.open(placement)
    let receipt = await session.publish(event)

    match await drop session {
        Complete(evidence) if evidence.peer_acknowledged => Ok(receipt)
        Complete(_) => Err(CleanupIncomplete::PeerUnconfirmed)
        Incomplete(detail) => Err(detail)
    }
}
```

Explicit close consumes the obligation, so function exit does not run it again.
No provider address or wire encoding appears in the API.

## VM/JIT And Transfer Contract

Bytecode and JIT MIR must reference the same `CleanupPlan` and emit the same
register/move/transfer IDs. JIT runtime failure returns an unwind request to the
common evaluator; it cannot abandon a native frame or rerun the function in the
VM. JIT safepoints yield a `CleanupContinuation` using the same ABI as VM
suspension. Cleanup bodies may be JIT-compiled, but orchestration and the ledger
remain common runtime code.

The callable effect, obligation plan, destructor identity, and retire glue are
semantic compiled facts. They must be hash-covered in transferred function
metadata, and every referenced AsyncDrop implementation must be included in the
transitive content closure. A peer that cannot validate the effect/plan refuses
before execution; it never drops by guessed kind or silently treats AsyncDrop as
sync Drop.

## Comparison With Rust

The affine move rules, reverse acquisition order, and no-cleanup-after-abort
limit follow Rust's ownership discipline. The major difference is deliberate:
Rust `Drop` cannot await, and dropping a Future only runs synchronous destructor
code. Shape's `AsyncDrop` adds an effect-visible suspension and an evaluator
ledger that survives ordinary runtime failure and cooperative cancellation.

The proposal is not stronger than Rust at process loss. It also refuses to run
arbitrary user cleanup after a potentially corrupt engine fault. Bounded
abandonment, provider evidence, and leases are explicit because no async
destructor can guarantee peer close or rollback after host death.

## Misuse Prevention And Tradeoffs

The compiler rejects copied owners, duplicate semantic destructors, sync exits,
unbounded cleanup operations, detached cleanup tasks, owner cycles, leaked
fields, effect-erasing higher-order calls, snapshot of live obligations, and
provider evidence that is not host-validatable. Runtime IDs catch malformed
VM/JIT/resume transitions.

Costs are real: callable types gain an effect, every live owner needs ledger
metadata, function completion may suspend after its body is done, cancellation
latency includes a finite cleanup budget, and automatic cleanup reports are not
ordinary source values. Explicit close remains necessary for transactional
logic. These costs are preferable to an implicit async destructor whose strongest
claims disappear on the first runtime failure.

## Ordered Implementation And Proof

1. Ratify `AsyncCleanup`, primary-plus-cleanup evaluator metadata, deadline and
   abandonment terms, and the no-`finally` rule in an ADR.
2. Add affine AsyncDrop/effect checking and generate one MIR cleanup plan with
   partial-init, move, return, collection, closure, task, and hook-state edges.
3. Replace direct VM error/return/cancel exits with the common ledger-driven
   settler; keep synchronous Drop as a non-suspending obligation kind.
4. Add cleanup coroutine frames, deadline shields, instruction safepoints, and
   in-memory suspension/resume. Add snapshot barriers before serialization.
5. Make JIT yield the same unwind/suspension requests and compare complete
   lifecycle traces, including side-effect-before-failure without VM rerun.
6. Adapt IoHandle/FFI owners conservatively, then provider sessions with typed
   evidence, opaque placements, leases, and explicit unknown outcomes.
7. Prove every CFG edge and inject failure/cancellation at every acquire, move,
   await, cleanup, deadline, retry, and nested-hook transition. Add process
   isolation tests that explicitly show external effects can survive abort.

The feature is not shippable behind only parser syntax or `DropCallAsync`.
Compiler ownership, evaluator outcomes, suspension, VM/JIT, snapshots, and
provider evidence are one atomic semantic unit.
