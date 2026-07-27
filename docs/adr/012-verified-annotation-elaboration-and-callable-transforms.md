# ADR-012: Verified Annotation Elaboration and Callable Transforms

## Status

Accepted (2026-07-25)

Clarifies ADR-006 and ADR-009, and composes with ADR-010, ADR-011, ADR-014,
and ADR-015.

Proposed amendment 2026-07-27 (ratified same day, grill Q3): the §5
matrix's async/suspend cell for Python/TypeScript is transitionally
`reject` — the current blocking implementation misrepresents the declared
contract — and reaches its target state `declared contract plus Suspend`
via ADR-019 §5's fast-tracked offload design (days-scale, scoped as
POLY-ASYNC-OFFLOAD), which delivers the cell at the same fidelity as
Shape's own shipped async model.

This ADR remains the authority for annotation elaboration and callable
transforms. ADR-014 owns their shared effect algebra and static capability
classification.

This ADR supersedes any annotation implementation decision that authorizes
source-spelling recognition, AST-shape classification, pseudo-tuples,
annotation-specific compiler markers, homogeneous argument arrays, a universal
`ComptimeTarget`, string-backed applied metadata, or a user-facing type that is
replaced before ordinary resolution and type checking.

## Context

ADR-009 defines annotations as typed compile-stage transformations and ordinary
typed runtime hook templates. The current implementation reached useful
behavior, but its knowledge is spread across parsing, sugar lowering, template
classification, pseudo-argument handling, wrapper weaving, call lowering,
backend exceptions, and hand-written LSP validation.

The most concerning paths include:

- recognizing `HookDecision` and its variants from source spelling and AST
  shape while no ordinary enum value reaches the type system;
- replacing that apparent enum with a compiler-internal tag and branch to keep
  one JIT path native;
- modeling heterogeneous arguments as a pseudo-tuple or homogeneous array;
- substituting `__remote_impl_ref()` and `__remote_arg_pack()` markers;
- extending `__ComptimeTarget` with more optional fields while ADR-009 already
  requires exact target descriptors;
- projecting applied annotations as names and arrays of rendered arguments;
- validating annotations independently in compiler and LSP paths.

These mechanisms are migration evidence, not a foundation to extend.

## Decision

### 1. One deep annotation-elaboration module owns two ordered semantic stages

All annotation semantics pass through one module, but public-contract
elaboration must precede the checks that depend on that contract:

```text
declared_contract(
    callable: CallableIdentity,
) -> Result<NormalizedDeclaredContract<Sig>, DiagnosticSet>

prepare_contract(
    target: ResolvedBaseTarget<T>,
    applications: OrderedTypedAnnotationApplications<T>,
) -> AnnotationContractElaboration<T>

finalize_plan(
    target: FrozenEffectiveTarget<T>,
    prepared: PreparedAnnotationContract<T>,
    body: CheckedTargetBody<T>,
) -> AnnotationBodyElaboration<T>
```

`NormalizedDeclaredContract<Sig>` exists only when every call-visible
parameter and result has a resolved explicit declared type. `_` and body
inference do not satisfy that requirement. Normalization may apply only
resolution-independent declaration rules: receiver normalization, default
materialization, and ABI-visible transforms such as hiding native out
parameters and synthesizing their call-visible result. Resolved generic binders
remain explicit `TypeParamRef` values. Ordinary unannotated inference remains
unchanged; an annotation-generated callable must publish a complete header
before it can be applied. A missing declaration is diagnosed at that
declaration with related annotation-application provenance.

`prepare_contract` runs after the target's identity, generic parameters,
parameters, result, receiver mode, source-body facts, annotation identities,
exact targets, and ConstLift arguments are resolved and typed. It selects and
type-checks clauses, fixes canonical application order, and computes every
externally visible contribution: effective effects, failure outcomes,
ownership, lifecycle requirements, and generated declaration/signature deltas.
Those deltas re-enter the declaration/contract fixed point. The resulting
`PreparedAnnotationContract<T>` participates in the effective contract freeze
before dependent call sites, implementations, overrides, artifacts, or LSP
facts are checked.

