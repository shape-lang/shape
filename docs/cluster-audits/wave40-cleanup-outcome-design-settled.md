# Wave 40Q: Explicit Settled Cleanup Outcome

Date: 2026-07-10

Scope: clean-break interface design over the accepted total lifecycle,
automatic `AsyncDrop`, two-phase cleanup, failure-channel, delivery, and
provider reports plus current Shape `Result` and `?` conventions. This report
does not claim implementation.

## Decision

Keep automatic `AsyncDrop` and its total evaluator ledger, but project expected
cleanup outcomes through one explicit typed surface:

```text
Settled<R, C>
```

`R` remains the callable's body completion type. `C` is the callable's closed
cleanup contract and report schema. Expected close timeout, rejection, unknown
outcome, close-body failure, deadline exhaustion, or emergency abandonment
produces `Settled::Incomplete`; none becomes `RuntimeFailure`, panic, or
`Faulted`.

Use three call forms:

1. `await f()` is allowed only when `C` proves cleanup cannot be incomplete and
   its evidence is safe to erase.
2. `await settle f()` materializes `Settled<R, C>` and requires exhaustive
   handling when evidence matters.
3. `await forward settle f()` propagates the cleanup control channel through a
   callable that declares a compatible contract, without allocating or nesting
   a wrapper at every call.

The compiler/evaluator still retires, closes, and releases every initialized
affine obligation on all contained `Completed`, `Failed`, and cooperative
`Cancelled` exits. The settlement surface observes that total lifecycle; it
does not replace it with optional `try`/`catch`/`finally` code.

Only an invariant violation may produce `Faulted(CleanupInvariant)`: a lost or
duplicate owner, skipped ledger entry, failed supposedly-total retire/release,
forged evidence, corrupt continuation, VM/JIT plan disagreement, or contained
provider ABI panic. Ordinary incomplete cleanup is data.

## Projection Alternatives

Three simpler projections are inadequate:

- **Primary always wins, cleanup only in host metadata:** preserves `R`, but
  source code cannot enforce acknowledgement or recovery obligations.
- **Incomplete cleanup becomes `RuntimeFailure`:** makes automatic cleanup
  visible by misclassifying expected distributed/resource outcomes and may
  discard useful `R`.
- **Every function returns `Result<R, CleanupError>`:** conflates domain errors
  with lifecycle evidence and creates nested wrappers through helpers,
  annotations, async tasks, and `remote::call`.

`Settled` is a separate, explicit settlement channel. A domain
`Result::Err(e)` remains a value inside `R`; a body runtime failure remains an
evaluator `Failed`, not a `Settled` variant.

## Typed Surface

The semantic types are:

```text
compiler-validated cleanup contract C for R {
    type Entry: CleanupEntry
    type ValueOnIncomplete: Available<R> | ValueWithheld
    const completeness: CompleteOnly | MayBeIncomplete
    const observation: Erasable | Required
}

enum Settled<R, C: CleanupContract<R>> {
    Complete {
        value: R,
        cleanup: CleanupReport<C, AllClosed>,
    },
    Incomplete {
        value: C::ValueOnIncomplete,
        cleanup: CleanupReport<C, HasIncomplete>,
    },
}

affine struct Available<R> { value: R }
struct ValueWithheld { reason: ValueWithheldReason }

struct CleanupReport<C, State> {
    entries: Array<C::Entry>,       // actual terminal order
    summary: CleanupSummary<State>,
}

type TerminalCleanupReport<C> =
    CleanupReport<C, AllClosed> | CleanupReport<C, HasIncomplete>
```

`AllClosed` proves every registered obligation has validated close evidence.
`HasIncomplete` proves at least one typed incomplete entry. Reports include all
obligations, including successful closes before and after a failure; they are
flat and ordered, not one wrapper per resource or stack frame.

A contract's entry sum is nominal and exhaustive:

```text
CleanupTerminal<E, I> =
    Closed {
        obligation: ObligationId,
        evidence: E,
        local_release: LocalReleaseEvidence,
    }
  | Incomplete {
        obligation: ObligationId,
        cause: I,
        local_release: LocalReleaseEvidence,
        external: ExternalCertainty,
        recovery: Option<affine RecoveryCapability>,
    }

ExpectedIncomplete =
    TimedOut | Rejected(TypedCause) | OutcomeUnknown(OperationId) |
    CloseBodyFailed(RuntimeDiagnostic) | CloseNotStarted(Deadline) |
    EmergencyAbandonment(AbandonReason)
```

