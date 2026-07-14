# Wave 40U: Generalized Signature And Argument Pack Design

Scope: clean-break interface design over accepted annotation, policy, remote,
comptime, generic, function, and reflection contracts; no implementation claim.

## Decision

The strongest coherent public feature is not "runtime named access to annotation
arguments." It is a new **ordered parameter-row kind** with a family of
signature-indexed types:

```text
ParamRow
Signature<Params: ParamRow, Return, Effects>
ArgumentPack<Sig>
Callable<Sig>
PackTransform<FromSig, ToSig>
```

That feature can support true variadic generics, independently compiled
middleware, generic RPC infrastructure, forwarding, partial application,
signature adapters, and constrained reflection. `ArgumentPack<Sig>` remains an
immutable complete invocation carrier with authoritative per-position types,
modes, ownership, and kinds. It never becomes `Array<Any>` or a heterogeneous
object inspected by strings.

However, current annotation and remoting needs do **not** justify exposing this
language-wide machinery:

- annotation definitions are Shape stdlib templates specialized after the
  target signature freezes, so strictly typed comptime can derive ordinary
  accesses without text or dynamic values;
- `@remote` only forwards the exact pack to `Callable<Sig>`/`RemoteDispatch`;
  it does not need arbitrary field access; and
- bounded failure/retry policies need only preserve a pack or apply a
  compiler-validated same-signature transform.

Most other proposed examples also have a concrete signature at specialization
time and compile better as generated ordinary code. Public generalized packs
become justified only if Shape intentionally supports reusable code compiled
once for an **unknown signature**, especially middleware stacks, callable
registries, or existential RPC envelopes.

Recommendation: keep `ArgumentPack<Sig>` opaque and compiler/runtime-internal.
Give annotation and policy authors sealed typed descriptors, hygienic symbols,
typed lenses, and checked fragments that re-enter normal compiler checking.
Ratify public packs only with variadic rows and first-class `Callable<Sig>`;
runtime named access alone does not deliver the generalized use cases.

## What Runtime Named Access Is Actually For

When `Sig` is concrete during compilation, this:

```shape
let tenant = args.borrow<tenant>()
```

can elaborate directly to the target's typed local/slot. It is ergonomic syntax,
not a runtime capability. Comptime already knows `target.params` and can generate
that code.

Runtime pack access matters only when the same compiled body must accept many
different signatures:

1. middleware constrained by selected named parameters;
2. generic apply/bind/curry/compose/forwarding over row variables;
3. existential RPC/router/actor envelopes without decoding to `Any`;
4. typed visitors for recorders, redactors, validators, or codecs; and
5. runtime callable adapters built from validated signature maps.

If Shape does not want those independently compiled abstractions, there is no
strong runtime-access case. Annotation specialization is not evidence for it.

## Current Boundary

The AST has fixed function/type parameter vectors but no row kind, variadic type
variable, label constraint, or signature algebra
(`crates/shape-ast/src/ast/types.rs:11-45,190-304`; `functions.rs:157-188`). `...`
supports spread and annotation comptime configuration, not general forwarding
(`crates/shape-ast/src/shape.pest:428,1370-1395`).

Comptime descriptors expose names and rendered/typed parameter forms
(`crates/shape-vm/src/compiler/comptime_target.rs:1-15,227-270,439-455`), enough
to derive a concrete wrapper but not row polymorphism. Native `&[KindedSlot]`
variadics (`crates/shape-runtime/src/marshal.rs:2498-2630`) are an internal ABI,
not a public heterogeneous Shape value.

## Strictly Typed Comptime Baseline

"Generated ordinary code" does not mean Zig-style text generation. After the
target signature freezes, a Shape annotation template receives compiler-sealed
values such as:

```text
TargetDescriptor<Sig>
ParamDescriptor<Sig, K, T, Mode>
ParamLens<Sig, K, T, Mode>
HygienicSymbol<T>
CheckedExpr<T, Effects, Ownership>
CheckedItem<DeclaredSig>
```

