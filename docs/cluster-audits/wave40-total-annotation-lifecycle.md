# Wave 40J: Total Callable-Annotation Lifecycle

Date: 2026-07-10

## Decision

Compile every callable annotation application into one **total lifecycle
scope**, even when the annotation author writes only one handler. The compiler
synthesizes identity, propagation, and observation defaults so the evaluator
never asks whether a phase exists. One affine `HookState<S>` owner is installed
when the layer is entered, moved across retry or suspension, and consumed by
generated drop glue on every in-process terminal path.

This is not try/catch/finally:

- `before` chooses typed `Proceed` or `Return`.
- `after` transforms only successful `R`.
- `on_failure` chooses typed `Propagate`, `Recover`, or `Retry`.
- `Cancelled` and `Faulted` are not caught as failures.
- an optional final observer cannot recover, retry, replace outcomes, or own
  cleanup.
- destruction is an ownership obligation generated from the lifecycle state
  machine, never a callback the author may omit.

The guarantee is Rust-like and bounded: for an in-process outcome whose
containment preserves the lifecycle ledger, every entered hook-state scope is
unwound in reverse entry order and its structural destructor is invoked exactly
once. Process abort, host loss, power loss, and arbitrary remote effects are
outside that guarantee.

## Accepted Algebra

For frozen `Sig = (P0[m0], ..., Pn[mn]) -> R`, retain the accepted outcome and
hook decisions:

```text
Evaluation<R> =
    Completed(R)
  | Suspended(Suspension)
  | Failed(RuntimeFailure)
  | Cancelled(Cancellation)
  | Faulted(EngineFault)

enum HookDecision<Sig, S> {
    Proceed { args: ArgumentPack<Sig>, state: HookState<S> }
    Return  { args: ArgumentPack<Sig>, result: R, state: HookState<S> }
}

enum FailureDecision<Sig, S> {
    Propagate { failure: RuntimeFailure }
    Recover {
        args: ArgumentPack<Sig>, result: R, state: HookState<S>
    }
    Retry { attempt: InvocationAttempt<Sig>, state: HookState<S> }
}
```

`Evaluation<R>` remains evaluator/host-only. A domain `Result::Err` is a
`Completed(R)` value. Only `Failed(RuntimeFailure)` reaches `on_failure`.
`ArgumentPack<Sig>`, recovered `R`, retry permits, exact-continuation rules,
execution certainty, and placement/provider safety remain unchanged.

## Current Gap

Current wrapper lowering has no owned lifecycle state. It creates a fresh
dynamic context object and interprets array/object/null results; state
replacement can even rebuild a three-field context with two fields
(`crates/shape-vm/src/compiler/functions_annotations.rs:2917-3151`). No
runtime-hook test exercises state.

Shape does have pieces of deterministic destruction:

- compiler drop scopes emit user `DropCall`, ownership `DropLocal`, and shared
  drops in reverse order on ordinary scope exit and explicit early exits
  (`crates/shape-vm/src/compiler/helpers.rs:5815-5924,6264-6404`);
- the ownership RFC promises reverse-order drops and drops on
  `return`/`break`/`continue`
  (`docs/vision/rfc-borrow-lifetimes-ergonomics-v1.md:145-154`);
- nested-call error teardown pops frames and kind-drops live stack shares
  (`crates/shape-vm/src/executor/call_convention.rs:977-1019`); and
- user drop-body failures are contained so later emitted drops continue
  (`crates/shape-vm/src/executor/trait_object_ops.rs:743-879`).

These are not yet one total unwind system. An arbitrary `VMError` can leave via
dispatch without executing source-level scope-exit `DropCall`s; frame truncation
releases kinded heap shares but is not equivalent to running user destruction.
Async drop opcodes also permit suspension in a place where cancellation and a
second failure make exactly-once completion impossible. A lifecycle design must
close these gaps rather than add `finally` to the current wrapper.

## Semantic Types

### Owned state scope

