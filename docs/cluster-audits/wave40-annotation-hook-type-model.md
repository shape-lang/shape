# Wave 40C: Annotation Hook Type and Composition Model

Date: 2026-07-10

## Decision

Callable runtime annotations should elaborate through one typed, per-application
hook plan after the target signature is final. The plan preserves the accepted
decisions:

- `ArgumentPack<Sig>` is a first-class immutable value parameterized by the
  complete callable signature.
- Argument replacement is a typed functional update checked by the ordinary
  compiler, not an array/object convention interpreted by the VM.
- A before hook returns `HookDecision::Proceed` or `HookDecision::Return`.
- Every callable hook preserves `Sig`; in particular, `@remote fn f(...) -> R`
  remains `(...) -> R` and transport failure remains a non-returning evaluator
  failure.

This is a callable-hook model. Expression, block, binding, and await
interception need a separate typed `ValueHook<T>` design; they must not reuse a
zero-argument callable pack by convention.

## Current-State Findings

The source model has six handler kinds and seven target kinds, but runtime
handler parameters are only names plus a variadic bit
(`crates/shape-ast/src/ast/functions.rs:202-301`,
`crates/shape-ast/src/parser/extensions.rs:127-203`). Definition compilation
stores before/after AST templates and placeholder function IDs, then infers
handler roles from magic parameter names
(`crates/shape-vm/src/bytecode/core_types.rs:1144-1179`,
`crates/shape-vm/src/compiler/statements.rs:3224-3498`). Explicit `targets:` can
currently override the otherwise-correct function-only inference for
before/after hooks.

Function hooks are specialized per application site. Multiple annotations are
built inside-out, with the first source annotation as the outermost wrapper
(`crates/shape-vm/src/compiler/functions_annotations.rs:2338-2453,
2637-2791`). This produces the useful runtime order
`outer.before -> inner.before -> body -> inner.after -> outer.after`.
Annotated wrappers deliberately have no `Function.mir_data`, because the
available MIR is for the unwrapped body and would omit hooks
(`crates/shape-vm/src/compiler/functions.rs:1091-1138`).

The remaining callable protocol is dynamic:

- `args` is a homogeneous typed array, so heterogeneous parameter types or
  carriers are rejected (`functions_annotations.rs:345-485`).
- A bare array means argument replacement. An object shaped as
  `{args, result, state}` is read through `FieldType::Any`; non-null `result`
  means short-circuit (`functions_annotations.rs:2917-3151`). Null is therefore
  both a language value and a control sentinel.
- Replacing state rebuilds a three-field context schema with only two fields,
  dropping the typed target from the construction
  (`functions_annotations.rs:3123-3143`). No runtime-hook test exercises state.
- After hooks return an unchecked replacement value directly
  (`functions_annotations.rs:3230-3287`). The current test suite even accepts a
  string result from an `int` function
  (`tools/shape-test/tests/annotations_runtime/wrapping.rs:31-49`).
- Expression and await lowering duplicate the same array/object/null protocol,
  use empty argument arrays and contexts without a target, and call
  unspecialized handler IDs
  (`crates/shape-vm/src/compiler/expressions/mod.rs:689-925,928-1315`).
- Foreign-function lowering silently wraps only the first runtime annotation
  (`crates/shape-vm/src/compiler/functions_foreign.rs:335-425`).

The Rust `AnnotationContext` advertises persistent cache/state/registries and
events (`crates/shape-runtime/src/annotation_context.rs:31-168,240-338`), but
generated callable wrappers do not consume it. Their `ctx.state` is a fresh
object on every wrapper invocation. These are distinct state systems and must
not be merged implicitly.

## Typed Callable-Hook Interface

For a finalized callable signature
`Sig = (P0[m0], P1[m1], ..., Pn[mn]) -> R`, use these semantic types (surface
spelling can be designed separately):

```text
opaque ArgumentPack<Sig>
opaque HookTarget<Sig>

enum HookDecision<Sig, S> {
    Proceed { args: ArgumentPack<Sig>, state: S },
    Return  { args: ArgumentPack<Sig>, result: R, state: S },
}

struct BeforeContext<Sig> {
    target: HookTarget<Sig>,
    events: HookEventSink,
}

struct AfterContext<Sig, S> {
    target: HookTarget<Sig>,
    state: S,
    events: HookEventSink,
}

type BeforeHook<Sig, S> =
    fn(args: ArgumentPack<Sig>, ctx: BeforeContext<Sig>)
        -> HookDecision<Sig, S>

type AfterHook<Sig, S> =
    fn(args: ArgumentPack<Sig>, result: R, ctx: AfterContext<Sig, S>) -> R
```

