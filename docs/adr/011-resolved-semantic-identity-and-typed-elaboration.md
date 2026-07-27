# ADR-011: Resolved Semantic Identity and Typed Elaboration

## Status

Accepted (2026-07-25)

Clarifies ADR-003, ADR-004, ADR-006, and ADR-009. This ADR supersedes
implementation decisions that assign language meaning from source spelling,
AST shape, display text, source location, declaration origin, or an ambient
"compiler internal" mode.

ADR-013 fixes the incremental-query and tracked-comptime architecture used to
implement this decision. This ADR remains the authority for semantic identity,
elaboration order, and the facts every consumer must observe; ADR-013 defines
how those facts are computed, invalidated, and shared.

## Context

Shape's accepted architecture already says that types, symbols, generated
fragments, annotations, and execution artifacts are semantic and strictly
typed. Some implementation paths nevertheless recognize a privileged source
name, inspect a particular parsed expression, carry a rendered name through an
untyped structure, or replace a user-facing value with a different
compiler-internal representation before ordinary resolution and type checking.

Those paths can be locally guarded and still be architecturally false:

- an alias may lose behavior while a same-spelled local declaration gains it;
- tooling and compilation may resolve the same source differently;
- a surface type can claim to exist even though no value of that type is ever
  resolved, checked, or lowered;
- VM and JIT can acquire different semantic exception tables;
- later phases must reconstruct facts that earlier phases already knew;
- performance pressure can become a reason to bypass language semantics.

A great typed language needs a precise answer to two questions:

1. What semantic definition does this use refer to?
2. At what typed boundary may that definition affect the program?

## Decision

### 1. Every definition and use has canonical semantic identity

After resolution, every declaration, use, callable, member, annotation,
intrinsic, generated symbol, and exact target carries a canonical resolved
definition identity. Source names are lookup syntax and presentation data; they
are not semantic authority.

The identity must be:

- issued by the semantic database, never fabricated by a backend;
- preserved by an import alias;
- distinct for a shadowing or same-spelled declaration;
- independent of source span, allocation address, table position, and display
  spelling;
- stable wherever an artifact, cache key, source map, or cross-module query
  claims stability.

`MethodId`, `SymbolId`, exact member identities, intrinsic identities, and
artifact identities are projections of this rule for their respective
domains. A dense local index may be derived for execution, but it is not the
portable or semantic identity.

#### Application and expansion identity

`ApplicationIdentity` is a versioned, domain-separated canonical hash of the
resolved annotation identity, resolved target identity, typed
`ConstArgumentProduct` identity, and one occurrence identity. It is never a
source span, Salsa id, table ordinal, allocation identity, or display spelling.

A source-written occurrence uses its ordinal only among applications with the
same annotation, target, and typed arguments on that target, in canonical
lexical order. A generated occurrence uses a `GeneratingExpansionAnchor` and a
versioned `GeneratedApplicationPath` issued by the generator:

- the anchor contains the parent generator, application, target, stage, and
  typed arguments, but excludes dependency-content hashes;
- the path is a stable structural key within that expansion, not a global
  emission index;
- duplicate generated paths are a compile error;
- a generator may explicitly choose an indexed path when position is intended
  semantic content, but the compiler never silently substitutes one.

Consequently, inserting or reordering an unrelated generated sibling does not
change the identities of stable-path siblings.

`ExpansionIdentity` is the complete tuple:

```text
(
    GeneratorIdentity,
    ApplicationIdentity,
    TargetIdentity,
    ComptimeStage,
    TypedArgumentsHash,
    TrackedDependenciesHash,
)
```

Run-once expansion memoization and generated-symbol provenance use this full
identity. No shorthand that omits generator, target, stage, or typed arguments
is equivalent.

### 2. Semantic behavior begins only after its inputs resolve and type-check

Semantic elaboration is stage-indexed, not one whole-program pass after all
checking. The canonical dependency order is:

```text
source
  -> parsed syntax
  -> declaration discovery and generated-header fixed point
  -> resolved, typed base declarations and base callable contracts
  -> contract elaboration
  -> effective contract freeze
  -> dependent body, call-site, effect, and ownership checking
  -> body and plan elaboration
  -> checked annotation-free typed Core/MIR
  -> semantic optimization with ownership/outcome/effect provenance
  -> ADR-010 lifecycle and teardown freeze
  -> VM or native lowering and bound realization optimization
```

The "base callable contract" fixes the target identity, generics, parameters,
normalized result, receiver mode, and source-body facts needed to type-check a
contribution. An annotation target must explicitly declare its parameter and
result types; annotation selection and contract contributions never depend on
body inference. Unannotated callables may retain inferred results. Contract
elaboration then computes every externally visible annotation or
generated-signature contribution, including effects, failure outcomes,
ownership, lifecycle requirements, and generated declaration headers. The
effective contract freezes only after those contributions reach a fixed point.
Callers, implementations, overrides, artifacts, and LSP facts are checked
against that effective contract, never the pre-elaboration one.

