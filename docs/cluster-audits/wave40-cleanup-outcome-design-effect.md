# Wave 40P: Checked `Cleanup<E>` Effect

Date: 2026-07-10

## Decision

Keep automatic `AsyncDrop`, the total evaluator ledger, and the two-phase
`retire -> close -> release` protocol, but represent every expected non-ideal
cleanup outcome as a checked `Cleanup<E>` effect. The effect is part of the
callable type and is separate from the normal return type `R`:

```shape
async fn read(placement: Placement<Storage>) -> Batch
    ! { Cleanup<StorageCleanup> }
```

`read` still returns `Batch`. A deadline, peer rejection, incomplete close, or
outcome-unknown release is not `RuntimeFailure`, is not a panic, and does not
turn `Batch` into `Result<Batch, _>`. It produces an affine typed cleanup batch
that must be exhaustively accepted, transformed, or propagated. Only a broken
ledger, forged ownership/evidence, or violation of a compiler/trusted-host
`total` contract may produce `Faulted(CleanupInvariant)`.

This is a clean break. Shape has no general checked effect-row syntax today;
preserving current implicit `DropCallAsync` behavior would retain abnormal-exit,
suspension, and JIT gaps. The compiler must lower ownership once to a total
plan, and VM and JIT must enter one evaluator cleanup machine on every contained
`Completed`, `Failed`, and cooperative `Cancelled` exit.

`Cleanup<E>` is not `try/catch/finally`:

- ownership registration, retirement, close, and release run before any source
  cleanup handler and cannot be omitted by it;
- the handler cannot catch or replace a primary failure/cancellation;
- the handler cannot replace `R`, retry the body, or resume a retired resource;
- handlers are total and run only after all obligations are terminal; and
- propagation is a checked, resumable secondary effect, not stack unwinding.

## Two-Phase Resource Contract

The semantic trait follows the accepted guarded two-phase design:

```text
affine trait AsyncDrop {
    type Retired
    type Closed: CleanupEvidence
    type Expected: CleanupEvidence

    total sync fn retire(self: own, cause: ExitCause) -> Self::Retired
        effects(NoSuspend, NoFail, NoReentry)

    async fn close(
        state: &mut Self::Retired,
        context: CloseContext,
    ) -> CloseResult<Self::Closed, Self::Expected>
        effects(Suspend, CleanupBody)

    total sync fn release(
        state: own Self::Retired,
        disposition: CloseDisposition<Self::Closed, Self::Expected>,
    ) -> LocalReleaseEvidence
        effects(NoSuspend, NoFail, NoReentry)
}

enum CloseResult<C, E> {
    Closed(C),
    Expected(E),
}

enum CloseDisposition<C, E> {
    Graceful(C),
    Expected(E),
    Abandon(UniversalCleanupIssue),
}
```

Registration occurs only after successful initialization and guard creation.
`retire` consumes public access without awaiting. `close` borrows the retired
state, so cancellation or a valid-code failure cannot strand ownership.
`release` always consumes the state and retires local mechanics. Source code
cannot invoke the three methods separately; explicit `close resource` asks the
same evaluator protocol to consume that ledger entry early.

`CleanupBody` permits only tracked, deadline-aware operations. It forbids
detached work, owner reentry, unbounded native calls, snapshot, effect-handler
invocation, and movement of the retirement guard. A provider callback that can
block must run behind a killable process boundary.

Operational close failures are data. A valid Shape `Failed` while evaluating a
close body is captured as `CleanupEvent::CloseFailed`; provider timeout,
rejection, and protocol incompleteness are typed outcomes. A Rust panic,
impossible state transition, duplicate terminal record, or failed `retire` or
`release` totality assertion is an invariant fault. This is the only cleanup
path allowed to become `Faulted`.

## Effect Payload And Aggregation

`E` is the resource/provider-specific expected outcome. The language supplies
the universal outcomes that can arise even when `E = Never`:

