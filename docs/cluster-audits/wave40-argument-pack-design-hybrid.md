# Wave 40T: Opaque `ArgumentPack` With Comptime Lenses

Date: 2026-07-10

## Decision

Keep `ArgumentPack<Sig>` as a first-class immutable runtime carrier, but give it
no runtime named/indexed reflection. Annotation comptime specialization selects
the parameters a policy needs and generates typed, constant-position lenses or
whole-pack transforms. Runtime handlers can then inspect or replace current
values through those capabilities without learning pack structure dynamically.

The answer to "what is runtime named access for?" is therefore: **nothing**.
Names are useful while specializing an annotation and producing diagnostics.
The selected value is useful at runtime because comptime cannot know the
invocation's credential, timeout, tenant, batch size, or failed-attempt input.
A lens bridges those phases without carrying a string lookup into execution.

This hybrid preserves the accepted constraints:

- `ArgumentPack<Sig>` is signature-bound, heterogeneous, immutable, and never
  an `Array<T>` or object;
- `@remote` forwards the complete pack and does not inspect it;
- generic recovery/retry policies may preserve a pack unchanged or apply a
  compiler-validated transform;
- annotation definitions remain ordinary Shape stdlib source specialized only
  after the target signature is frozen; and
- no compatibility layer preserves current array/object/null hook conventions.

## Semantic Surface

For frozen `Sig = (P0[m0], ..., Pn[mn]) -> R`:

```text
opaque affine ArgumentPack<Sig>
opaque ParamLens<Sig, const I, T, Mode>
opaque PackTransform<Sig>

enum HookDecision<Sig, S> {
    Proceed { args: ArgumentPack<Sig>, state: S }
    Return  { args: ArgumentPack<Sig>, result: R, state: S }
}
```

`ArgumentPack` has exactly one runtime position per call-visible parameter
after defaults are materialized once. Comptime-only parameters have no slot.
Each position retains its declared type, pass mode, authoritative `NativeKind`,
storage layout, ownership state, and loans. Immutability means replacement
returns another pack; it does not make owned elements copyable.

The public pack operations are intentionally small:

```text
HookTarget<Sig>::call(args: ArgumentPack<Sig>) -> R
HookTarget<Sig>::proceed(args: ArgumentPack<Sig>, state: S)
HookTarget<Sig>::retry_not_executed(failure, args: ArgumentPack<Sig>, ...)
HookTarget<Sig>::retry_with_permit(failure, args: ArgumentPack<Sig>, permit, ...)
remote::__dispatch(target: HookTarget<Sig>, args: &ArgumentPack<Sig>, ...)

ParamLens::read(&ArgumentPack<Sig>) -> ParamRead<T, Mode>
    where Mode: Readable
ParamLens::reborrow(&mut ArgumentPack<Sig>) -> ParamLoan<T, Mode>
    where Mode: Borrowed
ParamLens::replace(ArgumentPack<Sig>, Replacement<T, Mode>)
    -> ArgumentPack<Sig> where Mode: Replaceable
ParamLens::write_out(&mut ArgumentPack<Sig>, T)
    where Mode: Out
ParamLens::update(
    ArgumentPack<Sig>, total fn(ParamRead<T, Mode>) -> Replacement<T, Mode>
) -> ArgumentPack<Sig> where Mode: Readable + Replaceable

PackTransform::apply(ArgumentPack<Sig>) -> ArgumentPack<Sig>
PackTransform::then(PackTransform<Sig>) -> PackTransform<Sig>
```

There is no `len`, runtime `get(i)`, `get(name)`, index operator, iterator,
field lookup, `to_array`, `to_object`, raw-slot view, arity change, or cast to a
different signature. Pack equality/hashing is also absent because element
ownership and provider identities make a generic meaning unsound.

## Lens Capabilities

A lens is an unforgeable specialization artifact. `I`, `T`, and `Mode` are
compiler facts, not runtime values. A lens may be passed to an effect-polymorphic
generic helper, but monomorphization lowers it to fixed slot operations; there
is no runtime descriptor search. It has no source constructor, serializer,
string name, equality, or conversion to another lens.

