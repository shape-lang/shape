# ADR-013: Incremental Semantic Queries and Tracked Comptime

## Status

Accepted (2026-07-27)

Clarifies the execution architecture of ADR-009, ADR-011, and ADR-012.
ADR-011 remains the authority for semantic identity and elaboration order. This
ADR records the hard-to-reverse choice of incremental engine, the seam between
semantic analysis and bytecode emission, and the only permitted way for
comptime to observe external state.

## Context

The compiler and LSP need one semantic query graph. Recomputing whole compiler
sessions for tooling is expensive, while maintaining parallel resolution,
annotation, intrinsic, and generated-symbol logic lets the two consumers
disagree. Fine-grained invalidation is therefore part of correctness as well as
latency: a cache entry is sound only when every semantic dependency is tracked.

The existing bytecode compiler is also a large mutable driver. It owns an output
program, current-function and expression state, scope stacks, counters, caches,
transactional journals, generated-symbol tables, and backend-specific emission
state. Moving that object into an incremental database would confuse semantic
facts with a stateful consumer and make query purity depend on mutation order.

Comptime presents the corresponding external-input problem. A default runtime
VM can observe clock, randomness, environment, filesystem, processes, network,
target configuration, or secrets without teaching an incremental query which
input changed. Memoizing such execution would make cached expansion identities
and release artifacts unsound.

## Decision

### 1. Three modules meet at one semantic seam

The architecture has three roles:

1. `SemanticDb` owns immutable source and configuration inputs, local interned
   handles, and deterministic semantic query results.
2. `DiscoveryEngine` owns Shape's explicit bounded declaration and contract
   fixed point and returns an immutable `DiscoverySnapshot`.
3. `BytecodeEmitter` is an ephemeral mutable consumer of checked semantic facts
   and typed Core/MIR.

The conceptual external interface of the semantic module is small:

```text
callable_facts(DefinitionIdentity) -> CallableFacts
discovery_snapshot(UnitIdentity) -> DiscoverySnapshot
annotation_facts(TargetIdentity) -> AnnotationFacts
```

The interface includes deterministic diagnostics and source provenance.
Compiler and LSP may format or project the returned facts; they do not read
semantic database tables, discovery ledgers, or bytecode-compiler internals.
Additional implementation queries may exist behind this interface without
becoming new consumer seams.

`BytecodeCompiler`, bytecode programs, current-function stacks, mutable
expression registers, install journals, backend caches, and VM/JIT state are
never Salsa inputs, tracked values, or query-owned mutable state. Moving a fact
out of `BytecodeCompiler` means publishing an immutable semantic query result,
not storing the compiler in the database.

### 2. Shape adopts Salsa for incremental storage and invalidation

The semantic database uses the Salsa crate rather than a new in-house
incremental engine. The implementation ticket must pin one exact compatible
Salsa release, feature set, and supported Rust version in the workspace and
lockfile; a floating dependency is noncompliant.

Salsa owns:

- database revisions and dependency recording;
- memo storage, red-green validation, and early cutoff;
- local interning and concurrent read coordination;
- invalidation after explicit tracked inputs change.

Salsa does not own:

- Shape's portable `DefinitionIdentity`, `ApplicationIdentity`,
  `ExpansionIdentity`, intrinsic identity, or artifact identity;
- declaration/contract lattice joins, convergence bounds, or language-level
  cycle classification;
- expansion provenance or Shape diagnostics;
- comptime capability policy and external-input normalization;
- bytecode emission, backend state, or lifecycle admission.

Salsa ids are database-local acceleration handles. They never enter serialized
artifacts, cross-process cache keys, snapshots, source maps, diagnostics, or
portable equality. Shape canonical identities remain ordinary hashable query
inputs and outputs.

Salsa's default query-cycle behavior and optional fixed-point support do not
replace `DiscoveryEngine`. The discovery engine runs its explicit monotone
worklist inside a tracked query, applies Shape's bounds and joins, and returns
either one immutable snapshot or structured diagnostics with complete
provenance.

### 3. Discovery is one query with separate typed domains

Each checked expansion produces an internal:

