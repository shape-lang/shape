# Wave 40G: Declarative Failure-Recovery Policy Interface

Date: 2026-07-10

## Recommendation

Add a generic `@recover(plan)` annotation whose argument is a statically
specialized `RecoveryPlan<Sig, Operation, State>`. A plan is declarative data:
it declares a finite attempt/deadline budget, backoff, operation choices,
disjoint cause-and-certainty rules, typed state reductions, duplicate-safety
gates, and one exhaustive terminal action. The compiler validates and lowers
the plan to a finite graph; the runtime owns dispatch, waiting, retry, and
termination. Policy code cannot call the target or implement its own loop.

Keep remoting as one operation specialization, not as the generic failure-hook
API. `@remote(on: placement)` selects a remote operation and remains
transparent to the function signature. An outer `@recover(remote_policy)` may
wrap it, but the same plan model can wrap other declared recoverable operations.

Remote destinations must be opaque provider capabilities:

```text
opaque Placement<P: RemoteProvider>
opaque PlacementPool<P: RemoteProvider>
```

Neither type requires a string, host, port, URL, or public address projection.
Discovery, routing, address encoding, transport, auth, codec, negotiation,
deadlines, cancellation, and observability are provider interfaces beneath one
shared `RemoteDispatch`. Frozen signatures, argument kinds, certainty,
logical-call identity, idempotency/deduplication, and policy budgets stay above
providers in compiler/runtime-owned semantic machinery.

Do not expose `Evaluation<R>` as a general Shape value. Recovery is a
compiler-recognized invocation plan over internal outcomes. Ordinary Shape
code still sees only its declared `R`, `Result<R, E>`, or future thereof.

## Current Boundary

The accepted Wave-40 audits establish that `R` describes normal completion,
runtime failure is non-returning, and callable hooks preserve a frozen `Sig`
using `ArgumentPack<Sig>`. This design consumes those decisions.

The current remote seam should be replaced, not wrapped:

- `RemoteDispatcher` takes `addr: &str` in three separate raising/result/async
  methods (`crates/shape-runtime/src/module_exports.rs:120-161`).
- The stdlib dunders and `@remote` take `addr: string`, with `host:port` in the
  public example (`crates/shape-runtime/stdlib-src/core/remote.shape:85-115,
  165-190`).
- Sender code parses TCP/TLS destinations and flattens mechanics before an
  honest certainty classification exists
  (`crates/shape-vm/src/executor/builtins/remote_builtins.rs:878-960`).
- An imperative failure hook calling `ctx.target` cannot prove boundedness,
  duplicate safety, or call-local state ownership.

There is no compatibility constraint. Remove the opinionated address carrier
and duplicated dispatcher projections at this boundary.

## Designs Pressure-Tested

An **ordered rule table** is concise but overlapping rules create hidden
precedence; a broad transport rule can shadow an uncertainty gate. An
**explicit state graph** handles auth refresh and degradation, but user-authored
cycles are unbounded retry loops under another name. A **fixed retry envelope**
is easy to validate but cannot express typed argument changes, placement
selection, or state-dependent decisions without callback escape hatches.

The recommended synthesis is a mandatory bounded envelope plus an unordered,
statically disjoint rule set. The compiler lowers rules to graph edges, but
users cannot author cycles. Every retry edge consumes the same runtime-owned
attempt and elapsed budgets. A mandatory terminal clause handles unmatched
causes and budget exhaustion.

## Semantic Types

The following is interface pseudocode, not proposed parser syntax:

