# ADR-009: Strictly Typed Comptime and Annotations

## Status

Accepted (2026-07-11)

Clarified by ADR-011 and ADR-012 (2026-07-25). Dated slice and program
decisions are implementation history where they conflict with those ADRs; they
do not amend this architecture implicitly.

## Decision

Shape has two composable but independent mechanics:

1. `comptime {}` and `comptime fn` execute typed staged Shape code during
   compilation and produce values, checked code fragments, semantic rewrites,
   or tracked artifacts accepted by the exact surrounding expansion sink.
2. Annotations attach typed compile-stage transformations and/or ordinary
   runtime hook templates to explicitly supported target kinds and phases.

An annotation may call comptime code while it is specialized. A runtime hook
never invokes the comptime engine. Comptime used outside an annotation has the
same evaluator, capabilities, effect tracking, fragment algebra, and tooling.

All reflection and generation is semantic and strictly typed. Public comptime
interfaces contain no source strings, parsed string payloads, JSON AST, token
trees, name-selected compiler mutation, dynamic `Any`, or unchecked AST escape
hatch. Generated code re-enters the ordinary type, effect, ownership, borrow,
cleanup, and backend pipeline.

## Semantic Freeze

Compiler descriptors are issued only after the relevant semantic state is
complete. Inference variables, internal `Any`, unknown storage kinds, and
dynamic-schema fallbacks fail before user comptime code executes. Declared
generic parameters remain representable through typed parameter identities;
they are not inference holes.

Every descriptor and operation carries the canonical resolved definition
identity established by ADR-011. Terminal spelling, AST constructor shape,
unspellable rendering, source origin, and ambient compiler context cannot
select an annotation, phase, intrinsic, variant, target, or capability.

Semantic transformation is stage-indexed. Module/item outputs re-enter the
declaration-discovery fixed point. Annotation and signature contributions are
computed from resolved typed base contracts before effective contracts freeze
and dependent callers are checked. Body/plan elaboration then produces checked
declarations or annotation-free typed Core/MIR before optimization and backend
lowering. No stage uses raw parsed syntax as semantic authority or discovers a
public effect after its consumers have already checked.

`TypeRef<T>` is an opaque canonical type identity. Reflection returns the
exhaustive indexed sum:

```shape
FrozenType<T> =
    Primitive(FrozenPrimitive<T>)
  | Never(FrozenNever<T>)
  | Parameter(TypeParamDescriptor<T>)
  | Nominal(FrozenNominal<T>)
  | Tuple(FrozenTuple<T>)
  | Record(FrozenRecord<T>)
  | Callable(FrozenCallableType<T>)
  | Reference(FrozenReference<T>)
  | Union(FrozenUnion<T>)
  | Erased(FrozenErased<T>)
```

`FrozenPrimitive` is sealed over unit, bool, char, signed and unsigned integer
families, binary floating-point families, exact decimal, string, null, and
undefined. Arrays, `Option`, `Result`, `Future`, collections, and user generic
types are uniform nominal applications. Struct, enum, newtype, and opaque
structure is exposed through `NominalShape`.

Transparent aliases normalize away. Structural object intersections normalize
to records; trait intersections normalize into erased-domain bounds. Explicit
`any` and `dyn Trait` are erased domains, while compiler-internal `Any` cannot
freeze. Traits use `TraitRef`; proven implementations use branch-scoped
`ImplRef<T, Trait>` evidence.

## Generated Code

Static generated structure uses ordinary Shape syntax in a context that expects
an expression, pattern, statement, body, item, or module. Computed structure
uses typed builders and semantic edit cursors. Shape has no public quote/splice
sublanguage.

Ordinary comptime data crosses into generated runtime code only through the
closed `ConstLift` capability. References, resources, functions, secrets,
provider grants, compiler capabilities, and runtime handles cannot be lifted.

Generated bodies and templates carry complete environments:

```shape
CheckedBody<Sig, Captures>
CheckedTemplate<Sig, Captures>
CaptureDescriptor<Sig, I, T, Mode>
```

Generated closures declare every runtime capture explicitly as move, shared
borrow, or exclusive borrow. Runtime bindings are referenced through
compiler-issued descriptors, never names. Existing-body edits begin with the
whole current capture set; capture changes, body changes, environment layout,
ownership/drop plans, and generated references commit atomically.

Installation checks lifetimes, suspension, `Send`, effects, cleanup, `Drop`,
and `AsyncDrop` before publishing anything. There is no partial generated body
or runtime fallback for an invalid environment.

## Annotation Contract