Descriptors offer typed queries such as `param<K>()`; a lens is a proof that `K`
selects exactly one slot of type `T` and mode `Mode`. Fragment builders accept
only typed descriptors, symbols, lenses, and child fragments. Hygienic symbols
are unforgeable compiler identities, not spelling-based lookups.

The hook returns an expansion plan made of opaque checked fragments. Insertion
still re-enters ordinary name resolution, typing, effect inference, ownership,
affinity, lifetime, and exhaustiveness checks before VM/JIT lowering. "Checked"
means the builder preserves invariants; it is not authority to bypass context.

There are no source strings, parser round-trips, JSON AST payloads, dynamic
`Any`, unchecked AST constructors, or runtime reconstruction by parameter name.
A failed query or ill-typed fragment is a compile error at the annotation use.
This typed elaboration path is the required alternative to public runtime pack
access for every signature known after specialization.

## Type Model

```text
kind ParamRow
Param<Label, Type, Mode = Own | Borrow | BorrowMut | Out,
      Default = Required | Defaulted>
Signature<Params: ParamRow, Return, Effects>
type Sig = Signature<[Param<source, Source>,
                      Param<limit, int, Own, Defaulted>],
                     Batch, { Suspend, AsyncCleanup }>
opaque ArgumentPack<Sig>
opaque Callable<Sig>
```

`ArgumentPack<Sig>` is complete: defaults are materialized exactly once before
construction, and it has one runtime slot for every call-visible parameter.
Comptime-only parameters have no runtime slot. A partial application uses a
different type; an incomplete pack is never represented as an `ArgumentPack`.

### Stable labels

Named access makes parameter **labels** signature identity, separate from local
bindings. Renaming one changes the descriptor, callable type, ABI/content hash,
and named-call surface. Anonymous parameters get stable ordinal keys such as
`#0` but no source label. Clean-break syntax makes labels explicit:

```shape
fn send(to destination: Destination, payload bytes: Bytes) -> Receipt
// labels: `to`, `payload`; local bindings: `destination`, `bytes`
```

Duplicate labels reject; destructuring after binding creates no new labels.

### Ownership and modes

Immutability forbids in-place slot replacement; it does not make elements
copyable. Move, borrow, and functional update preserve every mode and lifetime.
A pack containing `BorrowMut` is affine: it cannot be duplicated, outlive its
borrow, cross an incompatible task boundary, or serialize.

Authoritative `NativeKind`, schema identity, and provenance travel beside each
slot under the frozen signature. No operation infers type or kind from bits.

## Core Operations

```text
ArgumentPack<Sig>::arity() -> const int
ArgumentPack<Sig>::borrow<K: ReadableKey<Sig>>(&self) -> ViewAt<Sig, K>
ArgumentPack<Sig>::replace<K: ReplaceableKey<Sig>>(self, CarrierAt<Sig, K>)
    -> ArgumentPack<Sig>
ArgumentPack<Sig>::update<K: ReplaceableKey<Sig>>(
    self, pure fn(CarrierAt<Sig, K>) -> CarrierAt<Sig, K>)
    -> ArgumentPack<Sig>
Callable<Sig>::call(self_or_ref, ArgumentPack<Sig>) -> ReturnOf<Sig>
PackTransform<A, B>::apply(self_or_ref, ArgumentPack<A>) -> ArgumentPack<B>
```

`K` is a compile-time key, written `pack.borrow<tenant>()` or
`pack.borrow<#0>()`, never a runtime string. `ViewAt` and `CarrierAt` are derived
from the frozen type, mode, and lifetime. Wrong labels or mode mismatches are
compile errors. `replace` consumes the pack, so it cannot duplicate untouched
owned slots. A `BorrowMut`/`Out` carrier is not readable through a shared view;
it can only be forwarded or consumed by a mode-safe transform.

Convenience syntax may build or destructure a complete pack when `Sig` is known:

```shape
let args: ArgumentPack<SendSig> = pack(to: destination, payload: bytes)
let pack(to, payload) = args
```