Annotation arguments are ordinary typed values closed over when the handler is
specialized; they are not part of the invocation pack. Handler roles are
determined by handler kind and parameter position, never by names such as
`args`, `result`, or `ctx`.

Every decision variant carries the effective pack and same-layer state. That
makes the after-hook input defined even when a before hook short-circuits.
Without a before hook, the compiler synthesizes
`Proceed { args: incoming, state: Unit }`. Without an after hook, the state is
dropped after the continuation or replacement result is obtained.

### `ArgumentPack<Sig>` invariants

- It has exactly one position per call-visible runtime parameter after defaults
  have been materialized once, before the outermost before hook.
- Each position retains its declared type, pass mode, and authoritative
  `NativeKind`. Comptime-only parameters have no runtime position.
- Constant-position access returns `Pi`; a functional update such as
  `pack.replace<I>(value)` consumes or borrows the old pack and returns a new
  `ArgumentPack<Sig>`. `value` must satisfy the normal assignment rules for
  `Pi[mi]`, including reference/out mode and ownership rules.
- A pack may be bound, passed, returned, and accepted by generic helper
  functions parameterized by the same `Sig`. Immutability does not make its
  elements copyable; ordinary move/borrow rules still apply.
- It has no implicit `Array<T>` view, dynamic heterogeneous index, arity-changing
  operation, raw-bits projection, or conversion to a pack for another
  signature.
- `HookTarget<Sig>::call(pack)` and compiler-known remote dispatch elaborate
  the pack positionally. Nested `Array<T>` remains one parameter slot.

These rules extend ADR-006's authoritative-kind invariant rather than deriving
kinds from values (`docs/adr/006-value-and-memory-model.md:1535-1583,
1876-1880`).

### Target and state semantics

For layer `i`, `ctx.target` is the exact typed **next-inner continuation**: the
suffix of the hook chain after layer `i`, ending at the raw implementation. It
is not an escape hatch that silently bypasses inner annotations. This matches
what chained wrappers currently place in `impl_idx` and makes stacking with
`@remote` precise: an outer `@remote` transfers/invokes the inner continuation;
an inner `@remote` runs outer hooks locally and its continuation remotely.

Calling `ctx.target` is explicit and may happen zero, one, or multiple times
(retry is a legitimate use). Returning `Proceed` still asks the wrapper to call
the continuation once; a hook that already obtained the replacement result
normally returns `Return` to avoid a second call.

`S` is call-local, same-layer phase state. A before hook creates it and the
same layer's after hook observes it. State does not flow between annotation
layers or across calls, and it is not transferred to an outer layer. Persistent
annotation state requires a separate explicit store design covering application
identity, recursion, synchronization, snapshot, and remote placement; the
dormant Rust `AnnotationContext` is not that contract. Event emission is a
separate capability and has execution-order, non-transactional semantics.

## Phase Model

1. **Definition validation.** Parse the annotation and validate one handler per
   kind, exact positional handler shapes, runtime non-variadic rules, and a
   target set valid for every declared handler. A before/after handler makes
   every permitted runtime target callable.
2. **Comptime pre.** Run all `comptime pre` handlers in source order
   (outermost first) against provisional target descriptors.
3. **Inference and comptime post.** Run normal constraint/inference work, then
   all `comptime post` handlers in the same source order. Apply directives
   through the normal AST/registration path.
4. **Signature freeze.** Re-run ordinary declaration/body checking after all
   signature-affecting directives. Freeze one concrete `Sig`; no runtime hook
   may alter it. Generic functions receive a hook specialization per concrete
   instantiation, never an `unknown` pack.
5. **Hook elaboration.** Type-check each before/after template against the
   frozen `Sig`, infer its private `S`, validate every `Proceed`, `Return`, and
   pack replacement, then build one `HookChainPlan<Sig>` in source order.
6. **Definition lifecycle.** `on_define` and `metadata` remain outside the
   invocation plan. They run in defined source order after target identity is
   known, cannot mutate the frozen signature/plan, and do not share call-local
   hook state.
7. **Runtime invocation.** Bind/default arguments once, construct the pack,
   then execute the plan below.

Steps 2-4 preserve the accepted pre-then-post ordering in
`docs/vision/rfc-comptime-transform-api-v1.md:60-68` and the normal-checker
re-entry requirement in `docs/design/comptime-excellence.md:323-332`.

## Composition and Short-Circuit Order

For source annotations `A0, A1, ..., An` (outermost to innermost):

```text
A0.before(args0)
  Proceed(args1, s0) -> A1.before(args1)
    ...
      Proceed(argsN, sn) -> body(argsN) -> An.after(..., sn)
    -> A1.after(..., s1)
  -> A0.after(..., s0)
```

The rules are:

- Before hooks enter outer-to-inner; successful results unwind through after
  hooks inner-to-outer.