```text
DiscoveryDelta {
    headers,
    contract_contributions,
    tracked_dependencies,
    provenance,
}
```

Only `DiscoveryEngine` can construct or join these deltas. Header and contract
contribution types remain distinct even though one scheduler owns their
dependency graph and termination. Re-publication of equal content under the
same identity is idempotent; different content under that identity is a
mutation/cycle error.

Run-once accounting uses ADR-011's full six-component `ExpansionIdentity`.
The convergence state covers published identities and content, pending
expansions, and dependency edges. Worklist order, display names, spans, and
database-local ids do not affect the result.

Compiler, LSP, artifacts, and later elaboration stages consume the resulting
`DiscoverySnapshot`. A body-stage attempt to add a public header, effect,
outcome, ownership rule, lifecycle requirement, or signature contribution is a
contract-cycle error and cannot trigger a hidden repair pass.

### 4. Comptime is pure by default and enters through one host

Every comptime execution enters through one deep `ComptimeHost` module:

```text
evaluate(
    CheckedComptimeRequest,
    ComptimeGrantSet,
) -> TrackedComptimeResult | ComptimeDiagnostic
```

An empty or omitted grant set is pure. No comptime entry point constructs an
allow-all VM or calls host clock, randomness, environment, filesystem,
processes, network, target inspection, or secret APIs directly.

External interaction occurs only through a registered typed provider. Each
provider operation records:

- provider identity and version;
- normalized typed request;
- toolchain and target configuration;
- public content digest;
- provenance and dependency edges;
- resource limits;
- freshness, offline-lock, or reproducible-snapshot evidence.

The canonical sorted tracked-input identities and public digests are the sole
external-input contribution to `ExpansionIdentity.TrackedDependenciesHash`.
Changing one tracked input invalidates its dependent comptime and discovery
queries; changing an unrelated input does not.

A secret grant is opaque provider authority. Secret bytes never become
ordinary comptime values and never enter query keys, logs, diagnostics, virtual
documents, dependency hashes, expansion output, artifacts, or snapshots.
Public provider results may be hashed. A release artifact rejects live external
data that lacks reproducible snapshot or lock evidence.

No query that executes comptime may be memoized until this host and tracked
input envelope is installed.

### 5. Query migration uses named non-vacuous tracers

The infrastructure migration is deliberately incremental:

- The first semantic-database slice publishes only the resolved definition
  identity, base contract, and diagnostics for
  `fn add(a: int, b: int) -> int` and one call site. Compiler and LSP consume
  that same query. Annotation facts, generated symbols, method-table migration,
  and bytecode state are outside this slice.
- The first intrinsic slice migrates the active declared
  `__native_ptr_size` primitive through ordinary resolution and the
  `IntrinsicCatalog`. The deleted `__into_*` and `__try_into_*` families are not
  valid tracers.
- The first exact-annotation slice uses repeatable
  `@tag(label: string)` to prove typed arguments, stable source and generated
  application identity, multiplicity, and canonical order.
- The first contract slice uses `@requires_env(name: string)` to contribute a
  closed Env effect before caller and LSP checking.

Each tracer includes a positive import-alias case, a negative same-spelled
homonym, direct inspection of the resolved semantic fact, and an observable
downstream consumer. An output-only assertion or diagnostic-string assertion
does not prove the producer or query seam is load-bearing.

The callable tracer also measures query execution across comment-only,
body-only, signature, import-retarget, alias, and local-shadow edits. The test
declares the queries expected to rerun and those expected to remain green.

### 6. Intrinsics migrate through one catalog, then in families

`IntrinsicCatalog` binds trusted, versioned manifest entries to resolved
declaration identities. It validates the declaration's exact type, effects,
ownership, lifecycle, and stage contract before minting:

```text
ResolvedIntrinsic {
    definition,
    intrinsic_identity,
    contract,
    access_policy,
}
```

Call resolution selects `definition` first. Catalog projection and access
checking follow. Backend dispatch consumes `intrinsic_identity`, never terminal
spelling, source-module text, an unspellable prefix, or an ambient internal
mode. Identity-keyed access restriction may remain after selection.