This is distinct from an object literal. Labels, order, modes, defaults, and
return/effect signature remain part of `SendSig`.

### Operations deliberately absent

- implicit `Array<T>`, tuple, object, map, JSON, or raw-bits projection;
- dynamic `get(name: string) -> Any` or runtime heterogeneous index;
- arity mutation or runtime-data-dependent reorder of an existing `Sig`;
- unchecked conversion between signatures with structurally similar slots;
- clone when any slot is affine or borrowed; or
- generic serialization without a codec specialized for `Sig`.

Nested `Array<T>` remains one parameter. Homogeneous runtime varargs remain
`Array<T>`; parameter rows are for compile-time heterogeneous arity.

## Row Algebra And Signature Maps

Arity-changing operations happen at the type level and produce proof objects:

```text
Concat<A: ParamRow, B: ParamRow> -> ParamRow
Project<P: ParamRow, Keys> -> ParamRow
Remove<P: ParamRow, Keys> -> ParamRow
Rename<P: ParamRow, From, To> -> ParamRow
Reorder<P: ParamRow, Keys> -> ParamRow

PackTransform<FromSig, ToSig> {
    source_for_each_target: typed positional mapping,
    conversions: total typed functions,
    inserted: typed default/value providers,
}
```

The compiler constructs a `PackTransform` only after proving every destination
slot has exactly one source or insertion, every source move is used at most once,
labels are unique, conversions are legal, and borrow/effect lifetimes survive.
Transforms are immutable plans; they do not inspect runtime descriptors to
decide types.

For same-signature failure policy, `preserve` is identity and replacement uses
only `replace<K>`/`update<K>`. A retry with `OutcomeUnknown` still needs the
accepted idempotency/dedup/equivalence gate; type-safe pack transformation alone
does not prove semantic retry safety.

## Variadic Generics

The generalized feature's strongest use is variadic type parameters over rows:

```shape
fn apply<..P: ParamRow, R>(
    target: fn(..P) -> R,
    args: ArgumentPack<fn(..P) -> R>,
) -> R {
    target.call(args)
}

fn around<..P: ParamRow, R, M: Middleware<fn(..P) -> R>>(
    middleware: M,
    target: fn(..P) -> R,
) -> fn(..P) -> R
```

`..P` is one type-level row variable, not a runtime rest array. Each concrete
instantiation still has a fixed frame descriptor and ABI. Constraints can state
row membership without knowing the rest:

```text
where P has tenant: TenantId
where P lacks raw_credentials
where every P satisfies SerializableParam
```

This requires real row unification, duplicate-label diagnostics, row-tail
variables, and effect/return preservation. Merely adding `ArgumentPack` without
these rules does not implement variadic generics.

Monomorphized row code can lower every key to a direct offset and needs no
runtime named lookup. A middleware binary that truly ships once additionally
needs a checked row-witness/dictionary ABI; without that ABI, specialization is
still comptime generation under a generic spelling.

## Middleware And Forwarding

A generalized middleware interface can be independently compiled when that row
witness ABI is available:

```shape
trait Middleware<Sig> {
    async fn call(
        self,
        next: Callable<Sig>,
        args: ArgumentPack<Sig>,
    ) -> ReturnOf<Sig>
}

type RequireTenant<Sig> where Params<Sig> has tenant: TenantId

impl Middleware<Sig> for RequireTenant<Sig> {
    async fn call(self, next, args) -> ReturnOf<Sig> {
        authorize(args.borrow<tenant>())
        next.call(args)
    }
}
```

This is the strongest reason for runtime named access: `RequireTenant` can ship
once for any future signature satisfying the constraint. Comptime can generate
equivalent wrappers, but each concrete signature gets another specialization.

Middleware that only observes every parameter uses a typed visitor, not `Any`:

```text
trait ParamVisitor {
    fn visit<K, T, M>(descriptor: ParamDescriptor<K, T, M>, value: &T)
}

args.visit(redacting_recorder)
```