```text
enum CleanupEvent<E> {
    Resource(E),
    DeadlineExceeded(DeadlineEvidence),
    CloseNotStarted(BudgetEvidence),
    CloseFailed(CloseFailureEvidence),
    CloseCancelled(CancellationEvidence),
    QuiescenceIncomplete(QuiescenceEvidence),
    LocalReleaseUnproven(ContainmentEvidence),
}

enum CleanupRecord<E> {
    Closed {
        obligation: ObligationId,
        evidence: OpaqueCloseEvidence,
        local: LocalReleaseEvidence,
    },
    Issue {
        obligation: ObligationId,
        event: CleanupEvent<E>,
        local: LocalReleaseEvidence,
    },
}

affine CleanupBatch<E> {
    primary_class: Completed | Failed | Cancelled,
    records: ordered Array<CleanupRecord<E>>,
}
```

The batch is ordered by actual cleanup execution, normally reverse dynamic
acquisition. It records successful closes for evidence and observability, but a
`Cleanup<E>` operation is emitted only when at least one `Issue` remains. Thus
ordinary all-closed calls do not run a source handler.

The batch is affine and its issue records are `must_consume`. A handler must
consume every issue with `accept` or `propagate`; dropping the batch, using a
non-exhaustive match, or hiding an issue behind `AnyError` is a compile error.
Complete records may be observed and then structurally discarded.

`CleanupEvidence` is sealed for claims such as peer acknowledgement, local
release, execution certainty, operation identity, or lease state. User-defined
domain detail may be carried inside provider-defined variants, but source code
cannot mint stronger certainty or reconstruct it from strings.

## Effect Rows

The illustrative callable syntax is:

```text
fn(P...) -> R ! { EffectA, EffectB, Cleanup<E> }
```

`async fn` implies `Suspend`; it does not erase `Cleanup<E>`. Effect rows obey
these rules:

1. A row contains at most one normalized `Cleanup` member.
2. Combining `Cleanup<E1>` and `Cleanup<E2>` produces
   `Cleanup<E1 | E2>`, where `|` is a compiler-generated closed tagged sum,
   not dynamic `any`.
3. `Cleanup<Never>` remains checked because universal deadline, cancellation,
   and abandonment events are still possible. Only proving that no armed
   `AsyncDrop` reaches an exit removes the effect.
4. A callable with fewer effects is usable where a wider row is permitted.
   `Cleanup<E1>` is a subtype of `Cleanup<E2>` only when every `E1` variant has
   a declared injection into `E2`.
5. Private functions and closures may infer rows. Exported functions, trait
   methods, annotation handlers, and higher-order bounds state them explicitly.
6. A handler removes only the variants it consumes and introduces only the
   residual variants it explicitly propagates.

The error type is a row-like sum because dynamic acquisition changes the count,
not the set of possible event types. Acquiring ten `Session` values still adds
one `SessionCleanup` variant to the callable row; the runtime batch carries ten
ordered records if all ten need attention.

Domain `Result<R, D>` remains an ordinary value. These are distinct:

```text
fn save() -> Result<Receipt, ValidationError>
    ! { Cleanup<SessionCleanup> }
```

`Err(ValidationError)` is `Completed(Result::Err(...))`; session cleanup issues
remain the checked secondary effect.

## Exhaustive Handling And Propagation

Handling is deliberately narrower than a general algebraic-effect handler:

```shape
handle cleanup from await load(placement) with |batch, ctx| {
    for issue in move batch.issues() {
        match issue.event {
            Resource(StorageCleanup::PeerRejected(e)) => {
                ctx.events.record(e)
                accept issue
            }
            Resource(StorageCleanup::OutcomeUnknown(e)) =>
                propagate issue as AppCleanup::StorageUnknown(e)
            DeadlineExceeded(e) =>
                propagate issue as AppCleanup::Deferred(e)
            CloseNotStarted(e) =>
                propagate issue as AppCleanup::Deferred(e)
            CloseFailed(e) =>
                propagate issue as AppCleanup::CloseCode(e)
            CloseCancelled(e) =>
                propagate issue as AppCleanup::Deferred(e)
            QuiescenceIncomplete(e) =>
                propagate issue as AppCleanup::Containment(e)
            LocalReleaseUnproven(e) =>
                propagate issue as AppCleanup::Containment(e)
        }
    }
}
```

