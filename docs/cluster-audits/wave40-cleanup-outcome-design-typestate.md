# Wave 40O: Total AsyncDrop and MustSettle Typestate

Date: 2026-07-10

Scope: clean-break interface design over the accepted Wave-40 lifecycle,
failure-channel, automatic-unwind, two-phase cleanup, annotation, and remoting
provider reports plus current ownership, VM, JIT, and snapshot surfaces. This
document describes a target model, not current implementation.

## Decision

Use two related but deliberately different contracts:

1. Ordinary `AsyncDrop` is automatic and total for every contained
   `Completed`, `Failed`, and cooperative `Cancelled` exit. It always settles
   each registered obligation to truthful `Closed` or `Abandoned` evidence.
   Expected timeout, rejection, incomplete close, and outcome uncertainty are
   cleanup evidence. They never replace the Shape return value `R`, become a
   `RuntimeFailure`, or panic.
2. Wrap resources whose successful close, commit, or protocol resolution is a
   correctness condition in affine `MustSettle<T, Goal>`. The compiler rejects
   every normal source edge that would abandon such an owner. Explicit
   settlement consumes it and returns a typed, must-handle outcome. Failure or
   cancellation before settlement still invokes the same automatic fallback.

Ordinary resources therefore stay ergonomic while critical resources cannot
silently rely on best-effort destruction. Unlike Wave 40N, `Completed(R)` plus
`Abandoned` remains `Completed(R)` plus evidence. If abandonment must invalidate
success, the API exposes it through `MustSettle`; no `try`/`catch`/`finally` is
introduced.

## Current State And Gap

Today the compiler emits lexical `DropCall`/`DropCallAsync` and rejects an
async-only drop type in a sync function
(`crates/shape-vm/src/compiler/statements.rs:7379-7401`). The VM calls the drop
and preserves the return value (`crates/shape-vm/src/executor/trait_object_ops.rs:700-875`).
A genuine suspension escapes the nested driver as a VM error
(`crates/shape-vm/src/executor/dispatch.rs:581-651`).

Evaluator failure and caught exception truncation do not run all pending user
drops; cancellation is not a total joined unwind. JIT rejects user `Drop`
because native lowering only releases heap shares
(`crates/shape-jit/src/executor.rs:612-661`). Snapshots reject live futures
(`crates/shape-vm/src/executor/snapshot.rs:132-150`). See the full lifecycle
evidence in `wave40-shape-destruction-unwind-audit.md`.

Move/storage machinery exists, but affine settlement does not; `flow` typestate
is only deferred RFC work (`docs/rfcs/003-implicit-state-lints.md:6,22,332-351`).
Neither the total ledger nor `MustSettle` exists today.

## Outcome Boundary

Keep evaluator execution outcome separate from source return type. Conceptually:

```text
host-only Evaluation<R> =
    Suspended(Continuation<R>)
  | Settled(SettledEvaluation<R>)
  | Faulted(EngineFault, FaultCleanupReport)

host-only SettledEvaluation<R> {
    primary: Completed(R) | Failed(RuntimeFailure) | Cancelled(Cancellation),
    cleanup: CleanupReport,
}
```

These are host values, not implicit Shape returns. Source `Result::Err(E)` is
still `Completed(Result<R, E>)`; suspension retains the ledger without cleanup.

The projection rule is fixed before cleanup starts:

| Frozen primary | Cleanup evidence | Projection |
|---|---|---|
| `Completed(R)` | all `Closed` | preserve `Completed(R)` |
| `Completed(R)` | any `Abandoned` | preserve `Completed(R)` and report |
| `Failed(f)` | any terminal report | preserve `Failed(f)` and attach report |
| `Cancelled(c)` | any terminal report | preserve `Cancelled(c)` and attach report |
| cleanup invariant violation | restricted release report | `Faulted(EngineFault)` |

Only impossible ledger transitions, forged evidence, duplicate ownership,
corrupt plans, or violated trusted total contracts may create the last row.

## Automatic AsyncDrop Interface