```text
type Sig = (P0[m0], P1[m1], ..., Pn[mn]) -> R

opaque ArgumentPack<Sig>
opaque Target<Sig>
opaque RecoveryPlan<Sig, Op: AttemptModel<Sig>, State>

trait AttemptModel<Sig> {
    type Choice
    type ChoiceSet
    type Cause: FailureCause
    type DuplicateGate: DuplicateSafety<Sig>
}

struct FailureFacts<Cause> {
    cause: Cause,
    certainty: ExecutionCertainty,
    phase: FailurePhase,
    attempt: PositiveInt,
    elapsed: Duration,
}

enum ExecutionCertainty {
    DefinitelyNotExecuted,
    OutcomeUnknown,
    ExecutionStarted,
}

struct AttemptBudget {
    max_attempts: BoundedInt<1, MAX_POLICY_ATTEMPTS>,
    max_elapsed: BoundedDuration,
    per_attempt: BoundedDuration,
}

enum PlanAction<Sig, Op, State> {
    Retry {
        choice: ChoiceExpr<Op>,
        args: ArgumentTransform<Sig>,
        state: StateReduction<State, Op::Cause>,
        gate: RetryGate<Op::DuplicateGate>,
    },
    Fallback { target: Target<Sig>, args: ArgumentTransform<Sig> },
    Recover { value: RecoveryFunction<Sig, State, Op::Cause> },
    Propagate,
}
```

`FailureFacts` is sealed plan input, not a general source outcome. It has no
completion value, continuation, target invocation method, `Result` conversion,
or public constructor. Suspension is runtime control. Owner cancellation and
engine faults bypass recovery.

`AttemptModel` keeps the hook generic. A local operation can expose a singleton
choice and program-failure causes. A remote operation exposes placements and
remote causes. Queue, database, or foreign-call providers can define their own
models without adding remote fields to `@recover`.

## Plan Contract

A plan specializes after the target signature freezes:

```shape
policy bounded_recovery<Sig, Op, S> for Op: AttemptModel<Sig> {
    state: S = initial_state
    budget { attempts: 4, elapsed: 3.seconds, per_attempt: 1.second }
    backoff: exponential(
        initial: 20.ms, factor: 2, maximum: 200.ms, jitter: full,
    )

    rule retry_pre_execution {
        when certainty == DefinitelyNotExecuted
          and cause in retryable_pre_execution
        then retry(
            choice: next_choice,
            args: preserve,
            state: record_failure,
            gate: no_duplicate_possible,
        )
    }

    terminal: propagate
}
```

This is not an executable handler body. Selectors and actions come from
restricted plan namespaces. A policy cannot invoke `ctx.target`, sleep, start
a task, open a connection, mutate global state, or recurse. Rules must be
disjoint over cause, certainty, and compiler-provable state guards.

Every plan declares one terminal action:

- `fallback to target` invokes a same-`Sig` target once. Local fallback is
  never implicit for `@remote`.
- `recover with fn` invokes a function returning exactly `R` once with the
  final pack, call-local state, and a read-only terminal failure summary.
- `propagate` preserves structured failure. The transparent surface projects
  it to evaluator failure; an explicit result API projects eligible operation
  failures to its declared `Err` value.

A terminal function's own failure propagates. It does not restart the plan.
Nested plans are valid only when their bounds are charged to the outer budget.

## Arguments, State, And Duplicate Safety

Argument transforms have this exact type:

```text
plan fn(ArgumentPack<Sig>, PlanStateView) -> ArgumentPack<Sig>
```

They cannot change arity, pass mode, slot type, or authoritative kind; convert
to a heterogeneous array; or produce another signature's pack. Nested arrays
remain one argument. Signature safety does not prove semantic equivalence:

- A transform is freely usable only after `DefinitelyNotExecuted`.
- A deduplicated uncertain retry preserves target, arguments, principal, and
  semantics-affecting options under the canonical fingerprint.
- A changed uncertain retry additionally needs an operation-owner-issued
  `EquivalentRetry<Sig, Transform>` capability.
- Reusing a dedup key with a changed fingerprint is terminal rejection.

There is no `idempotent: true` boolean. Retry edges carry one of:

```text
NoDuplicatePossible(DefinitelyNotExecutedWitness)
Idempotent(IdempotencyProof<Sig>)
Deduplicated(DedupLease<Provider, Sig>)
Equivalent(DuplicateSafety<Sig>, EquivalentRetry<Sig, Transform>)
```