Module/item expansion sinks publish resolved typed headers back into declaration
discovery. Signature and annotation-contract sinks complete before effective
contract freeze. Body and expression sinks run after the relevant effective
contract is available and must re-enter the local checker. A body/plan
elaboration may not discover a new public effect or other contract change; that
is a contract-cycle diagnostic and restarts no hidden repair pass.

Declaration and contract convergence use one bounded semantic discovery query.
One scheduler owns dependency edges, run-once accounting, convergence bounds,
and expansion provenance, while declaration headers and contract contributions
remain separate typed monotone domains. Each checked expansion returns an
immutable delta of headers, contract contributions, tracked dependencies, and
provenance. Publishing a new identity adds it; publishing the same identity
with equal content is idempotent; publishing it with different content is a
structured mutation or cycle error.

The convergence fingerprint covers the complete published header and contract
state, pending expansion identities, and dependency edges. It does not hash
display names or worklist order. Compiler, artifacts, and LSP consume the same
immutable discovery snapshot. Shape's discovery engine, not the incremental
storage library, owns the language-level joins, cycle classifications, bounds,
and full provenance diagnostics. ADR-013 defines that module seam.

Parsing may desugar syntax whose meaning is independent of resolution.
Anything that selects a definition, changes control flow, constructs a
capability, rewrites a call, or changes a type/effect/ownership contract must
wait for the relevant resolved typed inputs and run at the stage where its
result is still visible to every dependent consumer.

Semantic elaboration may create ordinary typed Core/MIR, checked declarations,
and proof-carrying plans. It may not reinterpret raw source, reparse rendered
text, classify a source expression by its spelling, or publish a partially
typed artifact for a later phase to repair.

### 3. Compiler primitives are explicit; compiler magic is forbidden

A compiler primitive is acceptable only when all of the following hold:

- it has one declared nominal identity selected by ordinary resolution;
- its complete type, effect, ownership, lifecycle, and stage contract is
  visible to the checker and tooling;
- aliases resolve to the same primitive and homonyms do not;
- the compiler validates its declaration against one canonical intrinsic
  catalog;
- VM, JIT, constant evaluation, artifacts, and LSP consume the same semantic
  fact;
- unsupported use fails at a typed boundary with a structured diagnostic.

Compiler magic is behavior selected from spelling, AST shape, source-file
origin, an unspellable name, a context boolean, a hidden value, or a surface
type that ordinary resolution and lowering never actually process. Compiler
magic is not permitted.

An unspellable generated name may provide hygiene. It does not provide
authority. An "internal builtin" flag may restrict access after a resolved
intrinsic is selected; it may not turn an ordinary same-spelled call into an
intrinsic.

One versioned `IntrinsicCatalog` binds trusted intrinsic manifest entries to
resolved declaration identities and validates each declaration's complete
contract. Successful validation mints a `ResolvedIntrinsic` containing the
definition identity, portable intrinsic identity, contract, and access policy.
Call resolution selects the definition first; only then may the catalog project
an intrinsic. Backend dispatch cannot classify the call's terminal name,
prefix, source module spelling, unspellable rendering, or ambient mode.

A dense backend opcode or local index may be derived from the portable
intrinsic identity, but is not semantic or serialized identity. Internal access
policy is evaluated after intrinsic selection. The first migration tracer and
every later intrinsic family require a positive alias test, a negative homonym
test, a resolved-fact assertion, and an observable downstream consumer; equal
output or a matching diagnostic string alone is not evidence that identity
selection is correct.

### 4. Proofs are minted from resolved typed facts

Proof-carrying values and plans use private constructors. Their issuing module
must derive them from resolved identity, exact types, effects, ownership,
lifecycle state, and complete control-flow evidence.

A proof cannot bless a result that bypassed the ordinary semantic pipeline.
In particular, a private token is not sufficient when the user-facing
construct it purports to validate was never represented as an ordinary typed
value or typed Core operation.

Serialized proof data is evidence, not execution authority. Admission or
revalidation rules from ADR-010 continue to apply.

### 5. Optimization may erase representation, never establish meaning

The checked typed program establishes meaning. Specialization, inlining, enum
unboxing, branch fusion, scalar replacement, and direct-call shaping may erase
abstractions only after their types and behavior are proven and while preserving
ownership, outcome, effect, and cleanup provenance. Semantic optimization and
inlining complete before ADR-010 freezes and certifies the final composite
region/teardown plan.

After that freeze, only realization-level substitutions already covered by the
`ExecutableTeardownRealizationBinding` may occur. Any transformation that
changes executable control flow, ownership, effects, outcomes, transfers, or
cleanup invalidates the certificate and must rerun the freeze; a backend cannot
silently optimize around it.

An optimization must not be required to make a surface construct well typed.
If a backend lacks a general Core/MIR operation, the compiler must implement
the general operation, choose an explicit compatible fallback, or reject that
backend. It must not recognize one library type or annotation and substitute a
private representation to obtain a benchmark result.

### 6. Compiler and tooling share one semantic query graph

The compiler and LSP use the same resolution results, typed semantic program,
elaboration facts, generated identities, diagnostics, and source maps. LSP
code may format or project those facts; it may not independently infer
intrinsic meaning, annotation support, target kinds, or generated-symbol
identity.