- `Ai.Return { args, result, state }` skips the automatic invocation of every
  deeper layer and the body. `Ai.after` still runs, followed by already-entered
  outer after hooks. Skipped inner layers run neither hook unless `Ai` invoked
  its `ctx.target` explicitly.
- Each after hook sees the effective pack selected by its own before hook, not
  a mutation made by a deeper layer. Result values alone flow outward through
  the after chain.
- A runtime/evaluator failure in any before hook, continuation, body, or after
  hook aborts immediately. Pending after hooks do not run. `after` is
  success-only, not `finally` or an error handler.
- For `Void`, `Option`, nullable, and domain `Result` returns, enum variants
  carry the typed value directly; no null sentinel participates in control
  flow.

`@remote` is then an ordinary transparent before hook:

```text
Return {
    args,
    result: remote_call_raising(addr, ctx.target, args),
    state: Unit,
}
```

Its result is exactly `R`; a domain `Result<T, E>` remains that same `R`, while
transport/protocol failure exits through the evaluator failure channel. This
agrees with `docs/design/distributed-function-transfer.md:176-212` and
`docs/cluster-audits/wave40-remote-annotation-error-model.md:1-32`.

## Invalid Combinations

The compiler should reject, at definition or application time:

- duplicate handler kinds in one annotation, or duplicate application of the
  same annotation to one target (the latter is already checked at
  `compiler_impl_reference_model.rs:1765-1803`);
- before/after on expression, await, block, binding, type, or module targets;
  explicit `targets:` cannot override handler-family compatibility;
- wrong handler arity/role order, variadic runtime handlers, or a handler that
  does not return its required semantic type on every reachable branch;
- a `Proceed`/`Return` carrying a different signature's pack, changed arity,
  wrong slot type/pass mode, runtime-computed heterogeneous index, or uniform
  array conversion;
- different state types across decision branches, or an after hook whose state
  type differs from its same-layer before hook;
- `Return.result` or an after result not assignable to the frozen `R` through
  ordinary typed conversion. Signature-changing after hooks, including the
  current `int -> string` test, require a separately declared transform
  interface;
- `@remote` adding `Result`, `Future`, parameters, or any other signature
  layer; recoverable transport remains explicit `remote::call`;
- treating after as an error/finally hook, sharing state between layers, or
  accessing persistent state through the call-local context;
- silently dropping extra foreign-function annotations. Foreign callables must
  use the same complete hook plan or fail compilation until they can.

## Compiler Module and Metadata Boundary

The deep module should expose one small compiler interface:

```text
elaborate_callable_hooks(target, applied_annotations) -> HookChainPlan<Sig>
lower_callable_hooks(plan, terminal) -> callable body
```

It owns signature witnesses, handler specialization, pack operations, state
typing, composition order, and diagnostics. Regular functions, methods, and
foreign functions are adapters into that interface. Expression/await lowering
is not an adapter because it has a different semantic target; it needs the
separate `ValueHook<T>` module.

The plan must also be the metadata source for both execution modes. Either MIR
is lowered from the complete plan or the wrapper remains VM-only; attaching
unwrapped-body MIR to a wrapped function is invalid. Function blobs must retain
the continuation and specialized-handler dependency edges, and every generated
callable must carry the same frozen-signature frame descriptor. The pack's
per-position kinds come from that signature/frame metadata, so transfer and JIT
must not reconstruct them from payload bits.

## Focused Proof Boundary

Compiler assertions should inspect a `HookChainPlan` directly: mixed parameter
types and modes, nested `Array<int>` as one slot, default materialization,
generic specialization, exact continuation signatures, state unification, and
all invalid replacements/results above.

Runtime tests should use exact event logs and non-commutative transforms to pin
`A.before, B.before, body, B.after, A.after`; cover `Return` at each layer,
same-layer after execution, skipped inner hooks, state isolation, `Void`/null/
`Option`/domain `Result`, and failure bypass of all pending after hooks. Existing
stacking tests only assert final values, and the chained argument test uses
commutative `+10`/`+5` operations
(`annotations_runtime/before_after.rs:87-122`,
`annotations_runtime/injection.rs:166-194`).

A real-socket regression should exercise `@remote` both outside and inside a
second annotation, assert execution location/order, preserve the declared
function type, and send a mixed/nested pack without arity or kind loss. The
book boundary should retain transparent `R`, domain-`Result` pass-through, and
the explicit `remote::call` recoverable-error contrast. Run the same hook-order
and short-circuit cases in VM and JIT modes to prove that MIR metadata never
bypasses the plan.

## Scope

This scout made no production, test, book, script, `CONTEXT.md`, or `AGENTS.md`
changes and ran no build or test command.