`IdempotencyProof` is an auditable operation-owner assertion with a declared
scope; the compiler cannot derive arbitrary external-effect semantics. A
`DedupLease` is negotiated once per logical call and binds provider brand,
principal, dedup domain, receiver epoch, retention, key, target, and request
fingerprint. A call ID or user string is not a lease. Cross-placement uncertain
retry requires one live dedup domain or an adequate idempotency proof.

Each invocation also creates one private policy state value. Pure, total state
reducers may update it after attempts, but cannot perform I/O or dispatch.
Runtime-owned state remains separate: logical-call identity, attempt count,
elapsed time, chosen placements, evidence ledger, dedup lease, deadline, and
cancellation. State is never shared across calls or annotation layers. Async
backoff state must satisfy snapshot carriers or establish an explicit snapshot
barrier.

## Remote Operation And Placement Pools

Remote execution specializes the generic model:

```text
struct RemoteAttempt<P: RemoteProvider, Sig>: AttemptModel<Sig> {
    type Choice = Placement<P>
    type ChoiceSet = PlacementPool<P>
    type Cause = RemoteCause<P::ProviderCause>
    type DuplicateGate = RemoteDuplicateGate<P, Sig>
}
```

The original "host pool" requirement is represented by `PlacementPool<P>`.
A pool may denote one node, replicas, a scheduler queue, a region, an
in-process worker, or provider-defined constraints. Policies may select
`same`, `next_untried`, `rediscover`, or a provider-supplied routing strategy;
they cannot inspect or construct addresses. The host injects placements, or a
typed provider module supplies values such as `placements.market_data`.

Placement is both routing intent and authority. It is non-forgeable,
provider-branded, scope-attenuated, and non-serializable unless its provider
defines a secure transfer form. Discovery may narrow service, tenant, region,
permission, and protocol scope but cannot broaden it.

## Provider Interfaces

All provider values share opaque brand `P`, so a route, encoded destination,
session, auth context, or dedup lease cannot cross provider families:

```text
DiscoveryProvider<P>:
  resolve(Placement<P>, DiscoveryContext) -> ProviderStep<RouteSet<P>>
RoutingProvider<P>: choose(RouteSet<P>, RoutingPolicy<P>, AttemptHistory)
  -> ProviderStep<Route<P>>
AddressEncodingProvider<P>: encode(Route<P>)
  -> ProviderStep<EncodedDestination<P>>
AuthProvider<P>: authorize(AuthSite<P>, CallAuthority)
  -> ProviderStep<AuthEvidence<P>>

CodecProvider<P>:
  encode_call<Sig>(TargetDescriptor<Sig>, ArgumentPack<Sig>, CallEnvelope)
    -> ProviderStep<EncodedCall<P>>
  decode_reply<Sig>(EncodedReply<P>, ReturnDescriptor<Sig>)
    -> ProviderStep<R>
ProtocolProvider<P>:
  negotiate(NegotiationSite<P>, ProtocolOffer, AttemptControl)
    -> ProviderStep<ProtocolSession<P>>
  validate_receipt(ProtocolReceipt<P>) -> ValidatedReceipt
TransportProvider<P>:
  open(EncodedDestination<P>, ConnectionOptions<P>, AttemptControl)
    -> ProviderStep<Session<P>>
  exchange(ExchangeContext<P>, EncodedCall<P>, AttemptControl)
    -> ProviderExchange<P>
DeadlineProvider<P>: constrain(LogicalDeadline, AttemptOrdinal)
  -> AttemptDeadline<P>
CancellationProvider<P>:
  bind(LogicalCallIdentity, Session<P>) -> CancellationLease<P>
  request(CancellationLease<P>) -> BestEffortCancellation
ObservabilityProvider<P>: record(RedactedDispatchEvent<P>)
```

These interfaces assign responsibilities, not a fixed pipeline order; each
provider supplies a bounded phase plan that may interleave them. Providers
report causes and phase evidence, never certainty. `RemoteDispatch` validates
the semantic checkpoints and derives certainty conservatively:

- failure before call submission is `DefinitelyNotExecuted`;
- after submission may have occurred, failure is `OutcomeUnknown` unless an
  authenticated protocol receipt proves pre-execution rejection or start;