Receiver operations depend on pass mode:

| Mode | Read | Replacement |
|---|---|---|
| owned input | scoped `&T` | owned `T`; old slot is released by normal ownership rules |
| copy input | `T` or `&T` | `T` |
| shared borrow | `&T` | another lifetime-compatible `&T` only |
| mutable borrow | reborrowed `&mut T` | another exclusive compatible loan only |
| out parameter | write capability | compatible out binding; no value read before initialization |

The ordinary borrow checker decides whether a lens operation is legal at its
use site. A replacement cannot shorten a required lifetime, duplicate an
affine value, drop an active loan, or turn a value parameter into a reference.
If the failed target consumed a non-replayable owned argument, the evaluator
cannot offer a reusable failed-attempt pack and retry construction remains
unavailable; a lens does not manufacture replayability.

`PackTransform<Sig>` is an immutable typed function over the entire carrier.
It may contain multiple generated lens updates and captured ordinary policy
configuration. It cannot call the target, change `Sig`, inspect unselected
positions, suspend, fail, or perform dynamic reflection. Effectful preparation
happens before `apply`. Composition provides reusable policy building blocks
without adding a `RetrySame`/`RetryWithArgs` decision family.

## Comptime Selection

Selection runs in the accepted `comptime pre/post` pipeline. Signature-changing
directives run first; ordinary inference/checking then freezes `Sig`; lens
selection runs against that final signature before runtime handler compilation.
It is a typed directive in the existing phase, not a new hook kind.

Concretely, `comptime post` emits a selector recipe and source span. The
compiler resolves that deferred recipe only after all directives have re-entered
the checker and the signature is frozen; the comptime handler is not rerun.

Illustrative annotation syntax:

```shape
annotation clamp(param: comptime ParamName, min: number, max: number) {
    targets: [function]

    comptime post(target, ctx) {
        specialize lens value: number = target.param(param).exact()
    }

    before(args, ctx) {
        let next = value.update(args, |current| current.clamp(min, max))
        HookDecision::Proceed { args: next, state: Unit }
    }
}

@clamp(param: #limit, min: 1.0, max: 1000.0)
fn search(query: string, limit: number) -> Array<ResultRow>
```

`#limit` is a comptime `ParamName`, not a runtime string. `exact()` requires one
call-visible parameter and emits a diagnostic for zero or multiple matches.
The declared `: number` requires exact assignment compatibility after
inference. Selection of a comptime-only parameter, unresolved type, unsupported
variadic position, or incompatible pass mode fails specialization.

Selectors may use only deterministic compile-time descriptor facts:

```text
target.param(#name)
target.param_at<const I>()
target.params_with_annotation(@sensitive)
target.params_implementing(Serialize)
```

The first two yield one lens when cardinality and type are known. A heterogeneous
multi-selection does not yield `Array<ParamLens>`. The comptime handler instead
unrolls generation into a typed tuple/record or emits one specialized operation
per selected position. Homogeneous selection may generate repeated code, but
never a runtime pack view.

The specialization environment closed over by a runtime handler is a generated
typed record:

```text
ClampSpecialization<Sig> {
    value: ParamLens<Sig, 1, number, OwnedInput>
}
```

It is compiler metadata, not annotation call state. Runtime `HookState<S>`
remains per invocation and separate.

## Why Both Phases Are Needed

Comptime can already enumerate parameters and generate ordinary typed code. It
should be used for policies that inspect structure: serialize every argument,
derive logs for `@public` parameters, enforce that all inputs implement a
trait, or reject forbidden pass modes. The generated code may contain one lens
operation per selected slot.

Runtime lenses are justified only when the selected slot's **value** matters:

- clamp or normalize a configured parameter;
- replace an expired credential before a proven-safe retry;
- substitute a tenant/correlation capability selected by declaration;
- redact or summarize explicitly marked runtime values; or
- compare one selected failed-attempt input with policy state.