`finalize_plan` runs only after that effective contract is frozen and the target
body and specialized hook bodies check against it. It composes state, control
flow, failure, replay, ownership, cleanup, and source maps and produces either
a closed `CheckedAnnotationPlan<T>` or diagnostics. Discovering a new effect or
other public-contract change at this stage is a cycle/error, never a late
signature patch.

Effect contributions are closed, canonical, and stage-separated. Compile-stage
effects authorize only compile-stage evaluation; runtime effects describe only
runtime execution. Operational effects, host grants, scopes, execution
constraints, and provider admission are separate facts. Neither pre-body
contract-row evidence nor the final MIR effect proof grants authority. ADR-014
owns the closed effect algebra, the mandatory `Ffi` effect for foreign calls,
and the capability verifier used after annotation lowering.

The names are architectural, not a commitment to a Rust API. The invariants
are binding:

- every input is resolved and typed to the level required by its stage;
- only this module can construct prepared annotation contracts or checked
  annotation plans;
- invalid or partial contracts/plans never reach dependent checking or
  lowering;
- facts and diagnostics come from these same two queries;
- downstream consumers never reinterpret annotation syntax or names.

The module hides exact clause selection, multiplicity, canonical ordering,
specialization, effect/failure joins, state layout, ownership, cleanup, source
mapping, and annotation-free Core/MIR construction behind this small interface.

`CheckedAnnotationPlan<T>` proves only that annotation contributions compose
into a closed typed semantic transform whose public contract already froze. It
is not execution authority. ADR-010 alone mints execution authority after the
resulting executable MIR, ownership, effects, outcomes, transfers, and teardown
have passed the late region freeze and admission checks.

### 2. Applied annotations and targets are exact typed values

One applied annotation has the semantic shape:

```text
AppliedAnnotation<
    AnnotationIdentity,
    ExactTarget,
    ConstArgumentProduct,
    Multiplicity
>
```

`AnnotationIdentity` is the resolved definition identity from ADR-011.
`ExactTarget` is a concrete target descriptor. Arguments are a generated typed
ConstLift product selected through hygienic annotation-parameter identities.
Multiplicity is proven before elaboration. Every occurrence also has a stable
`ApplicationIdentity` derived from resolved target/application provenance, not
from a source span or display name.

There is no universal annotation target, target-kind string, optional-field
target object, `Array<Any>`, `Array<string>`, or argument recovered from display
text. Annotation support is derived only from ordinary resolved typed clauses;
there is no second `targets` registry.

`on` syntax may remain as concise source sugar, but it must lower to typed
clauses and cannot create separate semantic authority.

Application order is part of the language:

- source-written applications are ordered lexically, first written outermost;
- repeatable occurrences remain distinct through their `ApplicationIdentity`;
- generated applications specify `Outermost`, `Innermost`, `Before(id)`, or
  `After(id)` relative to an exact application identity, and an emitted batch
  has explicit internal order;
- independent generated insertions that leave the same position ambiguous, and
  ordering cycles, are compile errors rather than hash- or discovery-order
  tie-breaks.

The resulting total outer-to-inner order is hash-covered and appears in
compiler/LSP facts and diagnostics. Lexical order is observable syntax; source
locations still do not become semantic identity.

### 3. Compile-stage construction is identity-first and typed

Annotation generators construct declarations from resolved semantic values.
Generated item types enter builders only as `TypeRef` values obtained from
`TypeRef::of<T>()`, exact target reflection, an explicit `TypeParamRef`, or
total constructors such as `option`, `array`, and `fn_`. Those constructors use
the canonical type interner. A rendered type, source fragment, path string, or
display name is never reparsed as type authority.

Typed builders such as `ItemFn` and `ItemType` accept only a `GeneratedName`
paired with each `TypeRef`. A `GeneratedName` is minted from an explicit source
binder, a hygienic identity, or a versioned `TypedNamePolicy`; independently
validating a string never creates naming authority. Generic references bind
through explicit parameter identity rather than strings. Quasiquotation may be
added later as syntax sugar, but it must lower to these same builders and
resolved identities. ADR-011 owns the semantic identity boundary; this ADR
requires annotation generation to stay on its typed side.

### 4. Runtime call interception uses an ordinary typed around-call core

The minimal callable transform is conceptually:

```shape
around<Sig>(
    args: ArgumentPack<Sig>,
    next: Next<Sig>,
) -> ReturnOf<Sig> ! Effects
```

`ArgumentPack<Sig>` is an opaque signature-indexed heterogeneous product over
the normalized call-visible contract. Defaults have already been materialized
and native out parameters are not slots. An unforgeable
`ParamDescriptor<Sig, I, T, Mode>` binds exact callable identity, ordinal,
type, and passing mode; a descriptor from another signature is incompatible
even when its displayed name and type match.

The supported modes have exact operations:

| Mode | Projection | Replacement |
|---|---|---|
| `CopyInput` | copied `T` or scoped `&T` | owned `T` |
| `OwnedInput` | scoped `&T`, never extraction | owned `T` only when displaced `T` is discardable |
| `SharedBorrow` | compatible scoped `&T` | lifetime-compatible shared borrow |
| `ExclusiveBorrow` | scoped exclusive reborrow | lifetime-compatible exclusive borrow |

Replacement consumes the pack and returns the same signature-indexed pack. A
linear or otherwise non-discardable owned slot cannot be replaced in the first
version. The only general operations are pass-through, typed projection, and
typed replacement. There is no runtime length, name lookup, integer indexing,
iterator, dynamic collection, raw slot, cast, structural equality, or public
representation.

`comptime for p in params_of<Sig>` is genuine compile-stage specialization.
Before ordinary body checking it clones and expands the loop body once per
parameter, with an exact `ParamDescriptor<Sig, I, T, Mode>` in each instance.
`where` filters select admissible instances; a missing operation outside a
filter is a compile error, not a runtime branch. Expansion is bounded by a
versioned node and arity budget. Only the compiler may mint a call-site pack;
a trusted remote path may mint one only after proving exact schema, ABI, and
ownership transfer.

`Next<Sig>` is an affine compiler-issued capability for the exact next-inner
continuation, including inner annotation layers and eventually the
implementation. It is not a raw-wrapper bypass and is not selected by name.

This core gives ordinary meanings to the common lifecycle operations:

- proceed: call `next` with an exact argument pack;
- short-circuit: return an exact `ReturnOf<Sig>` without calling `next`;
- after behavior in a raw `around`: ordinary typed code after `next` completes;
- per-invocation state: ordinary lexical typed state around that call;
- cleanup: ordinary ownership, `Drop`/`AsyncDrop`, and ADR-010 teardown;
- retry: a linear `RetryIntent` selected under `ReplayAuthority` and
  `RecoveryBudget`, followed by evaluator-private Retry Commit.

Convenient `before`, `after`, and declarative annotation blocks may remain as
sugar over this model. A composed `before`/`after` layer lowers through one
ordinary typed success join: both an exact short-circuit value produced by its
`before` contribution and a value completed by `next` flow through that layer's
`after` contribution exactly once. Already-entered outer sugar layers do the
same. A raw authored `around` early return retains ordinary early-return
semantics and does not execute code it skipped. This distinction is explicit in
typed Core/MIR; it does not require a second decision protocol.

The spelling-recognized `HookDecision` path is retired. If a library exposes a
decision enum for convenience, it is an ordinary resolved algebraic value:
ordinary generic instantiation, exhaustiveness, type checking, Core/MIR enum
lowering, and backend support apply. Its name and AST constructors have no
compiler privilege.

### 5. Target support is conditional and centralized

Every annotation application enters the common elaborator. An operation runs
only when a `CallableTargetAdapter` proves that normalized lifecycle ABI,
effects, outcomes, ownership, and any requested body access compose for that
target. The initial adapters are `ShapeCallableAdapter` and
`ForeignStubAdapter`; unsupported compositions produce a structured diagnostic
rather than silently dropping a clause.

An around transform wraps the VM-owned normalized call stub, never raw C ABI or
foreign source. Contract-only compile clauses therefore work without a body,
while a body-reading clause rejects `OpaqueForeignBody`. Every foreign call
contributes the runtime `Ffi` effect. The required first-version matrix is:

| Operation | Shape callable | `extern "C"` | Python/TypeScript | Field/parameter |
|---|---|---|---|---|
| resolution, ordering, exact application facts | yes | yes | yes | yes |
| declared contract | exact Shape | normalized visible | declared adapter | exact descriptor |
| contract-only generator | yes | yes | yes | metadata only |
| checked body inspection | yes | reject `OpaqueForeignBody` | reject `OpaqueForeignBody` | not applicable |
| around transform | yes | normalized call stub | normalized call stub | reject |
| async/suspend | declared contract | reject in v1 | transitional reject → declared contract plus `Suspend` (ADR-019 §5, POLY-ASYNC-OFFLOAD) | reject |
| remote placement | portable artifact | admitted library and symbol manifest | admitted extension/provider | reject |
| frozen facts and persistence | yes | yes | yes | yes |

Native out parameters are hidden and represented by a synthesized call-visible
result. Native status and error values remain ordinary declared values;
load/symbol/admission failures are structured pre-entry or evaluator outcomes.
Dynamic-language extension absence is likewise pre-entry, while an entered
foreign call retains its declared `Result`/failure contract. Direct remote
placement is allowed only when the portable artifact names every exact foreign
dependency and receiver admission verifies it.

### 6. Failure handling is explicit and remains distinct from success

Runtime failure is an explicit evaluator outcome, not an implicit Shape
`Result`. A typed `on_failure` clause may receive the structured
`RuntimeFailure` and a sealed failed-attempt capability. Its propagate,
recover, and retry decisions must be ordinary resolved typed operations over
exact `R`, argument, state, effect, replay, placement, and budget contracts.

`EngineFault` and cancellation remain distinct outcomes. `after` remains
success-only. Cleanup is evaluator-owned and total over every entered layer.
Omitted phases elaborate to typed identity/propagation operations so the
checked annotation plan is closed.

The elaborator derives one normative composition:

1. applications enter in the canonical total order, outer to inner;
2. each invocation of an around layer may invoke its exact affine `Next` at
   most once, with no replay exception;
3. success returns inner to outer through ordinary after code; an exact
   short-circuit in `before`/`after` sugar enters that layer's typed success
   join, while a raw `around` return follows ordinary source control flow;
4. failure unwinds inner to outer through typed failure handlers;
5. recovery rejoins the successful path with an exact `R`;
6. retry starts only after the current attempt's cleanup;
7. cleanup runs exactly once for every activated layer on every semantic
   outcome;
8. all paths end in completion, structured runtime failure, cancellation,
   suspension transfer, or contained engine fault.

Replay evidence may enter one linear `RetryIntent`; it never makes a consumed
`Next` reusable. Only the evaluator-private Retry Commit may authorize a new
attempt and re-enter the transform with that attempt's fresh `Next`.

ADR-014 classifies `Next` as affine and `RecoveryObligation`,
`AdmittedExecution`, and ADR-010's teardown continuation as linear. The
post-elaboration verifier checks those classes structurally across products,
enums, branches, and every semantic outcome edge; a runtime consumed flag is
only a debug assertion.

ADR-015 owns retry and durable-recovery sequencing:
`FailureIntent → RetryIntent → cleanup → Retry Commit`, including the linear
journal-backed recovery obligation, persistent budget/deadline, and exact-once
durable settlement. A failure handler selects intent; it never receives
cleanup-completion evidence or mints the next attempt's `Next`.

### 7. `@remote` is an ordinary stdlib transform

The compiler does not recognize the annotation name `remote`. The stdlib
annotation composes the generic protocol using:

- an exact `Placement<P>` and provider-neutral options;
- `ArgumentPack<Sig>`;
- a `PortableContinuationArtifact<Sig>` created by consuming the exact
  `Next<Sig>`;
- an `AdmittedExecution<Sig, P>` minted only after that artifact is admitted at
  the chosen placement under a pinned lease;
- the single typed `Remote Dispatch` contract;
- explicit `Remote(ResolvedProviderIdentity)` and `Suspend` effects, plus `Ffi`
  when the continuation contains a foreign call.