The cleanup driver converts an ordinary evaluator failure inside `close` into
typed `CloseBodyFailed`, then runs emergency release and continues. A provider
process exit can similarly be typed. An in-process panic, invalid evidence, or
ledger violation is an invariant fault and does not construct `Settled`.

`CleanupReport` is affine whenever an entry owns a lease, transaction-status
handle, dedup permit, or other recovery capability. `Required` reports are
`must_settle`: wildcard discard, implicit drop, boolean success conversion, and
unconditional `unwrap` are compile errors. Borrowing for logging does not
consume the recovery obligation.

## Is `R` Available When Cleanup Is Incomplete?

Not by default. The body-computed `R` remains provisionally evaluator-owned
until settlement. A generated cleanup contract selects exactly one policy:

### Withhold, the default

`C::ValueOnIncomplete = ValueWithheld`. The evaluator does not publish `R`.
It settles any affine obligations inside `R`, appends those entries to the same
report, and records why the value was withheld. Use this for transaction
results, flush-defined output, resource-backed views/iterators, borrowed data,
or any value whose validity depends on close acknowledgement.

### Preserve only with evidence

`C::ValueOnIncomplete = Available<R>` is legal only when:

1. ownership analysis proves `R` has no loan, capture, shared owner, provider
   handle, or affine dependency on an exited obligation;
2. the declared operation contract says close completion does not establish the
   semantic validity of `R`; and
3. no incomplete case means the body result itself is outcome-unknown; and
4. every forwarded-incomplete control edge can supply the exact outer `R`, or
   the outer contract withholds it.

The declaration is part of the public contract, not a provider-controlled flag.
For provider resources, the host validates it against the operation protocol.
The compiler can prove ownership independence; semantic independence needs a
nominal audited witness and defaults to withheld when absent.

Examples where preservation can be valid include an already-decoded immutable
response followed by failure to acknowledge closing a call-owned transport
session. A lost remote operation response has no `R` and cannot use this rule.

Generic code over unknown `C` sees `C::ValueOnIncomplete` and cannot assume an
`R`. A concrete preserve contract exposes `Available<R>` directly, without an
impossible `Withheld` branch; a withhold contract exposes no hidden value.

## Callable Typing

Cleanup is a callable effect separate from return type:

```shape
cleanup contract QueryCleanup for QueryResult {
    entries: SessionClose | LeaseRelease
    incomplete_value: available
    observation: required
}

async fn query(key: Key) -> QueryResult settles QueryCleanup
```

The semantic callable type is `fn(P...) -> R settles C`. `settles Clean` is the
default for callables with no observable async cleanup. Exported functions,
trait methods, closures crossing boundaries, higher-order bounds, and callable
annotations include `C`; private functions may infer it.

Contracts compose as a canonical cleanup row. Repeated dynamic obligations use
one entry case with many report entries. A public alias names a large inferred
row; composition never creates `Settled<Settled<R, C1>, C2>`.

Ordinary call syntax is admitted only for `CompleteOnly + Erasable`. This covers
ordinary pure values, synchronous structural release, and host cleanup whose
contract makes incompleteness impossible and evidence irrelevant. Neither an
annotation nor a call-site cast may downgrade `MayBeIncomplete` or `Required`.
For `CompleteOnly + Required`, `settle` remains mandatory but its `Incomplete`
variant is statically uninhabited.

Explicit settlement is exhaustive:

```shape
match await settle query(key) {
    Complete { value, cleanup } => {
        record(cleanup)
        use(value)
    }
    Incomplete { value: Available(value), cleanup } => {
        record(cleanup)
        use_with_warning(value)
    }
}
```

For a withhold contract, the second pattern binds `ValueWithheld`, not `R`.
Compiler diagnostics identify the unhandled entry cases and owned recovery
capabilities, not merely the top-level `Incomplete` variant.

Explicit early close is `await settle close(resource)`. It consumes and disarms
the same ledger obligation and returns `Settled<Unit, ResourceCleanup>`; scope
exit cannot run a second close path.

Forwarding avoids wrapper proliferation:

```shape
async fn query_alias(key: Key) -> QueryResult settles QueryCleanup {
    return await forward settle query(key)
}
```

On complete, forwarding yields `R` and merges the closed entries into the
current aggregate. On incomplete, it may preserve a value only for an exact
tail/type-compatible transfer or an explicit typed mapping; otherwise it
withholds it. It then unwinds the current scope and flattens both reports. This
control edge is not `throw`, does not invoke `on_failure`, and cannot be caught.
Use explicit `settle` to continue with an available incomplete value.

Higher-order typing is effect-polymorphic rather than wrapper-polymorphic:

```text
map<T, U, C>(xs: Array<T>, f: fn(T) -> U settles C)
    -> Array<U> settles repeat<C>
```