ADR-013 places that graph behind a `SemanticDb` seam. An explicit
`DiscoveryEngine` computes bounded declaration and contract snapshots, and a
mutable `BytecodeEmitter` consumes immutable semantic facts. Compiler-driver,
bytecode-emission, and backend state do not become semantic database state.
Database-local handles accelerate queries but never replace the portable
identities defined here.

### 7. Boundary contracts are truthfully typed

Every boundary records the complete semantic contract relevant at that
boundary:

- calls carry exact signature, ownership roles, effects, and evaluator
  outcomes;
- generated code carries complete captures and provenance;
- annotation applications carry exact target and typed configuration;
- remote execution carries placement, effects, execution certainty, and
  lifecycle ABI;
- cleanup and failure paths enter the same typed outcome graph as success.

A function may still return bare `R` when non-returning runtime failure is part
of its explicit evaluator outcome and effect contract. "Truthfully typed" does
not require converting every failure into `Result`; it requires that no
possible outcome be hidden from the semantic program, callable lifecycle ABI,
or host projection.

## Consequences

- Resolution and typed semantic IR become load-bearing compiler subsystems,
  not conveniences around AST rewriting.
- Intrinsics need a canonical identity catalog and declaration validation.
- Existing name switches, terminal-name checks, AST constructor recognizers,
  ambient builtin permissions, and display-string reconstruction are migration
  debt even when tests currently pass.
- General Core/MIR and backend support must be improved instead of creating
  feature-specific native shortcuts.
- Rename, alias, shadowing, generated-code, LSP, VM, and JIT tests become
  architecture tests rather than presentation tests.
- Application identity issuance becomes a semantic module used by discovery,
  ordering, generated provenance, artifacts, and tooling; callers do not mint
  occurrence identities themselves.
- Incremental computation and comptime execution must satisfy ADR-013 before
  their cached results can claim the semantic identities defined here.

## Mechanical enforcement

The migration must add ratchets proving:

1. aliasing a primitive or annotation preserves semantics;
2. a local homonym receives no privileged semantics;
3. renaming presentation text does not change semantic identity;
4. typed semantic nodes contain resolved definition identities before
   elaboration;
5. VM and JIT consume the same annotation-free typed Core/MIR;
6. compiler and LSP diagnostics originate from the same semantic query;
7. no semantic module dispatches on terminal names, unspellable renderings, or
   raw AST constructor shape;
8. optimized and unoptimized executions have identical semantic outcomes;
9. an elaborated effect/outcome is visible to caller checking, overrides,
   artifacts, and LSP before the effective contract freezes;
10. generated headers enter declaration discovery while body-only elaboration
    cannot mutate a frozen public contract;
11. whitespace edits and unrelated source or generated siblings preserve
    unaffected application identities;
12. generated application paths are unique and a duplicate rejects instead of
    falling back to discovery or emission order;
13. run-once expansion keys contain all six `ExpansionIdentity` components;
14. compiler and LSP project the same discovery and callable-fact content
    identities across independently created query sessions;
15. an intrinsic alias carries the same `ResolvedIntrinsic`, while a local
    homonym lowers as an ordinary call;
16. no database-local id, dense opcode, source span, or terminal spelling enters
    a portable identity or serialized semantic artifact.

Forbidden-pattern checks are maximum guards, not the proof by themselves.
Each migrated behavior also needs positive alias/rename tests and negative
homonym tests. Incremental and comptime gates additionally follow ADR-013.

## Rejected alternatives

- **Keep tightly guarded spelling recognizers.** Guards reduce accidental
  matches but do not make spelling a semantic identity.
- **Treat unspellable names as authority.** Hygiene prevents user collision;
  it does not prove what a definition means.
- **Let each backend recognize high-level features.** This duplicates semantics
  and makes parity an ongoing convention.
- **Use a hidden compiler representation for performance.** Optimization may
  derive that representation from typed Core/MIR; it may not replace the
  language's semantic value before typing.
- **Run all semantic elaboration after whole-program type checking.** Contract
  contributions would arrive too late for callers, overrides, effects,
  artifacts, and tooling. Elaboration is ordered by its expansion sink and
  contract dependencies.
- **Defer exact identity until code generation.** Annotation elaboration,
  effects, tooling, artifacts, and diagnostics already require it earlier.
- **Use a global source or emission position as application identity.**
  Unrelated insertions would invalidate stable generated facts, references, and
  cache entries. Source duplicates use a deliberately narrow same-triple
  ordinal; generated applications use explicit structural paths.
- **Treat an incremental database handle as semantic identity.** Such handles
  are local implementation details and cannot satisfy cross-process artifact,
  cache, or source-map stability.

## Related decisions

- ADR-003: Method Registry Single-Source
- ADR-004: Native C Interop In Language Core
- ADR-006: Value and Memory Model
- ADR-009: Strictly Typed Comptime and Annotations
- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-012: Verified Annotation Elaboration and Callable Transforms
- ADR-013: Incremental Semantic Queries and Tracked Comptime