Converting `Next` to a portable continuation consumes its affine local-call
authority. The portable artifact binds the exact next-inner chain, verified
artifact identity, callable lifecycle ABI, closed effects, required grants,
scopes, constraints, captures, and teardown-capability closure, but grants no
placement or execution authority.
Admission validates those facts against the selected receiver and placement,
then mints the non-serializable linear single-attempt
`AdmittedExecution`. Dispatch or explicit abort consumes that capability.
Transport retransmission for the same attempt reuses its `TransferId`; starting
a semantic retry requires replay evidence, a fresh `AttemptId`, a fresh
`TransferId`, and a new admission under the same `RecoveryEpisode`. Before
Retry Commit, any prior inaccessible escrow and `RecoveryObligation` must be
settled or atomically transferred to the new attempt's transaction; ownership
is never duplicated across attempts.

There are no `__remote_impl_ref` or `__remote_arg_pack` markers, no
annotation-specific `__call_raising` proof path, and no homogeneous array
substitute for a heterogeneous signature.

`RemoteOutcome<R>` preserves execution and ownership certainty. It distinguishes
completion, a settled remote failure, confirmed cancellation,
`DefinitelyNotExecuted` with its rejection proof, and `OutcomeUnknown` carrying
inaccessible escrow plus a linear `RecoveryObligation`. A timeout,
disconnect, or missing reply never becomes an evidence-free error.

Transparent `@remote` preserves the declared `R`. Completion projects to `R`;
a settled failure or confirmed non-execution projects to
`Evaluation::Failed(RuntimeFailure::Remote)`; confirmed cancellation projects
to `Evaluation::Cancelled`. `OutcomeUnknown` remains suspended/recovery-pending
or may become terminal failure only after a durable supervisor accepts the
recovery obligation and the transfer receipt is retained in cleanup evidence.
Caller code never resumes with speculatively restored owners.

ADR-015 owns the durable journal transitions, acceptance outcomes, receipts,
episode budget/deadline, and exact-once settlement behind that transfer.

The recoverable surface returns a typed `RemoteCallOutcome<R>` with
`Completed(R)`, `Failed(RemoteError)`, and
`Uncertain(RemoteUncertainty, RecoveryObligation)`. A convenience
`Result<R, RemoteError>` projection is available only when the contract proves
uncertainty impossible or an explicit policy first settles/transfers the
obligation. Both public surfaces remain projections of one dispatch
implementation. Because one variant carries a linear obligation,
both `RemoteOutcome<R>` and `RemoteCallOutcome<R>` are structurally linear and
must be exhaustively consumed or explicitly transferred as a whole.

### 8. `@prompt` contributes a non-vacuous checked template

`@prompt` is an ordinary compile-stage contract annotation. It neither invokes
a model nor silently changes the target implementation. Its required
contribution is a `CheckedPromptTemplate<Sig>` produced conceptually by:

```text
prepare_prompt_template(
    template: ConstString,
    contract: NormalizedDeclaredContract<Sig>,
    traits: ResolvedTraitFacts,
) -> CheckedPromptTemplate<Sig>
```

Preparation runs before effective-contract freeze. `ResolvedTraitFacts` must
already contain every `ToPrompt` obligation required by the template's exact
parameters; prompt preparation cannot defer trait selection until plan
lowering.

The first grammar contains literal UTF-8, `{identifier}` placeholders, and
escaped braces `{{` and `}}`. Arbitrary expressions and format specifications
are rejected. Each placeholder resolves to an exact `ParameterIdentity` with
an exact source span; an unknown name is diagnosed with the nearest parameter
name when one exists. Repeats and unused parameters are legal. A referenced
parameter must be readable through a shared borrow and have a resolved
implementation of:

```shape
trait ToPrompt {
    method to_prompt() -> string ! {}
}
```

`ToPrompt` is pure, total, runtime-effect-free, evaluator-failure-free,
non-suspending, locale-independent, adapter-independent, and canonically
UTF-8. There is no blanket bridge from display formatting, and capabilities or
compile-stage secrets cannot implement it.

The checked template stores literal segments, exact parameter and
implementation identities, source maps, and an ordinary typed renderer recipe.
Those facts, rather than runtime argument values, are hash-covered. Rendering
borrows live values through fixed typed `ArgumentPack` projections and lowers
to annotation-free ordinary calls. The stdlib annotation invokes the general
template intrinsic by resolved identity; the compiler does not recognize the
spelling `prompt`.

The prepared template becomes a frozen contract fact. Post-freeze
`finalize_plan` may consume it to build the renderer, but may not select a new
trait implementation, add an effect, or otherwise change the frozen contract.