Pure forwarding, remoting, tracing only target identity, and policies based
solely on signature metadata need no lens. This prevents `ArgumentPack` from
becoming a second reflection system merely because it is first-class.

## Forwarding, Failure, And Retry

The cheapest and most general operation is moving the pack unchanged:

```shape
before(args, ctx) {
    HookDecision::Proceed { args, state: RetryState::new() }
}

on_failure(args, failure, state, ctx) {
    FailureDecision::Retry {
        attempt: ctx.target.retry_not_executed(
            failure, args, at: ctx.current.placement, after: state.delay,
        ),
        state: state.next(),
    }
}
```

This generic retry policy does not need arity, names, types, or accessors. The
pack chosen by one attempt is the pack returned to that layer on failure and is
the pack seen by its `after` hook on success.

A policy that intentionally changes one selected argument receives a lens:

```shape
annotation refresh_credential(param: comptime ParamName) {
    comptime post(target, ctx) {
        specialize lens credential: Credential = target.param(param).exact()
    }

    on_failure(args, failure, state, ctx) {
        if !failure.is_auth_rejection()
            return FailureDecision::Propagate { failure }

        let next = credential.replace(args, state.credentials.refresh())
        FailureDecision::Retry {
            attempt: ctx.target.retry_not_executed(
                failure, next, at: ctx.current.placement, after: 0.seconds,
            ),
            state,
        }
    }
}
```

The normal certainty/idempotency gate still owns retry authorization. A lens
changes typed arguments; it does not prove `DefinitelyNotExecuted`, authorize
unknown-outcome replay, select a remote route, or weaken a deadline.

For `@remote`, the runtime handler is intentionally lens-free:

```shape
let result = remote::__dispatch(placement, ctx.target, &args, options)
HookDecision::Return { args, result, state: Unit }
```

The dispatch borrows `args`; the remoting host serializes positions from the
frozen ABI descriptor, then `Return` carries the same pack outward. Provider
discovery, routing, address encoding, transport, auth, codec, negotiation,
deadlines, cancellation, and observability do not receive reflective pack
access. Legitimate routing keys belong in a typed placement or explicit policy,
not an argument-name convention.

## Reusable Policy Interface

Annotation source is generic and specialized per application, so reuse occurs
at the source/type level rather than through an untyped runtime pack API:

```text
fn replace_with<Sig, I, T, M>(
    lens: ParamLens<Sig, I, T, M>,
    make: total fn(ParamRead<T, M>) -> Replacement<T, M>,
) -> PackTransform<Sig>

fn guard<Sig>(
    check: fn(&ArgumentPack<Sig>) -> bool,
    transform: PackTransform<Sig>,
) -> PackTransform<Sig>
```

The second helper's `check` can call only captured typed lenses; pack opacity
prevents generic introspection. Comptime may generate a specialized predicate
that composes several lenses. Specializations are cached by annotation
definition hash, frozen signature hash, target declaration identity, and
comptime selector inputs to control code growth.

Policies should expose semantic selectors rather than raw positions when
published, such as `param: #credential` or `where: @sensitive`. Internally the
compiler resolves them once to positions. A source position selector remains
available for low-level libraries, but diagnostics should show both declared
name and position.

## ABI And Transfer Metadata

The hidden carrier is not required to have one physical representation. VM may
own a kinded slot vector plus sidecars; JIT may scalarize a known pack or use a
frame view. Both obey one semantic descriptor:

```text
SignaturePackDescriptor {
    signature_id: Hash,
    arity: u32,
    params: [ParamAbi {
        position, type_id, pass_mode, kind_constraint, storage,
        default_materialized, ownership_class,
    }],
    return_type_id: TypeId,
}

LensDescriptor {
    signature_id: Hash,
    position: u32,
    type_id: TypeId,
    pass_mode: PassMode,
}
```

