# Wave-40E Minimal Algebraic Failure-Hook Design

Date: 2026-07-10

Scope: interface design only. This note does not authorize implementation or
claim that current evaluator paths preserve the required failure evidence.

## Decision

Add one source-visible decision enum with three variants:

```shape
enum FailureDecision<Sig, S> {
    Propagate { failure: RuntimeFailure }
    Recover {
        args: ArgumentPack<Sig>,
        result: ReturnOf<Sig>,
        state: S,
    }
    Retry {
        attempt: InvocationAttempt<Sig>,
        state: S,
    }
}
```

- `Propagate` sends the original failure or a structured replacement outward.
  A separate replacement variant adds no behavior.
- `Recover` completes normally with a value checked against the unchanged
  return type `R = ReturnOf<Sig>`.
- `Retry` asks the evaluator to run the exact next-inner continuation through
  a sealed attempt plan. The plan may use the same or signature-compatible new
  arguments and the same or another placement.

There is no source `Evaluation<R>`, implicit `Result`, catch-all exception,
remote-specific decision, or `retry_anyway` boolean. Supporting values are
opaque capabilities or records, not more decision enums.

The generic hook is placement-neutral. `Placement<Sig>` can denote local,
isolated, accelerated, or remote execution. It exposes no hostname, port, URI,
socket address, protocol, credential, or wire bytes.

## Current Gap

Callable annotations currently have no `on_failure`: the AST has six handler
kinds (`crates/shape-ast/src/ast/functions.rs:242-259`), and the parser accepts
only those names (`crates/shape-ast/src/parser/extensions.rs:127-159`).
Callable hooks also still use the dynamic array/object/null protocol described
in `docs/cluster-audits/wave40-annotation-hook-type-model.md:24-68`.

Current remoting is address-shaped. Internal calls accept `addr: string`, and
`@remote(addr)` forwards it directly
(`crates/shape-runtime/stdlib-src/core/remote.shape:72-115,165-190`). The
module documents bare `host:port` and a TLS URI at `:7-11`. The existing
`WireTransportProvider` only chooses TCP or QUIC
(`crates/shape-wire/src/transport/factory.rs:83-98`); it does not cover
discovery, routing, destination encoding, auth, codec, negotiation, deadlines,
cancellation, or observability.

With no compatibility constraint, replace address parameters rather than add a
new address spelling:

```shape
@remote(placement: Placement<Sig>)
fn target(...) -> R

remote::call(placement, target, args...) -> Result<R, RemoteError>
remote::call_async(placement, target, args...)
    -> Future<Result<R, RemoteError>>
```

`@remote` still preserves `R` and fails through the evaluator. Explicit
calls remain recoverable `Result` surfaces. Placement capabilities come from
deployment configuration or provider-backed discovery, never source addresses.

## Core Interface

For frozen `Sig = (P0[m0], ..., Pn[mn]) -> R`:

```shape
opaque ArgumentPack<Sig>
opaque HookTarget<Sig>
opaque InvocationAttempt<Sig>
opaque Placement<Sig>
opaque UnknownRetryPermit<Sig>

// Handler-scoped and affine, not an application value.
affine RuntimeFailure

struct FailureContext<Sig> {
    target: HookTarget<Sig>,
    current: AttemptView<Sig>,
    budget: RetryBudgetView,
}

type OnFailure<Sig, S> = fn(
    args: ArgumentPack<Sig>,
    failure: RuntimeFailure,
    state: S,
    ctx: FailureContext<Sig>,
) -> FailureDecision<Sig, S>
```

Handler roles are positional and compiler-known. Parameter names carry no
semantics.

### Immutable arguments

```shape
args.get<I>() -> ParamAt<Sig, I>
args.replace<I>(value: ParamAt<Sig, I>) -> ArgumentPack<Sig>
```

`ArgumentPack<Sig>` has one position per runtime parameter and retains type,
pass mode, and authoritative `NativeKind`. Replacement is functional. There
is no dynamic heterogeneous index, arity change, array flattening, raw-bit
view, or cross-signature conversion. Ordinary ownership rules still apply.

The failed attempt's effective pack enters `on_failure`. A later failure sees
the pack chosen by its retry. On success, the same layer's `after` sees the
successful attempt's pack.

### Exact continuation and attempts