The language contract has mandatory retire, close, and release phases:

```shape
affine trait AsyncDrop {
    type Retired
    type Closed: sealed CloseEvidence
    type Incomplete: sealed CleanupEvidence

    total sync method retire(self: own, cause: ExitCause) -> Self::Retired
        effects(NoSuspend, NoFail, NoReentry)

    async method close(state: &mut Self::Retired, ctx: CleanupContext)
        -> CloseAttempt<Self::Closed, Self::Incomplete>
        effects(CancelSafe, TrackedTasksOnly)

    total sync method release(
        state: own Self::Retired,
        disposition: CloseDisposition<Self::Closed, Self::Incomplete>,
    ) -> LocalReleaseEvidence
        effects(NoSuspend, NoFail, NoReentry)
}

sealed enum CloseAttempt<C, I> {
    Closed(C),
    Incomplete(I),
}

sealed enum CloseDisposition<C, I> {
    Graceful(C),
    Abandon(CleanupIncomplete<I>),
}
```

`retire` consumes public authority into evaluator ownership. `close` may
suspend but only borrows retired state, so cancellation and join restore host
control. `release` retires local mechanics for either disposition.

Source cannot call phases separately. Until effects are proven, `retire` and
`release` are compiler-derived or trusted host/provider operations. Provider
failure is `Incomplete`; `release` cannot throw, panic, or suspend.

### Typed evidence

The standard evidence envelope preserves cause and certainty independently:

```text
CleanupTerminal<C, I> =
    Closed { close: C, local: LocalReleaseEvidence }
  | Abandoned { incomplete: CleanupIncomplete<I>, local: LocalReleaseEvidence }

CleanupIncomplete<I> = Provider(I) | DeadlineExceeded | CancellationLatched
  | CloseRuntimeFailure | CloseNotStarted | QuiescenceIncomplete
  | ContainedWorkerTerminated
```

Provider `I` is schema-stamped, not text. It distinguishes rejection, transport
loss, protocol mismatch, and uncertainty. A host report may erase heterogeneous
evidence behind sealed schema IDs but cannot infer certainty from bits or I/O.

A close worker's `RuntimeFailure` becomes `CloseRuntimeFailure` evidence, never
the body primary. `EngineFault` is not converted or passed to source cleanup.

## Total Evaluator Ledger

Every successful construction or adoption registers one unforgeable
`ObligationId` after the emergency guard exists and before source use:

```text
Uninitialized -> Live/Moved -> Retired -> Closing <-> CloseSuspended
              -> Closed(evidence) | Abandoned(evidence)
```

The ledger records order, owner location, dependencies, plan, phase, deadline,
and evidence. Moves retain the ID; copies, untracked sharing, and escaping
borrows are rejected. Partial construction registers initialized fields only.

Every contained terminal edge enters one host unwind machine:

1. Freeze `Completed(R)`, `Failed(f)`, or `Cancelled(c)`.
2. Quiesce and join tracked borrowers under a bounded policy.
3. Retire every exiting live owner in reverse acquisition order.
4. Close and release retired entries in that same reverse order.
5. Append exactly one terminal evidence record per obligation.
6. Project the unchanged primary outcome with the ordered report.

Retire completes before the first close suspension. Failure never skips the
next entry; exhausted budgets still run emergency release and record
`CloseNotStarted` for every unstarted entry.

Cancellation is latched under an absolute host ceiling. Policy may narrow but
never extend it. Non-cooperative work requires a killable, joinable boundary.
For acquisition `a,b,c`, both sweeps run `c,b,a`, then project the primary.

Resources acquired by a close body form a nested cleanup scope and settle
before that close attempt terminates. No source callback can intercept or
reorder this sequence.

## MustSettle Typestate

`MustSettle` strengthens the normal control-flow rule without changing the
abnormal fallback:

```shape
sealed trait SettlementGoal<T: AsyncDrop> {
    type Proof: sealed SettlementEvidence
    type Incomplete: sealed SettlementIncomplete
}

affine opaque type MustSettle<T: AsyncDrop, G: SettlementGoal<T>>

@must_handle
affine enum SettleOutcome<P, I> {
    Settled(P),
    Incomplete(I),
}
```

