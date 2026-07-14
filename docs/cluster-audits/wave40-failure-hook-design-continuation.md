# Wave 40F: Continuation/Capability Failure-Hook Design

Date: 2026-07-10

## Recommendation

Add `on_failure` as a scoped evaluator effect around the next-inner callable
continuation. Give it affine capabilities that must continue evaluation exactly
once, rather than returning an algebraic decision:

```text
type OnFailureHook<Sig, S> = fn(
    args: ArgumentPack<Sig>,
    failed: FailedAttempt<Sig>,
    recovery: RecoveryContext<Sig, S>,
) -> never
```

The handler may inspect failure, start authorized attempts of the same typed
operation, wait, and update call-local state. It must then consume `recovery`
through `accept(success)`, `recover(args, R)`, `propagate(failed)`, or
`replace_failure(failed, rewrite)`. Each operation returns `never`; falling off
the end, returning a value, or consuming the context twice is a compile error.

This preserves the accepted Wave-40 split:

- `ArgumentPack<Sig>` remains immutable and signature-parameterized.
- `before` still returns typed `HookDecision::Proceed` or `Return`; `after`
  remains success-only.
- `@remote fn f(...) -> R` remains `(...) -> R`.
- `Evaluation<R>` remains an evaluator/host type, never a general Shape type.
- A Shape `Result::Err` is still a completed value; `on_failure` sees only a
  non-returning `RuntimeFailure`, not domain `Err` values.
- cancellation and engine faults are not catchable by `on_failure`.

There is no compatibility constraint. The current string-addressed remote
surface should be replaced, not wrapped in another string convention.

## Capability Kernel

For `Sig = (P0[m0], ..., Pn[mn]) -> R`, the semantic interface is:

```text
scoped capability FailedAttempt<Sig> {
    fn args() -> &ArgumentPack<Sig>
    fn failure() -> FailureView
    fn certainty() -> ExecutionCertainty
    fn origin() -> FailureOriginView
    fn attempt_number() -> int
    fn definitely_not_executed() -> Option<NotExecutedProof<Sig>>
}

scoped capability AttemptSuccess<Sig> {
    fn args() -> &ArgumentPack<Sig>
    fn result() -> &ReturnOf<Sig>
    fn observation() -> AttemptObservation
}

scoped enum AttemptOutcome<Sig> { Succeeded(AttemptSuccess<Sig>), Failed(FailedAttempt<Sig>) }
affine capability Attempt<Sig> { async fn outcome(self) -> AttemptOutcome<Sig> }
opaque capability AttemptRoute<Sig>
opaque capability RetryPermit<Sig>

affine capability RecoveryContext<Sig, S> {
    fn default_route(&self, failed: &FailedAttempt<Sig>) -> AttemptRoute<Sig>
    fn authorize(&self, failed: &FailedAttempt<Sig>, route: &AttemptRoute<Sig>,
        args: &ArgumentPack<Sig>, evidence: ReplayEvidence<Sig>) -> Result<RetryPermit<Sig>, RetryDenied>
    fn attempt(&mut self, failed: &FailedAttempt<Sig>, route: AttemptRoute<Sig>,
        args: ArgumentPack<Sig>, permit: RetryPermit<Sig>) -> Attempt<Sig>
    async fn pause(&mut self, delay: RecoveryDelay)
    fn state(&self) -> &S
    fn set_state(&mut self, state: S)
    fn accept(self, success: AttemptSuccess<Sig>) -> never
    fn recover(self, args: ArgumentPack<Sig>, result: ReturnOf<Sig>) -> never
    fn propagate(self, failed: FailedAttempt<Sig>) -> never
    fn replace_failure(self, failed: FailedAttempt<Sig>, rewrite: FailureRewrite) -> never
}
```

These are compiler-scoped types, not ordinary heap values. They have no public
schema, serialization, equality, reflection, or collection representation.
They cannot be stored in module state, returned, captured by a closure, sent to
a spawned task, or retained after the handler. A scoped helper may borrow them,
but the compiler proves that no capability escapes.

`AttemptOutcome<Sig>` is deliberately narrower than `Evaluation<R>`:

- evaluator suspension is absorbed by `await Attempt.outcome()`;
- root cancellation aborts the recovery scope as `Cancelled`;
- an engine fault bypasses the handler as `Faulted`;
- only successful `R` and a recoverable `RuntimeFailure` become scoped attempt
  outcomes.

The handler can therefore inspect typed attempt outcomes without making
evaluator control states source-level values or introducing general try/catch.

### Failure views and replacement

`FailureView` exposes structured, read-only data: stable kind/code, diagnostic,
origin, typed details, and execution certainty. It does not expose mutable VM
errors or an exception object.

`FailureRewrite` may replace the public code/message/details and add a cause
frame. It cannot set certainty, erase the original origin, construct a
`Cancelled`/`EngineFault`, or promote a failure to a completion. The core joins
certainty monotonically, so an `OutcomeUnknown` failure can never be rewritten
as `DefinitelyNotExecuted`.

## Invocation and Composition Semantics

For an annotation layer `Ai`, the evaluator behaves as follows:

1. Run `Ai.before`.
2. On `Proceed`, invoke the next-inner continuation once with its pack.
3. On success, run `Ai.after` and return its `R`.
4. On a next-inner `RuntimeFailure`, invoke `Ai.on_failure` with the effective
   pack, a `FailedAttempt<Sig>`, and `Ai`'s current state.
5. `accept` or `recover` resumes at step 3, so the same layer's after hook and
   already-entered outer after hooks observe the recovered `R`.
6. `propagate` or `replace_failure` skips `Ai.after`; the nearest entered outer
   `on_failure` receives the resulting failure.

The same layer never catches a failure raised by its own `before`, `after`, or
`on_failure`. Outer layers may catch that failure. An attempt started from
`on_failure` invokes the next-inner hook chain normally, including inner
failure hooks, but it does not recursively re-enter the current handler. An
unrecovered failure returns as `AttemptOutcome::Failed` for explicit handling.

A before-hook `Return` is already a successful result and does not invoke
`on_failure`. Pending after hooks do not run during failure propagation.

`S` remains Wave-40C's call-local, same-layer state. It arrives from the
before decision, survives pauses and attempts, and reaches the same layer's
after hook only after `accept`/`recover`. It is neither persistent state nor
shared across annotation layers.

### What an attempt replays

`FailedAttempt<Sig>` carries a hidden replay template: the exact typed logical
operation, next-inner continuation, signature witness, original attempt facts,
and provider execution seam. `RecoveryContext.attempt` replays that template;
it does not naively call an annotation wrapper that could apply placement
twice.

This is load-bearing for composition such as:

```text
@recover_with(alternates, replay_policy, backoff)
@remote(primary)
fn score(job: Job) -> Score
```

The outer recovery layer sees the failed remote operation. An alternate route
replaces that operation's placement at its execution seam while retaining the
same next-inner continuation. It does not execute `@remote(primary)` again on
the alternate destination.

## Retry Authority

Every second or later execution attempt requires a one-use
`RetryPermit<Sig>`. A permit is bound to:

- the recovery episode and prior failed attempt;
- the exact `Sig`, target identity, and content hash;
- the proposed argument fingerprint and pass modes;
- the destination/placement effect domain;
- the remaining attempt/deadline budget; and
- the evidence that permits possible duplicate effects.

`ReplayEvidence<Sig>` is a sealed capability family, never a boolean:

```text
NotExecutedProof<Sig>          // minted only by the evaluator's certainty model
PureComputationProof<Sig>      // compiler-derived from a future effect system
IdempotencyProof<Sig, Scope>   // trusted domain declaration with a key/scope rule
DeduplicationLease<Sig, Scope> // provider attestation, key, epoch, retention window
DuplicateEffectsAuthority<Sig> // explicit privileged host policy, never implicit
ArgumentChangeProof<Sig>       // domain proof covering the proposed transformation
```

The authorization rules are:

| Prior certainty | Same args, same route | Same args, alternate route | Changed args |
|---|---|---|---|
| `DefinitelyNotExecuted` | `NotExecutedProof` is sufficient | sufficient | sufficient |
| `OutcomeUnknown` | pure/idempotent/deduplicated/privileged evidence | evidence must span both placement effect domains | requires `ArgumentChangeProof`; an exact-request dedup lease is insufficient |
| `ExecutionStarted` | same as outcome unknown | same as outcome unknown | same strict argument-change proof |

Local callee failure is normally `ExecutionStarted`; certainty is not a
remote-only concept. Permission/admission refusal before user code can be
`DefinitelyNotExecuted`. Backoff never makes an ambiguous attempt safe.

An idempotency key is not an idempotency proof. A provider lease must bind the
key to principal, target, canonical argument fingerprint, effect domain,
receiver epoch, and retention window. Correlation/cancellation IDs cannot mint
such a lease. Cross-provider or cross-placement failover is denied unless the
evidence explicitly spans both effect domains.

Changed arguments use the ordinary immutable pack operation:

```text
let smaller = args.replace<0>(args[0].take(500))
```

This is automatically retry-authorizable only when the prior attempt is proven
not to have executed. After an ambiguous or started attempt, a same-request
idempotency or deduplication proof does not justify a different operation.

Recovery has a mandatory total deadline and attempt budget. `pause` and every
provider attempt consume that budget. Neither user code nor a provider can
extend the parent deadline. Cancellation aborts the scope promptly; a provider
may request remote cancellation, but that request never upgrades execution
certainty by itself.

## Opaque Placement and Provider Model

The source-facing remoting interface is capability-based:

```text
trait RemoteExecutionProvider

opaque RemotePlacement<P: RemoteExecutionProvider>
opaque RemotePlacementSet<P: RemoteExecutionProvider>

@remote<P>(placement: RemotePlacement<P>)
fn f(...) -> R

remote::call<P, Sig>(
    placement: RemotePlacement<P>,
    target: HookTarget<Sig>,
    args: ArgumentPack<Sig>,
) -> Result<ReturnOf<Sig>, RemoteError>
```

`RemotePlacement<P>` is an unforgeable authority to ask provider `P` to place
an attempt. It may denote one destination, a discovered pool, a service,
queue, actor, process class, region policy, or another provider-defined routing
intent. It has no public host, port, URI, socket, token, certificate, or codec
field. Printing it yields only a provider-redacted label. It is nonserializable
unless its provider supplies an explicit safe delegation format.

Providers mint placements through typed, provider-specific discovery modules.
For example, this source may select a workload class and failure-domain policy,
but it never constructs an address:

```text
let primary = fleet.place(Workload::Scoring, Preference::NearestHealthy)
let alternates = fleet.alternates(primary, Separation::FailureDomain)

@recover_with(alternates, ScoreByJobId, exponential(100ms, 2s), attempts(3))
@remote(primary)
fn score(job: ScoreJob) -> Score
```

A remote placement can be adapted to the generic `AttemptRoute<Sig>` only by
its registered provider. The generic failure-hook interface knows only
`AttemptRoute`; it has no remote-specific method or field. Local executors,
sandboxes, worker threads, GPUs, and future placement mechanisms can implement
the same route capability.

### Provider interfaces

The deep provider seam is one semantic operation:

```text
trait RemoteExecutionProvider {
    fn begin<Sig>(&self, placement: &Self::Placement,
        call: TypedCallIntent<Sig>, controls: AttemptControls) -> ProviderAttempt<Sig>
}
```

`TypedCallIntent<Sig>` carries the typed continuation/function blobs,
`ArgumentPack<Sig>`, expected `R`, authoritative per-position kinds,
permissions, logical-call identity, optional idempotency material, and tracing
context. It contains no provider address. `AttemptControls` carries a core
deadline, cancellation capability, payload/resource ceilings, and redaction
policy.

A provider may implement that operation monolithically or compose these
replaceable internal facets:

```text
trait DiscoveryProvider<Q, C> { fn discover(Q, DiscoveryScope) -> CandidateSet<C> }
trait RoutingProvider<C, P> { fn route(CandidateSet<C>, RoutingIntent) -> P }
trait AddressEncoder<P, E> { fn endpoint(P) -> E }
trait TransportProvider<E, Ch> { fn open(E, Deadline, CancellationCapability) -> Ch }
trait AuthenticationProvider<Ch, S> { fn authenticate(Ch, PrincipalCapability) -> S }
trait CallCodec<Sig, Session, Frame, Reply> {
    fn encode(Session, TypedCallIntent<Sig>) -> Frame
    fn decode(Session, Reply, SignatureWitness<Sig>) -> TypedProviderReply<Sig>
}
trait ProtocolNegotiator<Ch, S> { fn negotiate(Ch, ProtocolRequirements) -> S }
trait DeadlineController<S> { fn enforce(S, Deadline) -> DeadlineEvidence }
trait CancellationProvider<H> { fn request(H, CancellationCapability) -> CancellationObservation }
trait AttemptObserver { fn record(RedactedAttemptEvent) }
```