For layer `i`, `ctx.target` is exactly the next-inner continuation: layer
`i + 1` through the implementation. It cannot name another callable or
bypass an inner annotation.

Inside `on_failure`, it cannot be called directly. It can only mint a sealed
attempt tied to the current failure frame:

```shape
ctx.target.retry_not_executed(
    failure, args, at: placement, after: delay,
) -> InvocationAttempt<Sig>

ctx.target.retry_with_permit(
    failure, args, at: placement, after: delay,
    permit: UnknownRetryPermit<Sig>,
) -> InvocationAttempt<Sig>
```

The first constructor accepts only `DefinitelyNotExecuted`. The second is
required for `OutcomeUnknown` or `ExecutionStarted`. Both check that the
failure belongs to this invocation and the placement accepts this `Sig`. An
invalid plan is not run; the original failure propagates with a structured
policy-denied diagnostic.

A before hook establishes initial placement without invoking the target:

```shape
ctx.target.at(placement).proceed(args, state)
```

This is a placement-bearing proceed plan in the accepted before-hook model, not
another failure decision. It lets this layer initialize `S` before target
execution and receive a target failure.

### Failure token and state

`RuntimeFailure` exposes typed cause, origin, execution certainty, diagnostic
details, and attempt history. Handlers never parse display text. The token is
not freely constructible, serializable, storable in `S`, capturable, accepted
by an ordinary function, or returnable as `R`. It can escape only through
`Propagate`.

```shape
failure.with_context(code, message, details) -> RuntimeFailure
failure.replace(spec: FailureSpec, cause: failure) -> RuntimeFailure
```

Replacement retains the original cause and least-safe certainty. It cannot turn
`OutcomeUnknown` into `DefinitelyNotExecuted`, construct `EngineFault`, or
relabel cancellation.

`S` belongs to one annotation layer and logical call. `Retry` carries its
updated value; `Recover` carries it into normal success unwinding and the
same-layer `after`; `Propagate` drops it. Retry count and pool index fit in
`S`. Circuit health, rate limits, and dedup records require explicit
persistent provider or application stores.

## Evaluator And Composition

The runtime-only model remains:

```text
Evaluation<R> =
    Completed(R) | Suspended | Failed(RuntimeFailure)
  | Cancelled(Cancellation) | Faulted(EngineFault)
```

Only `Failed(RuntimeFailure)` enters `on_failure`.

- A domain `Result::Err` is `Completed(R)`; it follows the success path.
- Suspension is resumed internally. Retry backoff may suspend without changing
  `R` to `Future<R>`.
- Cancellation bypasses failure hooks and remains prompt during backoff or
  provider work.
- `EngineFault` bypasses user recovery and is handled at containment/host
  projection.

The nearest entered layer handles first. `Propagate` continues outward.
`Recover` resumes normal success unwinding through its own and outer
`after` hooks. `Retry` re-enters only that layer's next-inner continuation;
it does not rerun its `before`. A later failure re-enters the same
`on_failure` with updated state. An outer retry enters inner layers afresh.

A layer does not recursively catch a failure raised by its own `before`,
`after`, or `on_failure`; an entered outer layer may receive it. The
evaluator also enforces a hard per-layer attempt budget. Missing-blob resupply
after a proven pre-execution rejection remains a hidden protocol continuation,
not a semantic retry.

## Retry Safety

Cause and certainty are independent:

- `DefinitelyNotExecuted`: retry needs no unknown-outcome permit.
- `OutcomeUnknown`: retry requires explicit idempotency or real deduplication.
- `ExecutionStarted`: the same gate applies because side effects may remain.

`UnknownRetryPermit<Sig>` has only two trusted constructors:

```shape
idempotent(contract: IdempotencyContract<Sig>)
    -> UnknownRetryPermit<Sig>
deduplicated(guarantee: DedupGuarantee<Sig>)
    -> UnknownRetryPermit<Sig>
```

An idempotency contract is an explicit domain assertion over target and pack
semantics. Shape cannot prove arbitrary external effects, but the assertion is
visible and reviewable.

A provider-issued dedup guarantee binds principal, target identity, canonical
argument/capture fingerprint, retention window, receiver epoch/crash promise,
and all eligible placements. A per-worker cache cannot permit failover to a
different worker. Current `RemoteCallId` is only a cancellation/correlation
token (`crates/shape-vm/src/remote.rs:125-134`), and current receivers do not
replay results, so they cannot issue this guarantee.