```text
affine opaque HookState<S: DropSafe> { // hidden application/call brand
    fn value(&self) -> &S
    fn value_mut(&mut self) -> &mut S
    fn replace_value(&mut self, next: S)
}

type InitState<Sig, S> =
    fn(config: AnnotationConfig, args: &ArgumentPack<Sig>) -> S

type Before<Sig, S> = fn(
    args: ArgumentPack<Sig>, state: HookState<S>, ctx: BeforeContext<Sig>
) -> HookDecision<Sig, S>

type After<Sig, S> = fn(
    args: &ArgumentPack<Sig>, result: R,
    state: &mut HookState<S>, ctx: AfterContext<Sig>
) -> R

type OnFailure<Sig, S> = fn(
    args: ArgumentPack<Sig>, failure: RuntimeFailure,
    state: HookState<S>, ctx: FailureContext<Sig>
) -> FailureDecision<Sig, S>
```

`InitState` returns `S`; the compiler wraps it in the branded envelope before
calling `before`. `HookState` is bound to one annotation application and one
logical call. It is not cloneable, serializable by default, capturable,
storable in persistent annotation state, passable as an argument, or returnable
as `R`. `replace_value` drops the old `S` while preserving the envelope's
identity. `Proceed`, `Return`, `Retry`, and `Recover` must carry the same state
identity they received; constructing a fresh envelope is impossible.

The lifecycle scope, not a handler, owns the drop obligation. Passing state by
value into `before` or `on_failure` is an affine transfer. If the handler
returns `Proceed`/`Return`/`Retry`/`Recover`, ownership moves back to the scope.
If it returns `Propagate`, fails, or never constructs a decision, its owned
parameter unwinds locally. `after` only borrows state; the scope destroys it
after `after` returns or fails.

### `DropSafe`

The strict guarantee requires hook state to be synchronously and
non-failingly destructible:

```text
trait DropSafe {
    fn drop(self) -> Unit  // effect: NoSuspend + NoFail + NoReentry
}
```

The compiler derives this for ordinary values and for host resources whose
destructor contract is infallible and synchronous. User `Drop` code used by
hook state must prove the same effect. An async-only, fallible, target-reentrant,
or unclassified destructor makes the annotation definition a compile error.

Until Shape has effect checking strong enough to prove this, total lifecycle
must be gated to compiler-known structural drops and trusted host resource
types. Treating a contained drop error as success would be a weaker model. If a
supposed `DropSafe` destructor nevertheless fails, continue remaining
structural drops, then surface `Faulted(CleanupInvariant)` with the original
outcome attached.

## Synthesized Completeness

Every layer compiles to a `TotalHookPlan<Sig, S>` with all slots populated:

```text
state type omitted    => S = Unit
initializer omitted   => Unit::default(), or S::default() when S: Default
before omitted        => Proceed { args, state }
after omitted         => result
on_failure omitted    => Propagate { failure }
observer omitted      => no-op observation sink
```

If a declared `S` has neither an explicit initializer nor `Default`, compilation
fails. No null/uninitialized state is permitted.

Compile-time validation requires:

1. exactly one state type and initializer per annotation definition;
2. all handler branches return the required decision/value;
3. every transfer carries the exact `HookState` identity and frozen `Sig`;
4. `Propagate` leaves state to local deterministic drop;
5. `Retry` has a finite evaluator-owned budget and sealed attempt;
6. no state loan crosses a retry, suspension, spawned task, or handler return;
7. all terminal CFG edges reach generated `DropHookState`; and
8. VM bytecode and JIT MIR lower from the same total plan and drop ledger.

An annotation may remain terse because defaults are synthesized, but its
compiled lifecycle is never partial.

## Lifecycle State Machine

One layer has these runtime states:

```text
Unentered
  -> Initializing
  -> Entered { state, args }
  -> RunningAttempt { state, args, attempt }
  -> Suspended { state, continuation }
  -> HandlingFailure { state, failure }
  -> BackoffOrRetry { state, attempt }
  -> Completing { state, args, result }
  -> Unwinding { state, terminal }
  -> Closed { terminal, state_dropped = true }
```

The owner invariant is:

```text
nonterminal entered state: exactly one owner of StateId
transfer edge:             move owner, do not drop
terminal edge:             consume owner with DropHookState
Closed:                    no owner and one recorded drop
```

`StateId` and the drop ledger are evaluator metadata, not source values. Static
affine checking proves normal paths; runtime identity guards catch VM/JIT,
resume, or provider bugs that would double-transfer or double-drop.