This requires higher-rank generic visitor support. Without it, generic logging,
validation, and encoding should remain comptime-generated rather than gain a
dynamic escape hatch.

## RPC, Remote Forwarding, And Existentials

The accepted remote annotation needs only:

```text
ctx.target.call_or_dispatch(args: ArgumentPack<Sig>)
```

`@remote` must not inspect or rewrite the pack. The host uses a codec specialized
for `Sig`, validates its frozen descriptor, and sends a canonical invocation;
providers never receive an untyped argument array.

A generalized RPC framework can quantify over `Sig`:

```text
RpcCodec<Sig>::encode(&ArgumentPack<Sig>) -> EncodedArguments
RpcCodec<Sig>::decode(EncodedArguments) -> Result<ArgumentPack<Sig>, DecodeError>
Router::dispatch<Sig>(Callable<Sig>, ArgumentPack<Sig>) -> ReturnOf<Sig>
```

For a registry where the signature is unknown until runtime, use an existential
envelope:

```text
SomeInvocation = exists Sig. {
    descriptor: SignatureDescriptor<Sig>,
    target: Callable<Sig>,
    args: ArgumentPack<Sig>,
}
```

Consumers may forward, hash/trace through a typed visitor, or invoke the paired
target. They cannot extract a named value as a concrete type until a checked
signature match opens the existential. This preserves strict typing while
supporting actor mailboxes, plugin routers, and generic RPC queues.

Most RPC endpoints do not need this: the endpoint signature is known, so
comptime should derive `RpcCodec<Sig>` and direct encode/decode code.

## Currying, Binding, And Adapters

Partial invocation has its own affine type:

```text
BoundCallable<Sig, BoundKeys> {
    target: Callable<Sig>,
    bound: BoundValues<Sig, BoundKeys>,
}

bind<Ks>(Callable<Sig>, ProjectedValues<Sig, Ks>)
    -> Callable<RemoveParams<Sig, Ks>>
```

Binding consumes owned values and captures borrows with explicit lifetimes.
Calling the result combines remaining arguments with the bound values through a
compiler-proven `PackTransform`. Rebinding an already bound key rejects.

Adapters use the same transform plan for renaming, reordering, inserting a
context parameter, dropping an acknowledged parameter, or mapping one typed
value. There is no general "compatible shape" cast. This can support explicit
version adapters between RPC signatures without weakening identity.

For concrete callables, comptime can generate smaller direct closures for all
of these operations. First-class bound/adapter values matter only when callers
need to construct and compose them dynamically.

## Reflection Boundary

Keep reflection in three tiers:

1. `signature_of<T>()` and `target.params` are comptime descriptors used to
   derive lenses, transforms, codecs, and direct wrappers.
2. `SignatureDescriptor<Sig>` is a runtime read-only witness for diagnostics,
   negotiation, hashes, and checked existential opening. It cannot produce
   types from strings.
3. `ArgumentPack<Sig>` exposes static keys and typed visitors. Dynamic string
   lookup may return metadata or an opaque existential parameter reference, but
   never `Any`, raw bits, or an inferred concrete value.

Runtime reflection does not permit arity/type mutation. Any transformation that
changes a signature originates from a compile-time `PackTransform` witness.

## ABI, Ownership, And Security

The frozen signature witness covers parameter labels/order, types, modes,
defaults-after-materialization policy, return/effects, frame layout, schema
identity, and authoritative kinds. It enters callable identity, content hashes,
JIT/VM metadata, remote negotiation, and snapshot compatibility. Labels cannot
be presentation-only if runtime named access depends on them.

`ArgumentPack` owns a normal typed carrier per slot, including provenance
sidecars. Drop, move, borrow, closure capture, task transfer, FFI, snapshot, and
wire paths must preserve those carriers exactly. A live borrowed/resource pack
is a snapshot/wire barrier unless its owning protocol explicitly supports it.

Pack descriptors and values are capabilities. Reflection redacts secret values;
serialization requires explicit permission and a `Codec<Sig>`; an existential
signature match includes principal/provider/schema/version checks. Pack labels
must not be used as authorization or idempotency proof.