## Placement Provider Module

`Placement<Sig>` is an unforgeable capability minted by discovery/routing. It
contains a signature witness and opaque reference to a configured provider
graph. Source can pass it back to placement operations but cannot project
provider mechanics from it.

Conceptual host-side interfaces are:

```text
DiscoveryProvider.discover(selector, target, constraints) -> CandidateSet<Sig>
RoutingProvider.choose(candidates, history) -> Placement<Sig>
DestinationEncodingProvider.encode(placement) -> EncodedDestination
AuthProvider.materialize(principal, placement, scope) -> AuthMaterial
CodecProvider.encode_call(signature, target, args, metadata) -> EncodedCall
CodecProvider.decode_outcome(signature, bytes) -> DecodedOutcome
ProtocolProvider.negotiate(placement, capabilities) -> ProtocolSession
TransportProvider.submit(session, destination, auth, call, controls)
    -> SubmissionEvidence
DeadlineProvider.budget(logical_call, history) -> AttemptDeadline
CancellationProvider.attach(logical_call, provider_op) -> CancellationLink
ObservabilityProvider.observe(lifecycle_event)
```

These are provider seams below callable hooks. Embedders can replace discovery,
routing, address encoding, transport, auth, codec, negotiation, deadlines,
cancellation, and observability independently or as one deep provider module.
A default adapter may read `host:port` from deployment configuration, but the
encoding never appears in `@remote`, `remote::call`, `FailureDecision`, or
`Placement<Sig>`.

Providers report evidence, not retry safety. The invocation orchestrator above
them derives certainty from validated milestones such as pre-submission
rejection, receiver admission, execution start, and terminal response. A
provider cannot classify ambiguity as safe, weaken a signature check, bypass a
budget, or mint an idempotency contract.

The codec receives the frozen signature and authoritative kinds; it never
infers kinds from payload bits. Negotiation may choose an encoding, but cannot
change `Sig`, argument modes, target identity, or `R`.

## Shape Examples

The spelling is illustrative; the semantics are the proposal.

### `@fallback`

```shape
annotation fallback<Sig>(to: Callable<Sig>) {
    before(args, ctx) { ctx.target.proceed(args, ()) }

    on_failure(args, failure, state, ctx) {
        if failure.certainty.is_definitely_not_executed() {
            FailureDecision::Recover {
                args: args,
                result: to.call_pack(args),
                state: state,
            }
        } else {
            FailureDecision::Propagate {
                failure: failure.with_context(
                    "fallback.unsafe",
                    "fallback refused because the first call may have run",
                    {},
                ),
            }
        }
    }
}

@fallback(local_projection)
fn project(batch: Batch) -> Projection
```

The fallback is checked as `Callable<Sig> -> R`. Signature compatibility does
not prove semantic equivalence, so outcome-unknown fallback needs a separate
explicit operation-equivalence/idempotency contract.

### `@retry`

```shape
annotation retry<Sig>(
    max: int,
    backoff: Backoff,
    unknown: Option<UnknownRetryPermit<Sig>> = None,
) {
    before(args, ctx) {
        ctx.target.proceed(args, RetryState { used: 0 })
    }

    on_failure(args, failure, state, ctx) {
        if state.used >= max {
            return FailureDecision::Propagate {
                failure: failure.with_context(
                    "retry.exhausted", "retry budget exhausted", {}
                ),
            }
        }

        let delay = backoff.for_attempt(state.used + 1)
        let attempt = if failure.certainty.is_definitely_not_executed() {
            ctx.target.retry_not_executed(
                failure, args,
                at: ctx.current.placement, after: delay,
            )
        } else {
            match unknown {
                Some(permit) => ctx.target.retry_with_permit(
                    failure, args,
                    at: ctx.current.placement, after: delay,
                    permit: permit,
                )
                None => return FailureDecision::Propagate { failure: failure }
            }
        }

        FailureDecision::Retry {
            attempt: attempt,
            state: RetryState { used: state.used + 1 },
        }
    }
}

@retry(max: 3, backoff: exponential(20.milliseconds))
fn acquire_local_lease(key: string) -> Lease

@retry(
    max: 2,
    backoff: fixed(50.milliseconds),
    unknown: idempotent(read_only_query),
)
fn query_snapshot(key: string) -> Snapshot
```