### Transition table

| Event | Transition | State action |
|---|---|---|
| `before -> Proceed` | `Entered -> RunningAttempt` | move |
| `before -> Return` | `Entered -> Completing` | move |
| attempt `Completed(R)` | `RunningAttempt -> Completing` | move |
| attempt `Suspended` | `RunningAttempt -> Suspended` | move into suspension |
| resume | `Suspended -> RunningAttempt` | move out of suspension |
| attempt `Failed` | `RunningAttempt -> HandlingFailure` | move |
| `FailureDecision::Retry` | `HandlingFailure -> BackoffOrRetry` | move; no drop |
| retry/backoff suspends | `BackoffOrRetry -> Suspended` | move |
| `FailureDecision::Recover` | `HandlingFailure -> Completing` | move |
| `FailureDecision::Propagate` | `HandlingFailure -> Unwinding(Failed)` | drop |
| root cancellation | any entered state -> `Unwinding(Cancelled)` | drop |
| contained engine fault | any entered state -> `Unwinding(Faulted)` | structural drop |
| `after` returns | `Completing -> Unwinding(Completed)` | drop |
| handler fails | handler state -> `Unwinding(Failed)` | drop |

Suspension never duplicates or drops state. In-memory resume restores the same
owner. Durable snapshot/resume requires `S: SnapshotSafe` plus serializable
attempt/provider state; otherwise snapshot refuses at a clean barrier. It must
never serialize raw state bits, omit an owner, or recreate state with
`Default` on resume.

Retry first unwinds and drops every inner layer and attempt-local frame from
the failed attempt. The retrying layer's state survives and moves into the
sealed attempt/backoff continuation. An outer retry enters inner layers afresh,
so each new inner application gets a new `StateId`; the outer state keeps its
original identity.

## Outcome Semantics

### `Completed`

For normal body completion, before short-circuit, or failure recovery, run the
same layer's `after`, then drop its state, then continue success unwinding to
the next outer layer. A recovered `R` is not a special cleanup path.

### `Failed`

The nearest entered `on_failure` runs first. `Retry` transfers state;
`Recover` rejoins `Completed`; `Propagate` drops that layer's state before the
outer failure hook runs. A replacement failure does not change drop order or
execution certainty.

### `Cancelled`

Cancellation bypasses `on_failure` and `after`. The evaluator cancels active
child/provider work, then structurally unwinds entered lifecycle scopes deepest
first. Remote cancellation remains best effort and cannot delay local state
drop indefinitely or imply remote rollback.

### `Faulted`

A contained engine fault runs no user hook or source observer. The containment
owner executes the precomputed structural drop ledger in reverse order and
returns `Faulted`. Only `DropSafe` destruction is eligible. If the fault means
the state ledger itself is untrustworthy, the nearest native containment
boundary drops its own Rust/host owners; no stronger source guarantee is
truthful.

An uncontained panic, abort, process kill, or machine loss cannot promise
destruction. Exactly-once external effects are never inferred from local drop.

## Handler Failures

- Initializer or `before` failure drops any constructed state and frame locals;
  the layer never becomes active, and an entered outer layer may handle the
  failure.
- `after` failure is not caught by the same layer. Its borrowed state returns
  to the lifecycle owner, which drops it before the failure reaches an outer
  layer.
- `on_failure` failure is not recursively caught by itself. Its owned state
  unwinds before the new failure reaches an outer layer.
- A malformed/missing decision is a compile error, not a runtime default.
- A violated `DropSafe` contract is an engine cleanup fault. Remaining drops
  continue; the final evaluator outcome is `Faulted`, never a fabricated `R`.
- Observation failure is contained and recorded after cleanup; it cannot
  replace the primary terminal outcome.

This prevents the classic partial-finally bug where cleanup throws, later
cleanup is skipped, and the original failure disappears.

## Nested Order

For source layers `A` outer and `B` inner:

```text
success:
  A.before -> B.before -> body -> B.after -> drop(B) -> A.after -> drop(A)

B and A propagate:
  B.on_failure -> drop(B) -> A.on_failure -> drop(A)

B recovers:
  B.on_failure -> B.after -> drop(B) -> A.after -> drop(A)

A retries:
  failed inner layers drop -> preserve A -> enter fresh B -> body -> ...

cancel/fault:
  stop policy hooks -> drop deepest entered state through outermost
```