`MustSettle<T, G>` owns `T` and its ledger ID. A resource method marked
`settling(G)` consumes that owner through the same retirement, close, and
release machine and returns `SettleOutcome<G::Proof, G::Incomplete>` only after
local release is terminal.

Commit and abort may both satisfy `TransactionResolved`; socket close alone may
not. Proof types are sealed/provider-registered to prevent stronger claims.

Proposed source sugar makes the obligation visible:

```shape
must settle let own tx = await payments.begin(placement)
```

The marker is sugar. The affine wrapper and ID, unlike a binding-only rule,
survive moves into fields, aggregates, closures, tasks, state, and returns.

### Normal-edge rule

At every normal source edge, an in-scope `MustSettle` owner must have exactly
one of these dispositions:

1. A `settling(G)` operation consumed it and its outcome was handled.
2. It transitioned to certified settled proof.
3. A supervisor/provider accepted it and returned a transfer receipt.
4. It moved into the declared return type and therefore to the caller.

Otherwise compilation fails with `E_MUST_SETTLE_NORMAL_EXIT`. Checked edges
include function fallthrough, `return`, block exit, `break`, `continue`,
short-circuit operators, annotation `Return`/`Recover`, closure escape, and
source `Result::Err` propagation through `?`. In particular, `?` is a normal
`Completed(Result::Err)` edge, not an abnormal runtime failure and not a way to
invoke fallback cleanup.

`return PendingTransaction(tx)` is legal only when declared: it transfers the
owner. Failed handoff leaves the sender owning it.

Malformed code reaching `Completed(R)` with a local unresolved owner becomes
`Faulted(UnresolvedMustSettleOnNormalExit)` and runs restricted fallback. This
is an invariant fault, not expected cleanup failure.

### Expected incomplete settlement

Explicit settlement never throws for timeout, rejection, or an incomplete
external result:

```shape
settling(TransactionResolved)
async method commit(self: own, deadline: Deadline)
    -> SettleOutcome<CommitReceipt, TransactionIncomplete>
```

`SettleOutcome` cannot be dropped or wildcard-discard `Incomplete`. The checker
proves handling, not business policy; APIs normally map it into domain `Result`.

If external state is still unresolved, the incomplete variant carries a new
affine recovery obligation rather than pretending the original operation is
done:

```shape
enum TransactionIncomplete<P> {
    Rejected(RejectEvidence),
    TimedOut(TimeoutEvidence),
    OutcomeUnknown {
        evidence: UnknownEvidence,
        recovery: MustSettle<RecoveryHandle<P>, OperationResolved>,
    },
}
```

Definitive rejection may be terminal; `OutcomeUnknown` carries a recovery owner
that must be resolved, accepted by a supervisor, or returned. Retry requires
explicit idempotency, dedup, status, lease, or fencing authority.

## Examples

An ordinary session does not change `R` when peer close is incomplete:

```shape
async cleanup fn read_record<P>(provider: RemoteProvider<P>,
                                placement: Placement<P>, key: Key) -> Record {
    let own session = await provider.open_required(placement)
    await session.read_required(key)
}

// Possible host projection:
// Completed(record) + Abandoned(SessionCloseTimedOut(...))
```

`Placement<P>` is opaque, not an address string. The provider owns discovery,
routing, encoding, transport, auth, codec, negotiation, deadlines,
cancellation, and observability.

A critical transaction cannot fall through unresolved:

```shape
async cleanup fn bad_charge<P>(payments: Payments<P>, placement: Placement<P>,
                               command: Charge) -> CommitReceipt {
    must settle let own tx = await payments.begin(placement)
    let receipt = await tx.stage(command)
    receipt
    // E_MUST_SETTLE_NORMAL_EXIT: `tx` has no TransactionResolved proof
}
```

The honest API exposes expected settlement outcomes as a domain result:

```shape
async cleanup fn charge<P>(payments: Payments<P>, placement: Placement<P>,
                           command: Charge) -> Result<CommitReceipt, ChargeError<P>> {
    must settle let own tx = await payments.begin(placement)
    await tx.stage_required(command)

    match await tx.commit(2.seconds) {
        SettleOutcome::Settled(receipt) => Ok(receipt),
        SettleOutcome::Incomplete(TransactionIncomplete::OutcomeUnknown {
            evidence, recovery
        }) => Err(ChargeError::Pending { evidence, recovery }),
        SettleOutcome::Incomplete(problem) =>
            Err(ChargeError::NotCommitted(problem)),
    }
}
```

If `stage` returned `Result` and the code used `await tx.stage(command)?`, the
compiler would reject that short-circuit until the `Err` branch explicitly
aborted, settled, or transferred `tx`.

Failure/cancellation before `commit` returns no `SettleOutcome`; bounded
fallback retires, attempts abort/lease surrender, releases, records evidence,
and preserves that primary outcome.

## Annotation Composition

Annotations use the ledger, not `finally`. Ordinary state settles on layer
exit; `MustSettle` state adds normal-edge proof to every hook path.

Rules are:

1. An annotation cannot `Return(R)` or `Recover(R)` while its local
   `MustSettle` owner is unresolved.
2. Propagated failure/cancellation triggers automatic fallback.
3. An attempt becomes terminal before outer retry/recovery; outer state is
   separately owned.
4. An unknown external outcome blocks retry unless explicit idempotency/dedup
   policy authorizes it.
5. Cleanup outcomes never enter `on_failure` as `RuntimeFailure`; a read-only
   final observer may log them but cannot replace `R` or evidence.

Transparent `@remote` may therefore preserve a target signature `R` for an
ordinary call-owned session. It cannot hide a correctness-critical peer
acknowledgement or transaction commit behind automatic cleanup. Such a target
must return a typed domain `Result`, a settlement proof, or an affine recovery
owner. The generic failure hook remains transport-neutral.

## Providers And Remote Resources

Opaque `Placement<P>`/`RemoteDestination<P>` are inert capabilities, not
sessions or prescribed wire addresses; provider contracts may make them
snapshot-safe.

Sessions, channels, lease tasks, generation pins, and participants are affine.
Provider adapters supply guards, bounded close, release, and evidence; host
policy validates signatures, certainty, provenance, and retry authority.

Use ordinary `AsyncDrop` for a call-owned session when local release and an
honest abandonment report are sufficient. Use, for example:

```shape
type OpenRemoteTransaction<P> =
    MustSettle<RemoteTransaction<P>, TransactionResolved>

type AcknowledgedSession<P> =
    MustSettle<RemoteSession<P>, PeerCloseAcknowledged>
```

Pooling is an affine transfer. The call ledger removes a session only after the
pool durably accepts its obligation and returns a receipt. Provider reload pins
the old generation until every transferred owner is terminal. A provider may
report local release, peer rejection, authenticated acknowledgement, or
outcome unknown; it may not upgrade one into another.

## Suspension, Cancellation, And Snapshots

Body suspension stores the ledger. Cleanup suspension also stores the frozen
primary, retired guards, cursor, evidence, shield, and deadline; resume never
reconstructs owners from raw bits.

Cancellation during explicit settlement transfers control to automatic
fallback before source receives a `SettleOutcome`. The cancellation remains
primary. An operation-local deadline, by contrast, completes explicit
settlement with typed `Incomplete(DeadlineExceeded)` evidence.

Initially, any live/retired obligation, close future, cleanup borrow, or
unhandled affine outcome is a snapshot barrier. Snapshot never invokes cleanup.

Opt-in durable recovery may persist stable provider/version, operation ID,
lease, evidence schema, idempotency, and one recovery owner, never sockets,
secrets, routes, tasks, pointers, guards, or continuations. Restore revalidates
and creates exactly one obligation or refuses.

## VM And JIT Contract