A typed `PackRewrite<Sig>` parameter can support different arguments. For
example, a specialization may refresh position 0 with
`args.replace<0>(credential)`; it cannot change arity or slot type.

### `@remote_pool`

```shape
annotation remote_pool<Sig>(
    pool: PlacementPool<Sig>,
    unknown: Option<UnknownRetryPermit<Sig>> = None,
) {
    before(args, ctx) {
        ctx.target.at(pool.first(ctx.target, args)).proceed(
            args, PoolState { next: 1 }
        )
    }

    on_failure(args, failure, state, ctx) {
        let next = match pool.next(
            ctx.target, args, ctx.current.placement, state.next,
        ) {
            Some(placement) => placement
            None => return FailureDecision::Propagate { failure: failure }
        }

        let attempt = if failure.certainty.is_definitely_not_executed() {
            ctx.target.retry_not_executed(
                failure, args, at: next, after: 0.milliseconds
            )
        } else {
            match unknown {
                Some(permit) => ctx.target.retry_with_permit(
                    failure, args, at: next, after: 0.milliseconds,
                    permit: permit,
                )
                None => return FailureDecision::Propagate { failure: failure }
            }
        }

        FailureDecision::Retry {
            attempt: attempt,
            state: PoolState { next: state.next + 1 },
        }
    }
}

@remote_pool(analytics_workers)
fn compute(batch: Batch) -> Summary

@remote_pool(
    analytics_workers,
    unknown: idempotent(pure_batch_transform),
)
fn compute_pure(batch: Batch) -> Summary
```

`analytics_workers` is a provider-issued `PlacementPool<Sig>`, not a list of
addresses. The first form fails over only after proven non-execution. The
second explicitly permits duplicates.

A deduplicated form may use
`deduplicated(order_workers.shared_replay_guarantee(...))` only after a
provider proves cross-placement admission, fingerprinting, retention, and
result replay. Today's protocol cannot supply that capability.

## Hidden Work And Misuse Prevention

The module hides VM/JIT failure capture, exact continuation re-entry, unwind
state, call-local state ownership, attempt history and budgets, cancellable
backoff, provider dispatch, certainty classification, wire/blob transfer,
ABI/kind checks, dedup admission/replay, secret redaction, diagnostics, and host
projection. `Evaluation<R>` remains internal throughout.

Compiler/evaluator checks prevent:

- wrong-signature packs, argument modes/kinds, or recovered `R`;
- attempts against anything but the exact next-inner target;
- direct target calls that bypass retry validation;
- forged, expired, wrong-principal, or wrong-signature placements;
- unknown/started retries without a bound permit;
- cross-placement use of a per-placement dedup guarantee;
- certainty laundering or use of call IDs as idempotency keys;
- storing/capturing `RuntimeFailure` or exposing `Evaluation<R>`;
- delivery of cancellation/engine faults to `on_failure`; and
- unbounded retries or silently nested incompatible placement owners.

Two placement-owning annotations should be a plan error unless both declare
composable placement semantics. Non-placement annotations compose normally.

## Tradeoffs And Follow-Up

Three variants keep the interface deep: sealed attempts carry argument,
placement, delay, and retry-policy details without a `RetrySame` /
`RetryWithArgs` / `RetryAt` variant explosion. The cost is special handler
typing, an affine token, continuation re-entry, hidden suspension, and provider
evidence normalization.

User-declared idempotency can be false because Shape has no general effect
proof. It is still safer than implicit retry. Receiver dedup is stronger but
requires real protocol/storage support. Call-local `S` deliberately cannot
provide durable circuit breaking. Failure recovery is not cleanup;
cancellation and engine faults require structured ownership and containment.

Ordered follow-up is: ratify `RuntimeFailure` and certainty evidence; add
`on_failure` and sealed attempts to callable hook planning; define placement
providers independently; implement one VM/JIT invocation orchestrator; prove
composition and retry gates; then add real-socket loss, alternate-placement,
deadline, and cancellation regressions. Deduplicated retry stays unavailable
until a negotiated cross-placement replay guarantee exists.

Only `docs/cluster-audits/wave40-failure-hook-design-algebraic.md` changed. No
production, test, book, script, `CONTEXT.md`, or `AGENTS.md` file was edited.
No build or test command was run.