After the first tracer, a separate program ticket inventories and migrates all
live intrinsic families in shrink-only waves. Each wave reduces mechanical
ratchet maxima for raw-name selectors and ambient selection. Full catalog
rollout blocks completion of ADR-011, but does not block deletion of universal
annotation descriptors unless a specific direct intrinsic consumer is named.

### 7. Acceptance is behavioral and incremental

The architecture is not accepted merely because Salsa types, canonical hashes,
or new interfaces exist. The implementation must prove:

1. compiler diagnostics and LSP projections carry identical semantic fact
   content identities across independently created database sessions;
2. aliasing preserves identity while a local homonym receives no privileged
   semantics;
3. edit-sequence query traces demonstrate targeted recomputation and early
   cutoff;
4. a semantic cycle returns a Shape diagnostic rather than a Salsa panic;
5. randomized discovery scheduling produces an identical snapshot;
6. inserting an unrelated generated application preserves stable-path sibling
   identities;
7. a changed tracked provider digest invalidates the exact dependent expansion
   while an unrelated change does not;
8. ungranted external comptime operations fail closed and name the attempted
   operation;
9. canary secrets are absent from query dumps, diagnostics, virtual documents,
   artifacts, and snapshots;
10. no `BytecodeCompiler`, backend program, or mutable emission state is stored
    in the semantic database;
11. artifacts round-trip portable semantic and intrinsic identities across a
    fresh process;
12. forbidden-pattern ratchets shrink with each migrated intrinsic or tooling
    path and final deletion removes the old authority.

Performance claims require measured edit latency, query re-execution, and
memory baselines on a representative workspace. Adopting Salsa alone is not a
performance result.

## Considered Options

### Build an incremental engine in-house

Rejected. Shape needs to own its semantic domains and diagnostics, not revision
storage, dependency recording, red-green validation, and concurrent memo
coordination. Reimplementing those mechanics would add risk without adding
language value.

### Move all `BytecodeCompiler` state into Salsa

Rejected. The bytecode compiler is a mutable consumer containing emission and
transaction state. Treating it as semantic data would make query results depend
on mutation order, create an enormous shallow interface, and keep tooling
coupled to compiler internals.

### Let Salsa query cycles implement declaration discovery

Rejected. Shape requires explicit finite joins, two typed publication domains,
language-specific bounds, and provenance-rich cycle diagnostics. Those are
semantic rules, not generic storage behavior.

### Identify applications by source span or generated emission index

Rejected. Whitespace and unrelated sibling insertions would invalidate stable
facts, references, and cache entries. ADR-011's same-triple source ordinal and
explicit generated structural path retain only semantically necessary order.

### Permit untracked host access and mark comptime queries volatile

Rejected. Volatility avoids some stale cache hits but does not provide
reproducible artifacts, dependency provenance, LSP consistency, secret
discipline, or targeted invalidation.

### Block annotation-descriptor deletion on every intrinsic family

Rejected. Full intrinsic migration is an ADR-011 completion gate, but unrelated
math, random, native, or collection families do not belong on the annotation
deletion critical path.

## Consequences

- The first incremental slice is narrow in semantic scope but establishes the
  production database and query discipline used by later slices.
- Semantic queries must be deterministic over explicit inputs; ambient state
  becomes either a tracked provider input or a structured error.
- Compiler and LSP share facts without sharing mutable compiler sessions.
- Discovery and comptime gain explicit invalidation and provenance tests in
  addition to ordinary result tests.
- Intrinsic migration becomes a catalog-and-family program with measurable
  deletion progress rather than one unbounded prerequisite for annotations.
- The chosen Salsa release becomes a workspace dependency with an explicit
  compatibility and upgrade decision.

## Implementation Choices Left Open

The initial implementation ticket must decide and record the exact Salsa
release/features, the physical crate location of `SemanticDb`, cancellation and
snapshot ownership, and initial query-memory budgets. Cross-session persistent
memo storage is not required by this ADR. These choices may not weaken the
semantic identities, module seam, tracked-input rules, or acceptance evidence
above.

## Related Decisions

- ADR-009: Strictly Typed Comptime and Annotations
- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms
