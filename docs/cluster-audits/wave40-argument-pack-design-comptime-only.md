# Wave 40S: Comptime-Only Argument-Pack Specialization

Date: 2026-07-10

Scope: clean-break interface design, not implementation authorization; the current array/object/null protocol has no compatibility promise.

## Decision

Do not expose named, indexed, iterable, or first-class `ArgumentPack` access to
runtime Shape code. Keep one immutable, signature-bound argument carrier inside
the compiler and evaluator, but erase it from the source interface after the
callable signature is frozen.

Callable annotation definitions remain ordinary Shape stdlib code. The compiler
specializes each definition for one frozen callable signature, reflects over
that signature at comptime, and generates a wrapper whose parameters are the
target's ordinary typed parameters. Whole-call forwarding and changed-argument
calls lower to direct calls such as `next(p0, p1, p2)`. They never lower through
a homogeneous array, a runtime name lookup, or a source-visible heterogeneous
container.

This is deliberately narrower than the first-class `ArgumentPack<Sig>` proposal
in `wave40-annotation-hook-type-model.md` and the runtime `get`/`replace` surface
in `wave40-failure-hook-design-algebraic.md`. It preserves their important
invariants - frozen signature, immutable effective arguments, exact next-inner
continuation, authoritative kinds, and typed replacement - while moving all
argument projection into specialization.

## What Runtime Named Access Would Be For

Runtime lookup such as `args.get("request")` is useful only when code must remain
ignorant of the signature until the call is already executing. Examples are a
decorator loaded after compilation, a dynamic RPC gateway, middleware selected
by a runtime parameter name, or a program that stores and replays arbitrary
calls as application values.

Callable annotations do not have that problem. Their Shape definitions and
comptime hooks are available before signature directives finish, ordinary
checking reruns, `Sig` freezes, and the exact runtime wrapper specializes.

Runtime name lookup merely defers information the compiler already has. It adds
missing-name and wrong-type failure paths, a reflection and wire
surface, ownership questions for projected values, and VM/JIT work without
adding expressive power. A generic logger can generate one typed logging call
per parameter. A generic retry can preserve the current attempt without reading
arguments. A changed-argument policy resolves a typed lens at comptime.

Truly late-bound invocation should be a separate `DynamicCall` facility with an
explicit dynamic schema and security model. It should not make every annotation
call dynamic.

## Current Evidence

The compiler already builds a comptime target descriptor
(`crates/shape-vm/src/compiler/comptime_target.rs:219-272,439-487`), follows the
accepted pre-then-post order (`docs/vision/rfc-comptime-transform-api-v1.md:59-75`),
and specializes composed handlers per application site
  (`crates/shape-vm/src/compiler/functions_annotations.rs:2338-2451,
  2637-2791`). This is the seam to deepen.

The remaining representation is dynamic:

- Despite specialization, the compiler insists that every target parameter have
  one common element type and storage carrier, then emits a typed array
  (`functions_annotations.rs:345-485`). Mixed signatures are rejected.
- The wrapper interprets a returned array as replacement arguments and an
  `Any`-field object as `{args, result, state}`; null doubles as a control
  sentinel (`functions_annotations.rs:2917-3151`). It later indexes the array to
  rebuild the direct call (`:3153-3169`).
- `ctx.target` already has the target's specialized callable type
  (`functions_annotations.rs:2513-2557`), so runtime erasure of the arguments is
  not required by generic annotations.
- General comptime generation still permits unhygienic source text
  (`docs/design/comptime-excellence.md:334-362`). A callable-wrapper generator
  cannot reuse that surface.

The current `@remote` makes the mismatch visible: it accepts `args: Array<_>` and
passes that array to `__call_raising`
(`crates/shape-runtime/stdlib-src/core/remote.shape:72-108,165-190`). Its
address-string surface is also not retained by this design.

## Semantic Split

For a frozen signature

```text
Sig = (P0[m0], P1[m1], ..., Pn[mn]) -> R ! Effects
```

use two different layers.

### Comptime-only types

```text
comptime sealed FrozenCallable<Sig>
comptime sealed ParamLens<Sig, I, T, Mode>
comptime sealed HygienicSymbol<T>
comptime sealed CheckedExpr<T, Effects>
comptime sealed CheckedItem<Sig, Effects>
comptime sealed RewritePlan<Sig>
comptime sealed CallableHookGenerator<Sig>
```

`FrozenCallable` exposes ordered parameter descriptors, stable parameter
identity, type, pass mode, authoritative `NativeKind`, return type, effects, and
the exact next-inner continuation identity. `ParamLens` is an unforgeable,
comptime-only typed lens created only by selecting a member of that descriptor.
It contains no runtime value. `HygienicSymbol<T>` names a compiler-owned binding
by identity, never by generated text.

`CheckedExpr<T, Effects>` and `CheckedItem<Sig, Effects>` are typed compiler
fragments, not syntax trees encoded as user data. They can refer only to issued
hygienic symbols, handler inputs, annotation values, and explicit dependencies.
`RewritePlan<Sig>` is a finite set of checked functional replacements. It cannot
alter arity, reorder positions, change pass modes, or produce another signature.

The exact source spelling is open, but the semantic generator interface is:

```text
target.param<I>() -> ParamLens<Sig, I, Pi, mi>
target.require_param<T>(selector) -> ParamLens<Sig, I, T, mi>

gen.forward_next() -> CheckedExpr<R, EffectsOf<Sig>>
gen.forward_callable(to: Callable<Sig>) -> CheckedExpr<R, EffectsOf<Sig>>
gen.replace(param: ParamLens<Sig, I, T, mi>, value: CheckedExpr<T, E>)
    -> RewritePlan<Sig>
gen.compose(rewrites...) -> RewritePlan<Sig>
gen.forward_next_with(plan: RewritePlan<Sig>)
    -> CheckedExpr<R, EffectsOf<Sig>>
gen.install_before(item: CheckedItem<BeforeSig<Sig>, E>)
gen.install_after(item: CheckedItem<AfterSig<Sig>, E>)
gen.install_on_failure(item: CheckedItem<FailureSig<Sig>, E>)
```

`forward_*` is a generation operation. For three parameters it emits a normal
typed call with three operands. It does not construct a splat value. A nested
`Array<T>` remains one operand.

### Non-negotiable generation boundary

This interface is stricter than Zig-style text generation. There is no function
from `string`, bytes, JSON, map/object data, `Any`, or a user-constructed AST to
`CheckedExpr` or `CheckedItem`. In particular:

- no annotation emits source text and no generated wrapper is parsed again;
- no parser round-trip, JSON AST payload, dynamic object schema, or untyped
  fragment participates in specialization;
- descriptors, lenses, symbols, expressions, and items are compiler-issued
  sealed values with typed constructors only;
- names may resolve a lens at comptime, but emitted code refers to stable
  declaration identity and hygienic symbols, never reconstructs a runtime value
  by name; and
- installing a checked fragment is not a verifier bypass. The completed wrapper
  must re-enter normal type, effect, ownership, borrow, and native-kind checking
  before it can become bytecode or MIR.

The existing source-string comptime path is ineligible for callable-hook
generation even if its output would parse. A future general typed item builder
may share the sealed fragment representation; it may not add an
`unsafe_from_text` or `Any` escape hatch.

### Runtime-internal types

The evaluator may use a conceptual carrier such as:

```text
InternalArgumentState<Sig> {
    signature_id: FrozenSignatureId,
    slots: one typed storage location per Pi[mi],
    effective_attempt: AttemptId,
}
```

This type is not nameable, constructible, projectable, serializable, or
returnable by Shape code. It is an evaluator ledger for suspension, failure,
retry, provider dispatch, snapshot bookkeeping, and the same-layer `after`
view. An implementation may optimize it away when none of those operations
requires retention.

Each slot retains its type, mode, and kind. There is no `Array<_>` view and no
kind inference from payload bits. Immutability means replacement creates a new
effective attempt description; it does not make an affine value copyable.

## Specialization Pipeline

1. Run all annotation `comptime pre` handlers against the provisional target.
2. Infer and apply signature-affecting directives, then run `comptime post` in
   the accepted order.