One immutable `CleanupPlan` shared by bytecode and MIR records IDs, order,
dependencies, `Automatic`/`MustSettle(goal)`, adapters, schemas, and exits.

All VM/JIT exits enter one evaluator trampoline with primary and plan cursor;
native side effects are never replayed. Parity requires equal lifecycle traces,
evidence, ordering, and projection, not merely equal `R`.

Linked or remotely transferred code must carry hash-covered cleanup-plan,
goal, and evidence-schema facts. Runtime side tables may cache resolved
adapters, but missing or contradictory metadata is a link/invariant error, not
permission to omit cleanup or downgrade `MustSettle` to automatic drop.

## Guarantees

Given an intact evaluator process and cooperative or contained provider work:

1. Every initialized affine obligation has one owner and one terminal evidence
   record; no contained exit or exhausted budget skips it.
2. Retirement precedes suspension and all close/release work follows reverse
   dynamic acquisition/adoption order.
3. Automatic cleanup timeout, rejection, close failure, and uncertainty remain
   typed evidence and never replace `R`, `RuntimeFailure`, or cancellation.
4. Every normal source edge proves a `MustSettle` owner was settled or
   transferred. Abnormal failure/cancellation still performs fallback cleanup.
5. Local release occurs after every graceful or abandoned close attempt.
6. Cleanup cancellation is shielded, bounded, joined, and cannot erase the
   caller's cancellation.
7. VM and JIT consume the same plan and evaluator ledger.
8. Snapshot either preserves one stable logical owner under an explicit
   contract or refuses before persistence.
9. Provider evidence cannot claim stronger certainty than authenticated facts
   support, and unknown-outcome retry requires explicit safety authority.

## Limits

No language rule can execute after `SIGKILL`, abort, power loss, fatal FFI
undefined behavior, or host disappearance. The OS reclaiming a descriptor does
not prove peer close, rollback, remote cancellation, flush, or non-execution.
External correctness still requires leases, fencing, durable transactions,
deduplication, and status queries.

An engine invariant fault may make arbitrary Shape cleanup unsafe. The host
uses only trusted retirement/release metadata and records abandonment where the
ledger remains trustworthy. If ledger identity itself is corrupt, the runtime
must report `Faulted` rather than fabricate the no-skipped-obligations claim.

The compiler proves ownership and explicit handling, not business intent. A
program can explicitly map typed `Incomplete` to an application success; that
decision is visible in source review. Sealed evidence prevents fabricating a
proof but cannot decide policy for the application.

## Tradeoffs And Alternatives

A binding-only `must settle let` is simpler but unsound across moves and
aggregation. Transforming every function into `Result<R, CleanupError>` exposes
cleanup but changes `R`, burdens composition, and conflates expected close
outcomes with body semantics. Making automatic abandonment a non-returning
failure has the same problem indirectly.

The selected design adds affine aggregate rules, flow-sensitive normal-edge
checking, must-handle outcomes, a host cleanup report, suspension state, and
provider evidence schemas. `?` becomes less convenient while a critical owner
is live, and providers must implement cancellation-safe close plus total local
release. These costs are concentrated on resources that need the guarantees.

In return, ordinary code keeps its declared `R`; expected cleanup outcomes are
truthful and observable without panic; and correctness-critical resources
cannot disappear through normal control flow. Compared with Rust, Shape gains
automatic awaited destruction and a stronger explicit-settlement typestate,
while retaining the same honest process-abort boundary.

## Proof Plan

Compiler proofs cover all ownership carriers and exits, partial initialization,
sync rejection, and malformed completion. Fault injection covers every phase,
suspension, failure/cancellation, deadlines, nesting, continuation after
incomplete close, and invariant containment.

VM/JIT tests compare lifecycle traces. Snapshot/provider tests cover barriers,
single-owner recovery, rejection/ack loss/unknown outcome, containment,
transfer, reload, leases, and dedup retry. Process-loss tests assert only
durable external facts, never post-death destructor execution.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited, and no cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-cleanup-outcome-design-typestate.md`