This expression still has the underlying type returned by `load`. Its residual
row contains `Cleanup<AppCleanup>`. An all-accepting handler removes `Cleanup`
entirely. `propagate all` is permitted in a generic handler because it preserves
the exact affine batch and type; a wildcard that accepts unknown variants is
not exhaustive handling.

A cleanup handler has the semantic signature:

```text
type CleanupHandler<E, F> = total fn(
    batch: own CleanupBatch<E>,
    context: CleanupHandlerContext,
) -> CleanupResolution<F>
    effects(NoSuspend, NoFail, NoReentry)

affine CleanupResolution<F> =
    Accepted
  | Residual(CleanupBatch<F>)
```

The handler runs after all entries in that batch are `Closed` or `Abandoned`.
It receives no `R`, `RuntimeFailure`, cancellation token, continuation, retry
permit, raw resource, or provider credential. Its event sink is a total
nonblocking observation capability. Asynchronous reconciliation must be handed
to a pre-existing supervised service or propagated to a host handler; the
cleanup handler cannot detach a task during unwind.

On `Completed`, accepted handling permits the held `R` to be published. On
`Failed` or `Cancelled`, the same handler consumes or propagates cleanup issues
while the sealed primary continues unchanged. It cannot recover either primary.
Propagation searches outward without destroying or replaying frames; the
evaluator retains the affine primary value, ledger evidence, and handler cursor.
An unhandled root effect is a compile/configuration error, not a runtime panic.

The CLI, server, and embedding APIs must install a root handler for the entry
row. A standard host handler may render structured warnings, persist recovery
identities, and choose an operational exit status. It must not relabel the
evaluation as `RuntimeFailure` or claim successful close.

## Explicit Close For Business Decisions

Automatic cleanup handlers acknowledge or propagate evidence; they do not
replace `R`. Code whose domain decision depends on close, commit, rollback, or
peer acknowledgement closes explicitly:

`close` projects `Closed(C) | Expected(E) | Universal(UniversalCleanupIssue)`
as an ordinary closed value.

```shape
async fn publish(
    placement: Placement<EventProvider>, event: Event,
) -> Result<Receipt, PublishIncomplete> {
    let own session = await events.open(placement)
    let receipt = await session.publish(event)?

    match await close move session {
        Closed(e) if e.peer_acknowledged => Ok(receipt)
        Closed(_) => Err(PublishIncomplete::PeerUnconfirmed)
        Expected(e) => Err(PublishIncomplete::Provider(e))
        Universal(e) => Err(PublishIncomplete::Cleanup(e))
    }
}
```

Explicit close uses the same obligation ID and retire/close/release state
machine, then disarms the automatic entry. Its result is an ordinary exhaustive
value because the caller deliberately requested evidence. If the expression is
cancelled or fails before producing that value, the evaluator still owns the
retired guard; automatic unwind finishes or abandons it and emits `Cleanup<E>`.

## Automatic Unwind And Precedence

For a contained terminal exit the evaluator performs:

1. Freeze the primary `Completed(R)`, `Failed(RuntimeFailure)`, or
   `Cancelled(Cancellation)` and stop new admissions.
2. Request cancellation and join every cooperative child that can hold a loan.
3. Retire all exiting live owners in reverse acquisition order without
   suspension, making source access impossible before the first close awaits.
4. Close and release each retired owner in the same reverse order under one
   inherited finite shield, appending a record even when close was not started.
5. Invoke the checked cleanup-handler chain for any issue batch.
6. Publish the unchanged primary after the effect is accepted at some boundary.

Every initialized obligation has one owner, one retirement, one release, and
one terminal record. A close issue never short-circuits the next entry. If the
budget expires, unstarted entries receive typed `CloseNotStarted` records and
still run total release. There is no source `finally` edge to omit.

Expected cleanup outcomes never replace the primary:

| Primary | Cleanup issues after handling | Projection |
|---|---|---|
| `Completed(R)` | accepted | `Completed(R)` |
| `Completed(R)` | propagated | remain at checked `Cleanup<E>` boundary |
| `Failed(f)` | accepted | `Failed(f)` with cleanup audit |
| `Cancelled(c)` | accepted | `Cancelled(c)` with cleanup audit |
| any | cleanup invariant violation | `Faulted(CleanupInvariant)` with interrupted primary |