The concrete `Endpoint`, channel, credentials, frame format, and negotiated
protocol are provider-private types. A provider may use TCP, QUIC, a broker,
shared memory, an actor runtime, a service mesh, or something else without
changing `@remote`, `on_failure`, or the retry rules.

The current implementation is much narrower: source APIs and
`RemoteDispatcher` take address strings
(`crates/shape-runtime/stdlib-src/core/remote.shape:1-104,165-190`,
`crates/shape-runtime/src/module_exports.rs:120-162`), while
`WireTransportProvider` only chooses a built-in transport kind and is installed
through global state (`crates/shape-wire/src/transport/factory.rs:12-105`,
`crates/shape-vm/src/executor/builtins/transport_provider.rs:9-35`). This design
replaces all three seams.

### Safety above providers

Provider customization cannot weaken language invariants:

- Discovery/routing returns opaque candidates and placements; it cannot change
  `Sig`, arguments, target identity, or permissions.
- Address encoding and auth are hidden. Credentials are sealed host
  capabilities and never become hook values or diagnostic text.
- Codecs receive typed slots and signature witnesses. They cannot infer kinds
  from bits, flatten a nested array into arity, or accept a reply of the wrong
  `R`.
- Protocol negotiation may add guarantees. It cannot negotiate away signature,
  permission, resource, certainty, or return-kind validation.
- The core owns the attempt phase ledger. Providers report facts such as
  `not submitted`, `submitted`, `admitted`, `execution started`, and `terminal
  reply observed`; the core derives `ExecutionCertainty`.
- Cancellation observations remain separate from execution certainty.
- A provider can mint `DeduplicationLease` only from a negotiated guarantee
  with explicit scope, epoch, retention, and canonical request fingerprint.
- Deadline adapters may shorten, never extend, the recovery deadline.
- Observability receives redacted immutable events and cannot affect routing or
  outcome semantics. Correlation IDs are never retry authority.

## Realistic Handler Examples

### Recover a value or replace the failure

This hook is generic and knows nothing about remoting:

```text
annotation cached_fallback(cache: ReadCache<Request, Response>) {
    on_failure(args, failed, recovery) -> never {
        match cache.get(args.get<0>().request_id) {
            Some(value) => recovery.recover(args, value),
            None if failed.failure().kind == FailureKind::Permission =>
                recovery.replace_failure(failed, FailureRewrite {
                    code: "cache.permission_refused",
                    message: "fallback cache access was refused",
                    details: {},
                }),
            None => recovery.propagate(failed),
        }
    }
}
```

The replacement retains the original cause and certainty. A cached domain
`Result::Err` would simply be the recovered `R` if `R` itself is a Result.

### Retry with changed arguments only before execution

```text
annotation shrink_rejected_batch(limit: int) {
    on_failure(args, failed, recovery) -> never {
        let proof = match failed.definitely_not_executed() {
            Some(proof) => proof,
            None => recovery.propagate(failed),
        }
        let smaller = args.replace<0>(args.get<0>().take(limit))
        let route = recovery.default_route(failed)
        let permit = recovery.authorize(failed, route, smaller, proof)
            else recovery.propagate(failed)
        match await recovery.attempt(failed, route, smaller, permit).outcome() {
            Succeeded(done) => recovery.accept(done),
            Failed(next) => recovery.propagate(next),
        }
    }
}
```

An outcome-unknown rejection cannot enter this path. The compiler also rejects
reusing `proof`, `permit`, or the consumed attempt.

### Placement failover with backoff and state

```text
annotation recover_with<Sig, P>(alternates: RemotePlacementSet<P>,
    replay: IdempotencyProof<Sig, JobIdScope>, backoff: Backoff,
    budget: AttemptBudget) {
    on_failure(args, first, recovery) -> never {
        var failed = first
        for placement in alternates.within(budget) {
            let route = placement.route_for(failed)
            let evidence = failed.definitely_not_executed().unwrap_or(replay)
            let permit = recovery.authorize(failed, route, args, evidence)
                else recovery.propagate(failed)
            recovery.set_state(recovery.state().next_attempt(placement.label()))
            await recovery.pause(backoff.delay(recovery.state().attempt))
            match await recovery.attempt(failed, route, args, permit).outcome() {
                Succeeded(done) => recovery.accept(done),
                Failed(next) => failed = next,
            }
        }
        recovery.propagate(failed)
    }
}
```