Shape's existing `Result` and `?` remain domain-value propagation. Cleanup uses
different syntax and a different typed channel so `Result<Settled<...>>` is
never synthesized. If `R` is intentionally a `Result`, the explicit boundary
is simply `Settled<Result<T, E>, C>`.

## Two-Phase Ledger Integration

The accepted automatic lifecycle remains authoritative:

1. quiesce and join contained borrowers;
2. synchronously retire every exiting owner in reverse acquisition order,
   installing the emergency guard;
3. await each close under the bounded shield, then synchronously release;
4. record one `Closed` or `Incomplete` entry for every obligation; and
5. project only after the ledger has no missing entry or live exiting owner.

Deadline expiry cancels and joins the active close, releases it locally, and
records typed `TimedOut`/`OutcomeUnknown`. Remaining entries record
`CloseNotStarted` plus emergency release. No close outcome short-circuits later
obligations. Non-cooperative cleanup must run in a killable host process before
it can satisfy the close-loan contract.

Projection is fixed:

| Body/evaluator primary | Cleanup | Projection |
|---|---|---|
| `Completed(R)` | all closed | `Settled::Complete(R, report)` |
| `Completed(R)` | expected incomplete | `Settled::Incomplete(value policy, report)` |
| forwarded incomplete | outer cleanup terminal | flattened `Settled::Incomplete` |
| `Failed(f)` | any terminal report | preserve evaluator `Failed(f)` and attach typed report |
| `Cancelled(c)` | any terminal report | preserve `Cancelled(c)` and attach typed report |
| any | cleanup invariant violated | `Faulted(CleanupInvariant)` with interrupted primary/report |

Expected cleanup never becomes a primary failure. An existing body failure or
cancellation is also never replaced. The typed report remains available to an
outer total annotation, structured task owner, host, and observability sink.

## Annotations

Annotations preserve or widen `settles C`; they cannot erase it while keeping
the same callable type. Async-droppable annotation state and attempt-local
resources add entry cases to the composed contract and ordinary ledger.

The compiler gives every annotation plan a total settlement phase with an
identity default. It receives the already-typed settlement and may pass it
through, narrow an `Available<R>` by transforming `R`, or add its own cleanup
entries. It cannot remove entries, change `HasIncomplete` to `AllClosed`, mint
evidence, or access a withheld value. `after` runs only on an available `R`.

Expected incomplete cleanup does not invoke `on_failure`. A declarative
settlement policy may retry, compensate, or transfer recovery ownership only
after the prior ledger is terminal and only with explicit idempotency, dedup,
transaction, or lease/fencing authority. Generic failure recovery cannot parse
a report string or relabel `OutcomeUnknown`.

Transparent `@remote` keeps parameters and `R` but may add
`settles RemoteCallCleanup<P>`. That effect is part of callable typing even
though it is not a return wrapper.

## Cancellation And Tasks

Cleanup shields ordinary cancellation only until the host deadline. Expected
cleanup deadline is an incomplete report entry, not timeout `RuntimeFailure`.
Cancellation of the invocation itself remains evaluator control: cancelled code
does not resume merely to receive a `Settled` value.

A structured task owner can explicitly observe:

```text
TaskSettlement<R, C> =
    Returned(Settled<R, C>)
  | Failed { primary: RuntimeFailure, cleanup: TerminalCleanupReport<C> }
  | Cancelled { primary: Cancellation, cleanup: TerminalCleanupReport<C> }
  | Faulted { fault: EngineFault, cleanup: PartialCleanupReport<C> }
```

This is join-time ownership transfer, not lexical exception catching. Until the
settlement is delivered, owner cancellation wins: a provisional `R` is withheld
and its obligations are settled, while the task owner receives `Cancelled` plus
the report. Cancellation after delivery cannot revoke transferred `R`.

## Providers And Remote Calls

`RemoteDestination<P>` and `Placement<P>` remain opaque inert capabilities.
Live `RemoteSession<P>`, lease-renewal owners, transaction participants, and
provider generation pins are affine ledger obligations. No host/address
encoding enters `Settled`.

Provider-specific close causes appear only inside a typed entry such as
`SessionCleanup<P::CloseCause>`. Providers perform discovery/routing, transport,
auth, codec, negotiation, deadline, cancellation, and telemetry mechanics. The
host validates evidence and derives external certainty; provider strings cannot
upgrade `OutcomeUnknown` or construct `AllClosed`.

The explicit remote primitive remains semantically layered:

```shape
remote::call<P, Sig>(...) -> Result<R, RemoteError<P::Cause>>
    settles RemoteCallCleanup<P>
```