- a validated receiver failure after user-code entry is `ExecutionStarted`;
- a decoded and signature-validated reply is completion; and
- absent trustworthy evidence, post-submission failure defaults to unknown.

Thus a transport plugin cannot make retry safe by naming a write failure
"pre-send." Missing-blob resupply remains a bounded protocol continuation
inside one attempt only when a validated response proves no user execution.

The semantic runtime enforces the outer deadline. A deadline provider may map
or shorten it, not extend it; expiry after possible submission is unknown.
Cancellation stops backoff and future attempts, while provider cancellation is
best effort and proves no rollback. Observability cannot alter dispatch and
receives redacted data, never credentials or raw argument values by default.

## Shared `RemoteDispatch` And Public Surfaces

Replace the three dispatcher projections with one runtime-private operation:

```text
RemoteDispatch::attempt<P, Sig>(
    providers: ProviderSet<P>,
    placement: Placement<P>,
    target: TransferTarget<Sig>,
    args: ArgumentPack<Sig>,
    identity: LogicalCallIdentity,
    control: AttemptControl,
    dedup: Option<DedupLease<P, Sig>>,
) -> InternalAttemptOutcome<R, RemoteCause<P::ProviderCause>>
```

`InternalAttemptOutcome` retains completion, failure facts, suspension,
cancellation, and engine fault without constructing a Shape value or formatting
a string. The policy interpreter consumes only its recoverable failure arm.

```text
@remote(on: PlacementPool<P>)
fn f(args...) -> R

@recover(plan)
@remote(on: PlacementPool<P>)
fn f(args...) -> R

remote::call<P, Sig>(
    on: PlacementPool<P>,
    target: Target<Sig>,
    args: ArgumentPack<Sig>,
    recovery: RecoveryPlan<Sig, RemoteAttempt<P, Sig>, _> = single_attempt,
) -> Result<R, RemoteError<P::ProviderCause>>

remote::call_async<P, Sig>(...)
    -> Future<Result<R, RemoteError<P::ProviderCause>>>
```

`@remote` without `@recover` uses one attempt, no backoff, and propagation. Its
source signature remains exactly `Sig`. Explicit calls project the same final
failure to `RemoteError { cause, certainty }`; async adds suspension and local
cancellation, not different delivery semantics.

## Examples

Opaque placement with idempotent failover:

```shape
policy quote_failover for RemoteAttempt<Mesh, Sig> where Sig: QuoteRead {
    budget { attempts: 3, elapsed: 800.ms, per_attempt: 400.ms }
    backoff: exponential(initial: 10.ms, factor: 2, maximum: 40.ms)

    rule unavailable {
        when certainty == DefinitelyNotExecuted
          and cause in [DiscoveryUnavailable, RouteUnavailable, TransportOpen]
        then retry(next_untried, preserve, no_duplicate_possible)
    }
    rule reply_lost {
        when certainty == OutcomeUnknown
        then retry(next_untried, preserve, operation.idempotency)
    }
    terminal: propagate
}

@recover(quote_failover)
@remote(on: placements.market_data)
fn quote(symbol: Symbol) -> Quote { load_quote(symbol) }
```

`placements.market_data` is `PlacementPool<Mesh>`; the policy assumes no
transport or address format. `quote` remains `(Symbol) -> Quote`.

Side-effecting writes require real deduplication:

```shape
policy payment_delivery for RemoteAttempt<Mesh, Sig> where Sig: PaymentWrite {
    budget { attempts: 2, elapsed: 2.seconds, per_attempt: 1500.ms }
    backoff: fixed(25.ms)
    dedup: require operation.dedup_lease(
        key: arguments.payment_id,
        retention_at_least: 5.minutes,
    )
    rule uncertain {
        when certainty in [OutcomeUnknown, ExecutionStarted]
        then retry(same_dedup_domain, preserve, dedup)
    }
    terminal: propagate
}
```

Compilation/planning fails if common dedup scope cannot be established;
runtime planning fails before submission if the negotiated lease is too short.

Typed degradation is allowed only after proven non-execution:

```shape
plan fn relax_consistency(
    args: ArgumentPack<fn(Key, Consistency) -> Record>,
    state: LookupState,
) -> ArgumentPack<fn(Key, Consistency) -> Record> {
    args.replace<1>(Consistency.Eventual)
}

policy resilient_lookup for RemoteAttempt<Mesh, fn(Key, Consistency) -> Record> {
    state: LookupState = { degraded: false }
    budget { attempts: 2, elapsed: 500.ms, per_attempt: 300.ms }
    rule degrade {
        when certainty == DefinitelyNotExecuted
          and cause == StrongReplicaUnavailable
          and state.degraded == false
        then retry(rediscover, relax_consistency, no_duplicate_possible)
             update { ...state, degraded: true }
    }
    terminal: recover with cached_record
}
```

The transform preserves the exact signature; `cached_record` must return
`Record` and runs once. A generic non-remote use is simply
`@recover(local_compile_recovery) fn compile_unit(...) -> Artifact`; its
`LocalCall<Sig>` model has no placement fields.

## Misuse Prevention

The compiler/runtime must enforce:

1. Every plan has finite attempt, elapsed, per-attempt, and backoff bounds.
2. Rules are disjoint; unknown causes fall to the terminal action.
3. Only proven non-execution permits retry without duplicate-safety evidence.
4. Dedup requires admission, fingerprint matching, retention, epoch, replay or
   in-flight joining, and principal isolation. It never means exactly-once
   external effects.
5. Target, pack, transform, fallback, and recovery preserve frozen `Sig` and
   authoritative kinds.
6. Policy code cannot construct certainty, placements, provider brands, auth
   contexts, dedup leases, or logical-call identities.
7. Root cancellation and engine faults bypass recovery.
8. Provider changes cannot broaden placement authority or protocol policy.
9. Missing-dependency negotiation cannot mint extra policy attempts.
10. `@remote` never silently retries uncertainty, falls back locally, or
    changes `R` without an explicit validated plan.

The runtime-owned call envelope covers principal, provider brand,
target/content identity, frozen signature/frame descriptor, argument kinds,
semantic options, logical-call identity, and dedup fingerprint. A codec may
choose bytes but cannot remove or restamp those fields. Certainty receipts bind
the authenticated session, call, target fingerprint, and attempt.

## Hidden Complexity And Ownership

- The annotation compiler owns signature freeze, specialization, rule
  disjointness, bounded-graph validation, typed transforms/fallbacks, and hook
  composition.
- The policy runtime owns call-local state, budgets, backoff/jitter,
  cancellation, reductions, terminal projection, and attempt trace.
- `RemoteDispatch` owns target packaging, provider orchestration, evidence and
  certainty, protocol continuations, reply validation, and one internal
  outcome.
- Providers own mechanics only. Dedup services separately own atomic
  admission, fingerprint refusal, in-flight join, result replay, retention,
  epoch/crash behavior, and principal isolation.
- VM and JIT must execute the same complete plan. Async integration must retain
  state across suspension and avoid orphaned backoff tasks.

Deterministic proof needs injectable clocks/jitter, scripted provider events,
authenticated receipts, dedup epochs, and cancellation races. Otherwise
timeout and unknown-outcome tests prove timing, not semantics.

## Tradeoffs And Order

This interface is bounded, auditable, signature-safe, provider-neutral, and
optimizable. It makes unsafe uncertain retry impossible without visible domain
or dedup evidence. Its cost is a compiler/runtime feature and a restricted plan
language, plus provider branding and real dedup infrastructure. Idempotency
remains a human-owned assertion where external effects are not provable.

That cost is preferable to a callback over `Evaluation<R>`, which would create
exceptions by another name, allow unbounded target calls, expose remote
mechanics in the generic hook, and reduce certainty rules to conventions.

Ratify in this order: generic plan invariants; opaque placement/provider
brands; evidence-to-certainty rules; dedup lease semantics; compiler
specialization/composition; then the shared dispatcher projections for
transparent, explicit, and async remote calls.

## Changed File

`docs/cluster-audits/wave40-failure-hook-design-policy.md`