Dynamic entry order, not annotation names, drives unwind. Inner layers skipped
by an outer before `Return` were never entered; the returning layer still runs
its own `after` and drops its state. The traces omit observation; when enabled,
it runs immediately after that layer's drop and before unwinding outward.

## Optional Final Observation

The evaluator always emits one host-level lifecycle event after state drop. An
annotation may additionally declare:

```text
type ObserveFinally = fn(summary: LifecycleSummary) -> Unit
    where effects = NoSuspend + NoControl
```

`LifecycleSummary` contains only redacted observation data: terminal class,
attempt count, recovery flag, duration, and diagnostic/correlation IDs. It does
not contain `R`, an affine failure token, arguments, state, credentials, or a
continuation.

The source observer runs after cleanup on healthy `Completed`, `Failed`, and
`Cancelled` unwinds. It is normally skipped for `Faulted`; the host sink still
receives the fault event. Observer absence or failure cannot affect state drop,
result/failure selection, retry, cancellation, or outer-layer order. This is an
observation hook, not `finally` cleanup.

## Ergonomics

A simple timing annotation can omit all policy handlers it does not need:

```shape
annotation timed(metric: Metric) {
    state(args) { TimerState::start(metric) }

    after(args, result, state, ctx) {
        state.value_mut().mark_success()
        result
    }

    observe_finally(summary) { metrics.observe(summary) }
}
```

The compiler supplies identity `before` and propagating `on_failure`. The
`TimerState` destructor closes its local timing span; the observer only reports
the already-closed summary.

A retry annotation moves, rather than recreates, its state:

```shape
on_failure(args, failure, state, ctx) {
    if state.value().used >= state.value().max {
        return FailureDecision::Propagate { failure }
        // owned state drops while returning
    }
    state.value_mut().used += 1
    FailureDecision::Retry {
        attempt: ctx.target.retry_not_executed(failure, args, state.delay()),
        state,
    }
}
```

No `finally` handler is needed for either example.

## Limits and Required Mechanisms

- Deterministic drop guarantees destructor invocation, not successful external
  cleanup. Local OS handles may use infallible close-on-drop semantics; remote
  effects require leases, transactions, deduplication, or server ownership.
- Async cleanup cannot be an exactly-once destructor. Use explicit
  `close().await`/commit/rollback in a structured scope, optionally shielded by
  a bounded deadline; retain a lease as the process-loss backstop.
- Persistent circuit-breaker/cache state is not `HookState`; it belongs to an
  explicit synchronized persistent store.
- Suspension can retain state indefinitely. Cancellation budgets and provider
  deadlines must remain host-enforced.
- Cyclic/shared resources still need their ordinary GC/ownership rules; the
  lifecycle does not invent cycle collection.
- Source observation is best effort. Host telemetry is the authoritative
  terminal record.
- This design requires one evaluator unwind implementation shared by VM and
  JIT. Layering it beside bytecode-only drop emission would remain partial.

## Proof Boundary

Compiler proofs should inspect total plans and ownership CFGs: all synthesized
defaults, uninitialized state rejection, exhaustive decisions, exact state
identity, move-after-use rejection, terminal drop coverage, non-drop transfer
on retry/suspension, `DropSafe` enforcement, finite retry budget, and matching
VM/MIR ledgers.

Runtime proofs should instrument `StateId` and assert one drop for every
entered layer across direct/short-circuit completion, propagate/recover/retry,
failure in each handler, suspension/resume, cancellation during body/backoff,
contained fault, and observer failure. Nested tests must assert the exact order
listed above and prove fresh inner state on outer retry.

Ownership/Miri proofs should cover state moves through suspension records,
retry plans, closures, and frame unwind without leaks, stale references, or
double drop. VM/JIT parity must compare semantic lifecycle traces, not only
final text. Snapshot tests must either round-trip one owner or refuse before
persistence.

No production, test, book, script, `CONTEXT.md`, or `AGENTS.md` file was
edited. No cargo, test, build, extraction, or book command was run.