This contract is non-vacuous only when another ordinary annotation can consume
the checked fact, render it from the live argument pack, and then call
`Next<Sig>`. That composition must be an acceptance test. It performs no model
or network call and requires no `@prompt`-specific backend path.

### 9. Plans lower once to annotation-free typed Core/MIR

`CheckedAnnotationPlan<T>` lowers before ADR-010's
`RegionTeardownFreezeBoundary` into ordinary resolved typed calls, products,
enums, branches, ownership operations, effect edges, and cleanup regions.

Pre-body contract comparison may produce ADR-014's
`ContractEffectSubsetEvidence`. That evidence binds canonical contract rows and
the effective-contract identity; it does not bind or make a claim about MIR.

The mandatory executable pipeline is:

```text
prepare and freeze the effective contract
  -> check target and specialized hook bodies
  -> finalize and lower the annotation-free typed Core/MIR
  -> perform provenance-preserving semantic optimization
  -> derive VerifiedEffectProof and CapabilityProof on one FinalMirIdentity
  -> ADR-010 verifies and freezes that final composite MIR
```

The final effect pass derives actual effects from the optimized,
outcome-explicit MIR and binds them to the frozen effective contract. The
capability pass verifies the same `FinalMirIdentity`. Every completion,
failure, cancellation, suspension, and contained-fault edge must consume,
return, transfer, or evaluator-teardown each linear value exactly once.
ADR-010's freeze requires both proofs; lowering cannot replace either with a
runtime check.

That final Core/MIR contains no annotation name, hook spelling,
`ComptimeTarget`, marker call, or backend-specific semantic exception. VM and
JIT share it. Optimization may inline and erase `ArgumentPack`, `Next`, state
products, and plan structure only before the two final proofs and while
preserving ownership/outcome/effect provenance. After the freeze, only
realization substitutions covered by its binding are legal; any semantic
change invalidates the certificates and reruns verification.

If ordinary enum, heterogeneous-product, effect, or control-flow lowering is
missing in a backend, that is a general backend gap. It is fixed generally or
surfaced as an explicit backend refusal/fallback; annotation-specific compiler
magic is not an alternative.

### 10. Compiler and LSP consume the same elaboration facts

`AnnotationFacts<T>` is the sole semantic query result for compilation and
tooling. It includes resolved application identity, exact target, typed
arguments, chosen clauses, canonical application order, prepared/effective
contracts, `NormalizedDeclaredContract`, generated symbols,
`CheckedPromptTemplate`, plan shape, closed stage-specific effects, required
grants/scopes/constraints, failures, capability facts, diagnostics, and
bidirectional source maps.

LSP may project these facts for hover, completion, navigation, rename, and
diagnostics. It does not maintain its own target table or annotation validator.

### 11. Migration replaces old paths rather than adapting them

The replacement sequence is:

1. freeze new behavior on spelling, AST, pseudo-tuple, marker, and universal
   target paths;
2. land resolved intrinsic identities and the typed semantic-program boundary
   from ADR-011;
3. land `NormalizedDeclaredContract`, closed stage-specific effects, and the
   ADR-014 capability classifier/verifier;
4. land exact applied annotations, target descriptors, typed ConstLift
   argument products, and identity-first typed item builders;
5. introduce the elaboration module and verify a simple local around transform
   plus mode-branded `ArgumentPack` specialization through both VM and JIT;
6. migrate before/after, short-circuit, failure, state, retry, and cleanup as
   vertical typed slices;
7. land the target adapters and the complete Shape/C/Python/TypeScript
   acceptance/rejection matrix;
8. rebuild `@remote` using `Next<Sig>`,
   `PortableContinuationArtifact<Sig>`, `AdmittedExecution<Sig, P>`,
   `ArgumentPack<Sig>`, and the single Remote Dispatch path;
9. land `CheckedPromptTemplate`, `ToPrompt`, and a real consuming annotation;
10. migrate stdlib generators directly to exact descriptors and typed builders;
11. delete the old classifier, weave, marker, pseudo-tuple, string metadata,
   `__ComptimeTarget`, and annotation-specific backend paths with their final
   consumer.