VM and JIT lower static key access to the same slot and consume the same
`PackTransform` plan. Neither may fall back to runtime string search or replay a
call after a failed specialization.

## Use-Case Assessment

| Use case | General pack adds unique value? | Better default today |
|---|---|---|
| Specialized annotation hooks | No | Typed comptime descriptors/lenses/fragments |
| `@remote` transparent forwarding | No | Opaque internal pack and direct dispatch |
| Failure/retry preserve/replace | Little | Same-signature internal pack plus generated transforms |
| Fixed RPC endpoint codec | No | Comptime-derived direct codec |
| Concrete forwarding/adapter | No | Generated ordinary typed call/closure |
| Concrete currying | Usually no | Generated closure with captured values |
| Signature-polymorphic middleware | Yes | Requires `ParamRow` and `Callable<Sig>` |
| Dynamic callable registry/router | Yes | Requires existential `SomeInvocation` |
| User-composed runtime adapters | Yes | Requires first-class transform/bound values |
| Dynamic heterogeneous varargs | No | Refuse; use homogeneous `Array<T>` or typed rows |

The first six cases cover the accepted Wave-40 requirements. They do not pay for
the feature. The next three are legitimate but represent a broader language
direction that Shape has not otherwise committed to.

## Complexity And Tradeoffs

A public generalized design adds:

- a `ParamRow` kind, row-tail variables, membership/lacks/every constraints,
  row unification, and variadic generic specialization;
- stable parameter-label identity and source/ABI migration rules;
- `Callable<Sig>`, existential signatures, higher-rank visitors, transform
  witnesses, partial packs, and affine bound callables;
- ownership/borrow checking across heterogeneous pack operations;
- descriptor/hash/bytecode/JIT/FFI/snapshot/wire versioning;
- sealed fragment IR, symbol hygiene, compiler re-entry, caching, and diagnostics
  for the strictly typed comptime path;
- monomorphization and code-size controls for row-heavy libraries; and
- LSP display, completion, rename, hover, and diagnostics for row equations.

It can deepen the language if these pieces form one callable-programming module.
It pollutes the language if only annotations use `get<name>` while every deeper
operation remains compiler-private.

Comptime specialization has costs too: generated code volume, one specialization
per concrete signature, and no single runtime middleware value. Those costs are
preferable while use cases are predominantly static. Generalized packs win only
when separately compiled signature-polymorphic values are a product goal.

## Ratification Threshold

Do not expose runtime named access until all of these are accepted together:

1. at least two non-annotation use cases require unknown-`Sig` code, with one
   being signature-polymorphic middleware or an existential callable registry;
2. parameter labels are declared stable signature identity;
3. variadic generics and row constraints are designed for functions, traits,
   higher-order values, and effects;
4. ownership rules cover every pack mode and transform;
5. runtime reflection remains visitor/existential based, with no `Any` escape;
6. VM/JIT/ABI/hash/snapshot/wire metadata share one signature witness; and
7. strictly typed comptime generation remains the concrete-signature path.

Until then, the deep boundary is:

```text
compiler/comptime: derive ParamKey and ArgumentTransform for frozen Sig
runtime policy:    preserve or apply validated same-Sig transform
remote dispatch:   forward opaque ArgumentPack<Sig>
user code:         ordinary typed parameters and generated helpers
```

This gives annotations and recovery policies signature safety without forcing
every Shape user to learn parameter-row programming.

## Proof Boundary

If ratified, compiler proofs cover row unification, labels, modes/defaults,
transform totality, variadic specialization, existential opening, and rejection
of heterogeneous access. Ownership proofs cover affine moves, borrows, binding,
task transfer, and no duplicate release.

VM/JIT proofs compare slot access, transforms, call results, and failure traces.
Provider/RPC proofs validate signature identity and codec arity/kinds. Snapshot
and wire proofs refuse live borrowed/resource packs and round-trip only explicit
serializable signatures. Comptime parity compares generated direct wrappers with
generalized operations for the same signature.