Declared names and spans live in comptime/debug metadata for selector
resolution and diagnostics. Runtime lens execution and wire validation use
position plus frozen type/mode/layout facts, never a name. A parameter rename
therefore reruns annotation specialization without creating a runtime ABI lookup.
`kind_constraint` states the carriers permitted by the frozen type; the pack's
per-invocation sidecar remains the authoritative actual `NativeKind`.

The compiler lowers lens calls to constant `PackBorrow<I>` and
`PackReplace<I>` MIR/bytecode operations carrying the signature/layout witness.
The verifier checks the witness against the authoritative frame descriptor.
JIT and VM use the same `HookChainPlan`; neither infers kind from payload bits.

Transferred callable metadata includes the pack descriptor, specialized lens
descriptors, handler/transform dependency edges, and exact next-inner target.
It is hash-covered. A receiver recomputes and validates arity, type IDs, modes,
kinds, and lens positions before execution. Codecs may change bytes, not these
facts. No serialized lens contains an address, pointer, provider object, or
runtime parameter name.

## Misuse Prevention

Reject at compile time:

1. runtime name/index expressions, enumeration, destructuring, or uniform
   array/object projection of a pack;
2. constructing, casting, serializing, comparing, or arithmetically changing a
   lens position;
3. using a lens with another `Sig`, type, mode, or stale pre-transform
   signature;
4. replacing borrowed/out/affine parameters without satisfying ordinary loan
   and ownership rules;
5. runtime selection from user input or provider data;
6. a multi-selector whose cardinality/type requirements are not statically
   resolved;
7. treating pack access as retry permission or execution-certainty evidence;
   and
8. letting a provider codec/routing plugin inspect values through pack metadata.

Runtime guards remain defense in depth for malformed transferred metadata:
signature/layout mismatch, out-of-range constant position, wrong authoritative
kind, duplicate ownership, or stale lens identity becomes an engine/protocol
fault before user code, never a fallback dynamic read.

## Compiler Burden And Tradeoffs

The hybrid adds real machinery: a typed `ParamName`, post-freeze selector
phase, lens and transform IR, pass-mode-specific operations, specialization
records, monomorphized generic helpers, cache keys, ABI/hash metadata, LSP
support, and VM/JIT verifier parity. Comptime's current string-rendered
`ParamDescriptor.type` is insufficient for minting lenses; the directive must
query compiler type identities directly and fail on unresolved types.

Generated handlers can increase code size, especially for "all serializable
parameters" policies. Caching and comptime unrolling make cost proportional to
selected positions, but there is no single tiny reflective loop. Error messages
must map generated operations back to selector source spans and target params.

In return, the runtime interface stays deep and small: preserve, forward, and
apply compiler-authorized typed transforms. Compared with a fully reflective
pack, this loses runtime-selected fields and generic iteration but avoids
dynamic type values, heterogeneous containers, name/version semantics, and a
second serialization API. Compared with comptime-only code generation, lenses
retain a reusable typed vocabulary for policies that must update invocation
values or failed-attempt packs.

The feature is justified only if these runtime value policies are important
enough to warrant lens IR and effect-aware generic specialization. If the real
use cases remain only `@remote` forwarding and whole-pack retry, keep the pack
compiler-internal and choose the comptime-only design instead.

## Proof Boundary

Compile-time proofs should cover name/position/annotation selectors, signature
freeze ordering, exact and failed cardinality, heterogeneous unrolling, generic
specialization caching, every pass mode, affine replacement, and diagnostics.

Plan/metadata proofs should compare VM and JIT lens traces, reject stale/cross-
signature lenses, preserve nested arrays as one slot, validate transferred
descriptors, and prove that `@remote` only forwards. Runtime annotation tests
should cover clamp, redaction, credential refresh, unchanged retry, stacking,
short-circuit, suspension, and authoritative kind preservation.

No production, test, book-site, script, `CONTEXT.md`, or `AGENTS.md` file was
edited. No cargo, just, test, build, extraction, or book-truth command ran.

## Changed File

`docs/cluster-audits/wave40-argument-pack-design-hybrid.md`