3. Re-enter ordinary declaration/body, effect, borrow, and ownership checking.
4. Freeze `Sig`, including defaults, parameter modes, kinds, return type, and
   effects. Generic callables specialize after concrete type substitution; no
   hook sees `unknown` as a wildcard slot.
5. Resolve every comptime parameter selector and callable dependency. An absent
   or ambiguous name, wrong type, wrong mode, or incompatible fallback is a
   compile error at the annotation application.
6. Run the Shape annotation specializer with `FrozenCallable<Sig>` and a
   structured `CallableHookGenerator<Sig>`.
7. Generate ordinary typed wrapper parameters and direct continuation calls,
   then run the generated code through the same type, effect, ownership, and
   native-kind verifier as handwritten code.
8. Lower the complete hook plan to one execution representation used by VM and
   JIT. Never attach the raw body's MIR to an annotated wrapper.

Defaults are materialized exactly once before the outermost layer. Comptime-only
annotation parameters do not become target runtime positions. Annotation
runtime arguments remain normal typed captures or wrapper constants; they are
not mixed into the target argument state.

## Forwarding Without A Pack

For target parameters `(batch: Batch, limit: int, policy: Policy)`, a generic
`gen.forward_next()` emits the equivalent of:

```shape
__next_inner(batch, limit, policy)
```

The annotation author does not spell or maintain that list. The generator owns
ordering, default completion, reference/out wrapping, dependency recording, and
diagnostic source maps. The emitted wrapper still exposes only the target's
ordinary public signature.

`HookTarget<Sig>` continues to mean the exact next-inner continuation. The
generator can emit a call to it, but runtime policy code cannot substitute an
arbitrary target under that name or bypass an inner annotation. A separately
declared fallback callable is checked as `Callable<Sig>` and is recorded as a
different explicit attempt target.

## Failure And Retry Without Runtime Pack Access

Whole-argument preservation is a property of an attempt, not a reason to expose
the arguments. The source-visible failure algebra can be narrowed to:

```shape
enum FailureDecision<Sig, S> {
    Propagate { failure: RuntimeFailure }
    Recover { result: ReturnOf<Sig>, state: S }
    Retry { attempt: InvocationAttempt<Sig>, state: S }
}
```

The earlier `Recover.args` field is unnecessary. The evaluator already knows
which effective attempt completed or was recovered. The same-layer `after`
hook receives any argument values it statically requested as ordinary typed
parameters; otherwise it receives only `R` and `S`.

Sealed context operations preserve the hidden state:

```text
ctx.proceed_current(state) -> BeforeDecision<Sig, S>
ctx.retry_current(failure, at, after, permit?) -> InvocationAttempt<Sig>
ctx.retry_rewritten(failure, plan_id, at, after, permit?)
    -> InvocationAttempt<Sig>
ctx.try_alternate_current(failure, to, at, permit?)
    -> InvocationAttempt<Sig>
```

`plan_id` is emitted by specialization and cannot be forged or selected from a
runtime string. Runtime code may choose among a finite set of generated plans,
for example `refresh_token` versus `renew_session`, but every branch was checked
against the same `Sig`.

Execution certainty, idempotency, deduplication, attempt budgets, and exact
continuation rules remain above provider interfaces. `OutcomeUnknown` or
`ExecutionStarted` still requires an explicit `UnknownRetryPermit<Sig>`.
Changing a destination does not make an unsafe retry safe.

Affine parameters require an additional proof. The compiler must reject a retry
or fallback when an owned argument cannot be restored from the failed attempt,
replayed by a declared `Replayable` operation, or replaced before reuse. The
internal carrier cannot silently clone resources, references, mutable cells, or
provider handles. A reference/out parameter must also remain valid for the full
logical call and every permitted suspension.

## Illustrative Shape Definitions

Spelling is illustrative. Each `comptime specialize` block builds sealed checked
fragments; it is not quotation, text generation, or a parser input.

### Transparent `@remote`

```shape
pub annotation remote<Sig>(at: Placement<Sig>) {
    comptime specialize(target: FrozenCallable<Sig>, gen) {
        gen.replace_invocation(gen.remote_call_raising(
            at, target.next_inner, target.signature_id
        ))
    }
}

@remote(analytics_placement)
fn summarize(batch: Batch, limit: int) -> Summary
```

The generated body is an ordinary direct call equivalent to
`__remote_specialized(at, next, sig, batch, limit)`. `Placement<Sig>` is an
opaque provider-issued capability. Discovery/routing, destination encoding,
transport, auth, codec, negotiation, deadlines, cancellation, and observability
remain provider interfaces. No host, port, URI, address bytes, or argument array
appears in `@remote`.

### Generic `@retry` and `@fallback`

```shape
pub annotation retry<Sig>(max: int, backoff: Backoff, unknown = None) {
    before(ctx) { ctx.proceed_current(RetryState { used: 0 }) }
    on_failure(failure, state, ctx) {
        if state.used >= max {
            return FailureDecision::Propagate { failure: failure }
        }
        FailureDecision::Retry {
            attempt: ctx.retry_current(
                failure, at: ctx.current.placement,
                after: backoff.for_attempt(state.used + 1),
                permit: require_retry_permission(failure, unknown),
            ),
            state: RetryState { used: state.used + 1 },
        }
    }
}

pub annotation fallback<Sig>(to: Callable<Sig>) {
    on_failure(failure, state, ctx) {
        if failure.certainty.is_definitely_not_executed() {
            return FailureDecision::Retry {
                attempt: ctx.try_alternate_current(failure, to), state: state
            }
        }
        FailureDecision::Propagate { failure: failure }
    }
}
```

Neither definition reads arguments. Specialization checks replayability and the
fallback's complete signature/effects. Unknown-outcome fallback still needs an
explicit equivalence/idempotency or dedup permit.

### Changed arguments

```shape
pub annotation refresh_retry<T, Sig>(
    comptime parameter: ParamSelector<T>,
    refresh: fn(T, FailureView) -> T,
) {
    comptime specialize(target: FrozenCallable<Sig>, gen) {
        let lens = target.require_param<T>(parameter)
        let rewrite = gen.replace_with_failure(lens, refresh)
        gen.install_on_failure(retry_policy(max: 1, rewrite: rewrite))
    }
}

@refresh_retry(param("request"), refresh_request)
fn fetch(request: Request, policy: FetchPolicy) -> Response
```

The comptime name resolves once to a typed lens and stable declaration identity.
The checked fragment is equivalent to
`next(refresh_request(request, failure.view()), policy)`. There is no runtime
lookup. Multiple replacements require unambiguous ownership and evaluation
order; a runtime branch may choose only among already checked rewrite plans.

### Provider-neutral `@remote_pool`

```shape
@remote_pool(analytics_workers, unknown: idempotent(pure_batch_transform))
fn compute(batch: Batch) -> Summary
```

The provider-issued `PlacementPool<Sig>` contains no addresses. The policy
changes placement and calls `retry_current`. Argument-aware routing requires a
comptime lens plus a typed routing function, which becomes an ordinary typed
parameter use in the specialized wrapper.

## ABI And Artifact Metadata

No source `ArgumentPack` type or serialized layout belongs in the public Shape
ABI. The artifact contract needs these hash-covered facts instead:

```text
HookSpecializationDescriptor {
    frozen_signature_id,
    hook_semantics_revision,
    ordered_layer_ids,
    next_inner_function_hash,
    rewrite_plan_hashes,
    required_execution_capabilities,
}
```

The generated wrapper's function artifact carries its exact frame descriptor,
parameter modes/kinds, return kind, bytecode/MIR, constants, and static
dependencies. The current `FunctionBlob` hash already covers most of those
concrete fields and dependencies
(`crates/shape-vm/src/bytecode/content_addressed.rs:33-192`), but the clean
artifact contract also needs the execution ABI binding described in
`wave40-execution-abi-binding.md`.

`hook_semantics_revision` covers internal attempt, failure, continuation, and
argument-ledger semantics that generated instructions alone do not explain.
Each rewrite hash covers selected stable parameter identities, replacement
callable dependencies, evaluation order, and ownership mode. Annotation
definition hash, comptime arguments, build configuration, and generator version
are specialization-cache inputs. Transferred execution needs the generated
artifact and dependencies, not the annotation source or a runtime reflection
table.