Annotation definitions expose typed clauses for exact target/phase pairs.
Applying an annotation where no clause exists is a compile error. Target
support is derived from those handlers rather than maintained in a second
registry.

Each application is an
`AppliedAnnotation<Identity, ExactTarget, ConstArgs, Multiplicity>` whose
identity is resolved, target is exact, arguments are a typed ConstLift product,
and multiplicity is proven. There is no universal target, target-kind string,
optional-field target object, homogeneous argument array, or string-backed
annotation metadata.

One annotation-elaboration module owns two ordered queries. Contract
elaboration consumes the resolved typed base target and canonical ordered
applications, then contributes effective effects, outcomes, ownership,
lifecycle requirements, and generated headers before the final contract
freeze. After dependent checking, body/plan elaboration returns either
diagnostics or a closed `CheckedAnnotationPlan` plus the shared compiler/LSP
facts. This plan proves annotation composition but is not execution authority;
it lowers to ordinary typed Core/MIR, semantic optimization/inlining preserves
its proof provenance, and ADR-010 freezes the final composite before backend
lowering.

Runtime callable transformation uses an ordinary typed around-call core:

```shape
around<Sig>(
    args: ArgumentPack<Sig>,
    next: Next<Sig>,
) -> ReturnOf<Sig> ! Effects
```

`ArgumentPack<Sig>` is a real heterogeneous signature-indexed product.
`Next<Sig>` is an affine capability for the exact next-inner continuation.
Calling `next` proceeds; returning an exact result without calling it
short-circuits; code after the call is ordinary after behavior; per-invocation
state is lexical and typed. `before`/`after` sugar lowers through an explicit
typed success join so its after phase runs once for both inner completion and a
same-layer short-circuit, while a raw `around` early return has ordinary source
semantics. Neither form defines a spelling-recognized decision protocol.

Hook composition is total. Failure transforms receive explicit structured
runtime-failure and failed-attempt capabilities and can recover an exact result,
retry under replay authority and budget, or propagate. Cleanup and `AsyncDrop`
obligations remain structured and cannot be skipped by success, failure,
short-circuit, cancellation, suspension, or contained engine fault.

`@remote` is therefore ordinary stdlib composition, not compiler syntax or an
annotation-specific intrinsic path. It consumes `Next<Sig>` into a verified
`PortableContinuationArtifact<Sig>`, admits that artifact at an exact
placement to obtain single-attempt `AdmittedExecution<Sig, P>`, and dispatches
it with the signature-indexed argument pack through the single Remote Dispatch
contract. Its `Remote<P>`/`Suspend` effects and exhaustive remote outcomes enter
the effective callable contract before callers check. The compiler does not
know the `remote` annotation name or substitute marker calls.

Remote uncertainty is never collapsed into an ordinary error. A missing reply
retains inaccessible escrow and an affine `RecoveryObligation`; transparent
placement remains suspended/recovery-pending or fails only after a durable
obligation transfer. The recoverable surface exposes uncertainty explicitly,
and projects to `Result<R, RemoteError>` only when settlement or a restricted
contract proves that no obligation is lost. Providers may customize transport,
discovery, authentication, retries, placement, and host encoding, but verified
Shape artifacts, exact execution ABI identity, typed values, outcomes, and
execution certainty remain host/runtime-owned contracts.

## Tooling Contract

Compiler and LSP consume the same staged query graph, resolved annotation
identities, exact targets, canonical application order, selected-clause
proofs, prepared/effective contracts, typed arguments, final elaboration facts,
generated symbol identities, source maps, and diagnostics.
Completion and signature help are expansion-sink and hook-phase sensitive.
Hover exposes the stage, exact descriptor type, capture mode, and effects.
Navigation, references, and rename include generated symbols and
descriptor-identified captures. LSP does not maintain a second annotation
support table or reconstruct meaning from source headers.

Virtual expansion documents are deterministic read-only renderings of checked
IR, never parser input. Unsupported completion, navigation, provenance, or
diagnostics for an enabled comptime structure is a language completeness defect.

## Consequences

The existing string/source/JSON directive, universal-target, spelling
classifier, pseudo-tuple, marker-substitution, and annotation-specific backend
paths are migration scaffolding, not the target public architecture. They must
be replaced by vertical typed slices that each include compiler descriptors,
checked generation, VM/JIT parity, diagnostics, LSP behavior, and
positive/negative alias, rename, and homonym examples.

This is intentionally a breaking migration. Shape is pre-production, so no
compatibility layer is required for accidental comptime or annotation APIs.
Each newly enabled descriptor or fragment category must be complete enough that
partially implemented behavior is rejected at compile time rather than deferred
to a runtime error.