`placement.label()` is provider-redacted presentation data, not an address.
The idempotency proof is bound to the exact arguments and a scope spanning the
placement set. A provider-local dedup lease would fail authorization for a
placement outside its deduplication domain.

## Misuse Prevention

The compiler/runtime must make these invalid by construction:

- exposing evaluator/failure/attempt capabilities as ordinary Shape values;
- catching domain `Result::Err`, cancellation, suspension, or engine faults;
- returning from `on_failure` without one terminal continuation operation;
- invoking the continuation after failure without a retry permit;
- using a boolean `idempotent: true`, a correlation ID, or a cancellation ID as
  replay evidence;
- changing arguments after uncertain/started work under an exact-request proof;
- rerouting outside the evidence's effect/deduplication domain;
- reusing any affine attempt, permit, success, failure, or recovery capability;
- replacing a failure with weaker certainty or hiding its causal origin;
- exposing/parsing destination strings through the generic hook interface;
- letting provider policy alter `Sig`, `R`, argument kinds, or retry authority;
- starting unbounded attempts or sleeping beyond the recovery deadline; and
- silently retrying or running the body locally from plain `@remote`.

Parallel hedging is intentionally absent. Cancelling losing attempts cannot
prove they did not execute, so a future hedge capability would require replay
evidence for every concurrently started attempt and would return a set of
post-cancellation certainty observations.

## Hidden Complexity

The small source interface hides substantial implementation work:

1. Delimited continuations must capture the next-inner chain, frame descriptor,
   and attempt template without exposing VM frames.
2. The checker needs scoped/affine capability and `never`-path analysis, with
   useful escape/double-consumption diagnostics.
3. Pack ownership, references, defaults, and kind tracks must survive attempts
   without implicit copies or kind reconstruction.
4. Attempt/backoff suspension must retain recovery state; cancellation must
   tear down handler and provider work exactly once.
5. VM/JIT failures need one structured interception path, with no interpreter
   rerun after JIT side effects.
6. Certainty and permits combine phase facts, cancellation races, fingerprints,
   effect domains, dedup epochs, retention, and nested causes.
7. Rerouting must replace the failed route, while transfer/auth/codec/protocol
   details stay provider-private and observability stays redacted.

## Tradeoffs

This design is powerful without turning every function call into `Result` or
making evaluator outcomes general values. Recovery code can express fallback,
failure context, sequential retry, changed arguments, placement failover,
backoff, and state through one deep interface. The affine terminal operations
make control flow explicit, while permits put certainty/idempotency checks at
the actual attempt seam rather than in documentation.

The cost is a materially more advanced language/runtime feature than a small
`FailureDecision` enum. Scoped affine capabilities, delimited continuations,
async recovery suspension, and provider-neutral rerouting all require strong
compiler and debugger support. Handler code is verbose, and policy authors must
understand attempt identity and effect domains. Domain idempotency remains a
trusted assertion unless a future effect system proves purity; the interface
can force the assertion to be explicit but cannot prove arbitrary external
effects.

The provider model also moves substantial responsibility to embedders. That is
intentional: address syntax, service discovery, credentials, wire formats, and
transport policy are deployment concerns. Keeping them out of the language
surface prevents today's `host:port` choice from becoming the permanent shape
of recovery and distributed execution.

## Proof Boundary

Compiler-level proofs should cover signature-preserving recovery, `R` typing,
pack replacement, all capability escape/double-use cases, mandatory terminal
paths, permit binding, changed-argument refusal, cross-domain refusal, and
provider inability to mint or strengthen certainty.

Runtime proofs should pin nested hook order, same-layer after on recovery,
outer failure propagation, no same-layer recursive interception, state across
pause/attempt, cancellation bypass, engine-fault bypass, and VM/JIT parity.

Provider fixtures should cover opaque discovery/address types, alternate
transport/auth/codec, negotiation, deadlines, cancellation, observability,
reply validation, every certainty class, and scoped dedup leases. Use both a
real-socket and an in-memory provider so no TCP/address assumption leaks into
semantics.

No cargo, test, build, extraction, or book command was run for this design.