Remote codecs receive `FrozenSignatureId`, the authoritative frame descriptor,
and ordered slots. Snapshots that suspend inside a hook record the verified hook
plan ID, effective attempt ID, and typed frame state. A resume with another hook
revision, rewrite plan, signature, or execution ABI fails before restoring
values. Cache keys are namespaced by artifact format and execution ABI.

Parameter display names remain diagnostics. A name-based comptime selector is
resolved to a stable parameter identity and forces re-specialization after a
rename. Libraries that need rename-stable selection should use an explicit
parameter marker; runtime fallback from a missing name is forbidden.

## Generated-Code Hygiene

The callable specializer is a compiler-owned structured module with these
additional invariants:

- Generated parameter and local identities are unforgeable compiler IDs;
  annotation source cannot capture a caller local accidentally.
- The next-inner token is supplied by the compiler and can only emit a call or
  sealed attempt against that continuation.
- Every fragment is typed before installation, then the completed function
  re-enters ordinary type, effect, ownership, borrow, and kind verification.
- Expansion is deterministic. Layer order, parameter order, rewrite evaluation
  order, and synthesized IDs do not depend on hash-map iteration.
- Diagnostics anchor the application and definition and can render the checked
  fragment and final ordinary wrapper.
- VM and JIT lower from the same complete hook plan. Neither execution mode may
  bypass failure hooks, rewrites, or provider dispatch.

Annotation authors may generate repetitive typed behavior, but they cannot emit
unchecked opcodes, raw slot casts, arbitrary frame metadata, or a claimed
`FrozenSignatureId`.

## Misuse Prevention

The compiler rejects:

- heterogeneous runtime arrays or any implicit `Array<T>` projection;
- runtime name/index lookup, pack iteration, pack storage, or cross-signature
  conversion;
- missing/ambiguous selectors, wrong replacement type or mode, changed arity,
  or parameter reordering;
- fallback or placement capabilities for another `Sig`;
- retry of non-replayable affine/reference/resource state;
- unknown-outcome retry without a valid idempotency/dedup permit;
- a provider attempting to weaken signature, certainty, budget, or permission
  checks;
- generated code whose effects exceed the target or annotation contract; and
- transfer or snapshot restore under mismatched hook-plan or execution-ABI IDs.

## Limitations And Tradeoffs

Annotations applied after compilation, runtime name enumeration, arbitrary call
recording, and plugins that discover signatures during execution require the
separate dynamic-call model.

A runtime policy sees an argument value only when specialization selects it and
generates a typed parameter/helper call. Open-signature runtime generics and
"the argument named by this runtime string" are unavailable; annotations
instantiate once per concrete `Sig`.

Per-signature wrappers and rewrite thunks increase compile time, artifact count,
and code size. In exchange, runtime calls avoid reflection, allocation, array
flattening, and dynamic type checks. Artifact caching should deduplicate
identical specializations by content hash.

Name selectors are rename-sensitive; stable markers cost declaration syntax.
Complex rewrites need explicit ordering, and affine values sharply limit retry.

## Bounded Implementation Shape And Proofs

The smallest coherent implementation sequence is:

1. Freeze a complete typed callable descriptor after all signature directives.
2. Add the structured hook generator and direct ordinary-parameter forwarding.
3. Replace array/object/null decisions with typed hook decisions and the hidden
   evaluator attempt ledger.
4. Add comptime `ParamLens` selection and finite typed rewrite plans.
5. Migrate stdlib hooks, remove dynamic carriers, hash-bind metadata, and use
   one plan for VM, JIT, transfer, and snapshot resume.

Compiler proofs cover mixed modes, nested arrays, defaults, specialization,
selector/rewrite failures, affine rejection, continuation edges, and stable
hashes. Runtime proofs cover same/changed-argument retry, fallback, recovery,
suspension, and VM/JIT parity. A real socket transfers a mixed signature through
opaque placement and proves no argument-array schema is present.

## Conclusion

Runtime named `ArgumentPack` access solves a late-binding problem that callable annotations do not have.
Once `Sig` freezes, sealed descriptors and fragments generate ordinary typed calls; the ledger stays
internal, and generic forwarding/recovery uses checked rewrites and sealed attempts instead of reflection.