Each slice must include compiler/LSP facts, VM/JIT differential behavior,
positive alias and rename tests, negative homonym tests, book examples,
forbidden-pattern ratchets, and deletion evidence. Contract-changing slices
also prove callers and LSP observe the effective contract; ordering slices
cover repeatable and generated applications plus ambiguity rejection; remote
slices cover every execution-certainty/ownership state and prove that
OutcomeUnknown cannot reach an evidence-free failure or `Result`. Specializing
pack slices prove no runtime loop remains; foreign slices cover every matrix
cell; prompt slices render live arguments through resolved `ToPrompt`
implementations; capability slices exercise every semantic outcome edge. A
bridge that translates an old untyped plan into the new plan is not an accepted
end state.

## Consequences

- Annotation authors get a small compositional typed model instead of a set of
  compiler-recognized shapes.
- State, failures, effects, ownership, replay, and cleanup remain visible in
  types and lifecycle plans.
- Annotation implementation becomes local: one elaborator and one typed
  lowering, not behavior distributed across parser, planner, weave, JIT, and
  LSP.
- Annotated callables publish an explicit normalized declaration contract;
  foreign targets expose support per operation rather than pretending to share
  Shape bodies.
- Argument-pack specialization can increase compile time and code size, so its
  deterministic expansion budget is part of the language contract.
- Linear recovery, admission, and teardown obligations are rejected
  statically on any unconsumed semantic outcome edge.
- `@prompt` becomes a reusable checked contract fact with an ordinary typed
  renderer, not a validation-only marker.
- VM and JIT performance comes from ordinary optimization over checked Core/
  MIR, not from lying about the source type.
- The migration is larger than extending E4/E6, but it deletes the current
  gravity wells instead of making them permanent.

## Rejected alternatives

- **Make `HookDecision` a reserved spelling with a private runtime
  representation.** This preserves the false surface/compiler reality split.
- **Keep the current paths and add stronger guards.** Proof tokens around
  spelling and AST recognition do not create semantic identity or ordinary
  typing.
- **Add more fields to `__ComptimeTarget`.** This deepens the universal,
  optional-field descriptor ADR-009 rejects.
- **Use arrays for arbitrary callable arguments.** Heterogeneous signatures,
  passing modes, ownership, and exact runtime kinds cannot be represented
  truthfully by a homogeneous collection.
- **Reflect or iterate over an argument pack at runtime.** This would erase
  signature, mode, and ownership facts. Parameter-wise abstraction is bounded
  compile-stage specialization over unforgeable descriptors.
- **Infer missing public types for annotated callables.** The annotation would
  then participate in a circular, order-sensitive contract. Annotated call
  boundaries require a `NormalizedDeclaredContract`.
- **Treat every target as if it had a Shape body.** Foreign transforms compose
  only through a proven normalized stub; body access and unsupported lifecycle
  operations reject explicitly.
- **Use display formatting as the prompt conversion protocol.** Display is not
  necessarily pure, total, canonical, or safe for capabilities and secrets.
  Prompt conversion requires the stricter resolved `ToPrompt` contract.
- **Special-case `@remote` as a flagship exception.** Remote is the acceptance
  test for the general protocol, not a reason to bypass it.
- **Let VM and JIT interpret verified plans independently.** That recreates two
  semantic authorities; both must consume the same annotation-free typed
  Core/MIR.
- **Add annotation effects while weaving the body.** Callers and tooling would
  already have checked a false contract. Contract elaboration must expose every
  public contribution before effective-contract freeze.
- **Collapse remote uncertainty into `RuntimeFailure` or `Result::Err`.** That
  loses execution certainty and linear ownership/recovery obligations. Only a
  settled outcome or an accepted obligation transfer may release the caller
  from that state.
- **Require every runtime failure to become `Result`.** This would change
  transparent placement semantics. Failure must be explicit in evaluator
  outcomes, effects, and lifecycle ABI. `Result` remains an opt-in recoverable
  value surface only where settlement/ownership certainty makes it truthful.

## Related decisions

- ADR-006: Value and Memory Model
- ADR-009: Strictly Typed Comptime and Annotations
- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-014: Closed Effects and Static Capability Ownership
- ADR-015: Recovery Episodes and Durable Obligation Journal
- Typed comptime Decisions 61-65: Annotations and Hooks
