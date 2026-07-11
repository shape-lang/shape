# ADR-009: Strictly Typed Comptime and Annotations

## Status

Accepted (2026-07-11)

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

Compile-stage clauses consume frozen target descriptors and return typed atomic
rewrite plans. Runtime clauses produce ordinary Shape hook templates whose
signature is indexed by the target callable and lifecycle phase. Invocation
parameters, results, failures, retry state, cleanup obligations, and other
runtime values enter only through those exact hook inputs; hooks have no ambient
capture.

Hook composition is total. The compiler constructs one complete lifecycle plan
and rejects missing, conflicting, or effect-incompatible paths. Runtime failure
transforms can recover a valid result, retry, choose another placement, or
propagate a typed failure according to the plan. Cleanup and `AsyncDrop`
obligations remain structured and cannot be skipped by success, failure,
return, cancellation, or panic-like abort.

`@remote` is therefore a stdlib annotation, not compiler syntax. Its comptime
specialization validates and lowers a typed placement/policy into calls to
privileged distributed intrinsics. Runtime network and peer behavior remains
ordinary effectful execution. Providers may customize transport, discovery,
authentication, retries, placement, and host encoding, but verified Shape
artifacts, exact execution ABI identity, typed values, outcomes, and execution
certainty remain host/runtime-owned contracts.

## Tooling Contract

Compiler and LSP consume the same staged query graph, typed descriptors,
generated symbol identities, source maps, and diagnostics. Completion and
signature help are expansion-sink and hook-phase sensitive. Hover exposes the
stage, exact descriptor type, capture mode, and effects. Navigation, references,
and rename include generated symbols and descriptor-identified captures.

Virtual expansion documents are deterministic read-only renderings of checked
IR, never parser input. Unsupported completion, navigation, provenance, or
diagnostics for an enabled comptime structure is a language completeness defect.

## Consequences

The existing string/source/JSON directive paths are migration scaffolding, not
the target public architecture. They must be replaced by vertical typed slices
that each include compiler descriptors, checked generation, VM/JIT parity,
diagnostics, LSP behavior, and positive/negative examples.

This is intentionally a breaking migration. Shape is pre-production, so no
compatibility layer is required for accidental comptime or annotation APIs.
Each newly enabled descriptor or fragment category must be complete enough that
partially implemented behavior is rejected at compile time rather than deferred
to a runtime error.