The evaluator owns a computed affine `R` until cleanup effects are accepted; a
handler cannot observe, copy, or lose it. Cleanup cannot recursively invoke
`on_failure`, trigger retry, or fabricate a replacement value.

## Cancellation, Deadlines, And Faults

The first cooperative cancellation is latched as the primary. Cleanup defers
ordinary cancellation but uses an absolute deadline equal to the minimum of
host policy, caller budget, enclosing scope, resource cap, and annotation
budget. Nested cleanup cannot reset it.

At deadline the active close receives deadline cancellation and is joined. Its
entry gets a typed timeout/cancellation issue; every remaining retired entry is
released and gets `CloseNotStarted`. The subsequent total handler pass is
instruction-bounded and cannot suspend. A non-cooperative close must be process
contained; inability to terminate that container is typed
`LocalReleaseUnproven`, while a false claim that it was terminated is a fault.

For a contained engine fault, arbitrary Shape close/handler code does not run.
Trusted host metadata retires/releases what it can and records abandonment for
the host fault report. Process abort, `SIGKILL`, OOM kill, segfault, power loss,
or host disappearance cannot run either phase or emit an effect. Remote safety
after process loss still requires leases, fencing, transactions, durable
deduplication, and status recovery.

## Sync, Async, And Higher-Order Types

Owning an armed `AsyncDrop` value introduces both `Suspend` and `Cleanup<E>` on
every edge where automatic close may run. A synchronous callable may construct
or receive such an owner only if it transfers ownership out on every path; it
cannot block during exit or install a hidden runtime.

An async function may handle `Cleanup<E>` locally and export no cleanup effect,
but it remains async because close can suspend. An `AsyncDrop<Expected = Never>`
resource still makes exit suspendable even though only universal deadline or
containment issues can appear.

Higher-order APIs are effect-polymorphic:

```text
fn map<T, U, fx>(
    values: Array<T>, f: fn(T) -> U ! fx,
) -> Array<U> ! fx

fn with_session<R, fx>(
    placement: Placement<P>,
    body: fn(&Session<P>) -> R ! fx,
) -> R ! normalize(fx + Suspend + Cleanup<SessionCleanup<P>>)
```

Invoking a callback with a wider row than the bound permits is a compile error.
Trait implementations may narrow declared effects but cannot add cleanup
variants. Closure capture of an owned resource moves its obligation and latent
cleanup effect with the closure.

## Annotation Composition

Treat effects as part of a finalized callable type:

```text
Callable<Sig, Fx>
Annotation<Sig, Fx> -> Callable<Sig, normalize(Fx + HookFx)>
```

Before, after, and failure hooks declare their own rows. `HookState<S>` that can
`AsyncDrop<Expected = E>` adds `Suspend + Cleanup<E>` to `HookFx`; its
obligation moves unchanged through `Proceed`, `Return`, retry, backoff, and
suspension. Per-attempt obligations settle before a new attempt. Layer-wide
state settles only when that annotation layer terminates.

An annotation may declare a total cleanup-effect transformer:

```text
type AnnotationCleanup<E, F> = CleanupHandler<E, F>
```

Inner handlers consume or transform first; residual batches flow outward in
reverse layer order. This phase owns no hook state and runs after that layer's
resource ledger is terminal. It is not `after`, `on_failure`, or `finally`.
`on_failure` still receives only the sealed primary `RuntimeFailure`; cleanup
issues never recursively enter it. A retry policy may inspect read-only cleanup
evidence, but outcome-unknown retry still requires a typed idempotency,
transaction-status, or lease/fencing capability.

`@remote<P>(Placement<P>)` remains parameter/return transparent but explicitly
adds its mechanics and cleanup effects to the callable row. It does not turn
`R` into `Result<R, RemoteError>` and does not map session cleanup to remote
`RuntimeFailure`.

## Providers And Evidence

Provider mechanics remain behind opaque capabilities:

```shape
impl<P: RemotingProvider> AsyncDrop for RemoteSession<P> {
    type Retired = RetiredSession<P>
    type Closed = SessionCloseReceipt<P>
    type Expected = SessionCleanup<P>

    // Provider-owned discovery, routing, address encoding, transport,
    // authentication, codec, negotiation, cancellation, and telemetry.
}
```

`RemoteDestination<P>` and `Placement<P>` are inert typed values, not addresses
or live resources. The host validates provider evidence and retains authority
over signature, attempt identity, execution certainty, idempotency, deadline,
and cancellation semantics. `SessionCleanup<P>` may carry authenticated
rejection, peer-unconfirmed close, `OutcomeUnknown`, operation identity, or
lease expiry. It cannot infer non-execution from a socket close or status text.

`remote::call` may still return a domain `Result<R, RemoteError>` for the remote
operation. Session cleanup is independently `Cleanup<SessionCleanup<P>>`.
Connection-pool adoption moves the affine obligation into the provider's root
ledger; provider shutdown later emits its own checked aggregate.

## Suspension, Snapshot, Transfer, And VM/JIT

Body suspension preserves the live ledger and effect-handler stack without
running cleanup. Cleanup suspension preserves the frozen primary, retired
owners, active close loan, reverse cursor, absolute deadline, aggregate, and
handler search point. Resume continues the same obligation; it never reruns the
body, close prefix, annotation, or remote submission.

Durable snapshot is fail-closed. Any live `AsyncDrop` owner, borrower, retired
state, close continuation, unconsumed cleanup batch, or pending handler is a
typed `SnapshotBarrier`. Snapshot never closes or accepts cleanup implicitly.
A future opt-in may consume one owner into a provider-neutral, versioned,
idempotent recovery token; it may not serialize routes, addresses, sockets,
credentials, tasks, provider pointers, or close futures.

The compiler emits one hash-covered `CleanupPlan`: effect row, obligation IDs,
move/transfer edges, retire/close/release identities, landing edges, and handler
transforms. VM bytecode and JIT MIR reference that same plan. Native negative
signals return to the common evaluator without interpreter replay. VM/JIT parity
means identical ownership transitions, cleanup records, effect propagation,
and primary projection, not merely equal printed output. A receiver that cannot
validate transferred cleanup metadata refuses before execution.

## Ergonomics And Tradeoffs

Private code normally writes no per-call handler: cleanup effects infer and
propagate with the enclosing row. Applications can install one exhaustive
provider or subsystem handler at a deliberate boundary. All-closed calls run no
source handler, and `R` stays uncluttered. Explicit close remains available when
business logic needs the evidence as a value.

The costs are substantial: Shape gains checked effect rows, closed-sum
normalization, affine handler batches, total-handler effect checking,
higher-order effect polymorphism, cleanup continuation metadata, and larger
callable/content identities. Public APIs expose cleanup variants, and adding a
provider outcome can be a source-breaking exhaustiveness change. Restricting
automatic handlers from suspension or result replacement is less expressive
than general algebraic effects.

Those restrictions are the point. Expected cleanup is visible and typed without
becoming an exception, ordinary return type, or optional callback. Ownership
makes cleanup total; effect checking makes its evidence impossible to silently
discard; explicit close handles the cases where cleanup must influence domain
control flow.

## Proof Boundary

Compiler proofs must cover partial initialization, affine move/transfer,
reverse dynamic order, every terminal edge, effect-row inference/normalization,
exhaustive affine handlers, higher-order bounds, annotation transforms, sync
rejection, and transferred-plan validation.

Evaluator fault injection must cover every retire/close/release transition,
multiple expected issues, deadline before each close, close-body failure,
handler propagation, cancellation, retry, suspension/resume, and contained
fault, asserting one terminal record and no skipped obligation. VM/JIT tests
compare full lifecycle traces. Snapshot tests refuse every live phase. Provider
fixtures cover acknowledged close, typed rejection, dropped acknowledgement,
outcome unknown, pool transfer, lease expiry, and process containment.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited. No cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-cleanup-outcome-design-effect.md`