`RemoteError` describes invocation/delivery. `RemoteCallCleanup` describes
automatic retirement of call-owned mechanics. `await settle remote::call(...)`
therefore yields one `Settled` around the intentional domain `Result`, not an
accidental stack of cleanup wrappers.

If a provider pool accepts a session obligation with an affine transfer receipt,
the call report does not own that session; provider shutdown settles it later.
Call-local unknown cleanup never authorizes retry. Leases, durable dedup/result
replay, transactions, and status queries remain the process-loss mechanisms.

## Snapshot, VM/JIT, And Fault Boundary

Every live owner, retired state, active close, pending settlement control edge,
or affine recovery capability is a snapshot barrier in the first version.
`state.capture_all()` metadata is not a cleanup checkpoint. A terminal
`Settled` is ordinary typed data and is snapshot-safe only when `R`, all report
entries, and recovery capabilities independently satisfy snapshot rules.
Sockets, sessions, routes, credentials, provider pointers, and close futures are
never serialized.

The compiler's cleanup plan fingerprints the contract row, observation mode,
value policy, ownership transfers, and retire/close/release entries. VM bytecode
and JIT MIR reference the same plan. Both yield to one evaluator settler and
construct the same `Settled` layout and ordered trace; JIT cannot rerun in the VM
after side effects.

Expected timeout, rejection, close failure, abandonment, and process-contained
provider exit are typed incomplete entries. Only corrupted ledger/ownership,
violated total glue, invalid evidence, contained ABI panic, or equivalent engine
invariant failure may `Fault`. After process abort or host loss no in-process
settlement or abandonment report is guaranteed; only OS reclamation and durable
external protocols act.

## Misuse Prevention

1. Every initialized affine obligation has one owner and one terminal report
   entry; move/suspension transfer ownership without copying it.
2. Every contained exit runs the same two-phase ledger in VM and JIT; no source
   handler can omit or replace it.
3. `MayBeIncomplete` or `Required` contracts reject ordinary call syntax,
   wildcard discard, and effect-erasing higher-order adaptation.
4. Incomplete-value preservation requires ownership proof plus nominal semantic
   evidence; absence means `ValueWithheld`.
5. Close timeout/rejection/failure is typed data. Only invariant violation may
   fault, and expected cleanup never becomes `RuntimeFailure`.
6. Reports retain exact cause, local release, external certainty, operation
   identity, lease status, and evidence provenance without provider-authored
   certainty.
7. Forwarding flattens reports and cannot invoke failure recovery, retry unknown
   outcomes, or lose affine recovery capabilities.
8. Cancellation, snapshot barriers, provider transfer, and process-loss limits
   remain explicit; no boolean `closed` or unchecked `unwrap` collapses them.

## Examples And Tradeoffs

Value-preserving incomplete cleanup is explicit:

```shape
match await settle query(key) {
    Complete { value, cleanup } => {
        record(cleanup)
        return value
    }
    Incomplete { value: Available(value), cleanup } => {
        await recovery_queue.accept(cleanup)
        return value
    }
}
```

Correctness-critical close withholds the body value:

```shape
match await settle publish(batch) {
    Complete { value: receipt, cleanup } => commit_receipt(receipt, cleanup)
    Incomplete { value: ValueWithheld { reason }, cleanup } =>
        resolve_transaction(cleanup, reason)
}
```

The costs are a new callable effect, settlement control operator, contract-row
inference, value-independence proof, affine report handling, and report storage
until projection. Ordinary call syntax becomes intentionally unavailable for
uncertain cleanup, and generic APIs must quantify over `C`.

The benefit is a single honest boundary: automatic cleanup remains total,
expected outcomes stay typed, callers see `R` only when its contract permits,
and domain `Result`, runtime failure, cancellation, cleanup incompleteness, and
engine fault remain distinct.

## Proof Boundary

Compiler proofs cover partial initialization, affine transfer, contract-row
composition, exhaustive matches, ordinary-call rejection, value withholding,
preservation witnesses, forward-settlement flattening, and identical VM/JIT
plans. Runtime fault injection covers every retire/close/release/suspend/deadline
transition and asserts one ordered terminal entry per obligation without later
skips.

Provider proofs cover acknowledgement, rejection, dropped response,
outcome-unknown, pool transfer, affine recovery consumption, lease expiry, and
process-contained provider exit. Snapshot proofs refuse in-progress settlement
and accept only terminal snapshot-safe reports. Process-loss proofs must show
that no in-process `Settled` is promised after death.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited, and no cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-cleanup-outcome-design-settled.md`
