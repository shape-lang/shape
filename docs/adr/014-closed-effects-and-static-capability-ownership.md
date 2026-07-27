# ADR-014: Closed Effects and Static Capability Ownership

## Status

Accepted (2026-07-27)

Clarifies ADR-010, ADR-011, and ADR-012, and composes with ADR-015.

Where earlier text classifies the teardown continuation, an admitted execution,
or a recovery obligation as affine or merely single-use, this ADR's linear
classification governs.

Proposed amendment 2026-07-27 (pending ratification): §8 fixes how effect rows
appear in function types and how effects flow through higher-order and generic
code. It extends §1's algebra without weakening it, under §8.3's
schema-versus-fact distinction: a generic declared contract may persist with
explicit effect-parameter binders, exactly as it persists with explicit type
binders, while closed row facts — subset evidence, effect proofs, serialized
artifact rows, admission checks — never contain an unbound parameter. The
amendment also classifies `DurableSupervisor` in §4 and cross-references
ADR-015 §10's fixed-product obligation batch.

## Context

Shape needs one representation for what verified code may do and one static
accounting model for authorities that must not be copied, leaked, or silently
dropped. Those are related proof problems, but they are not the same as host
authorization.

The existing `Permission` vocabulary mixes at least three categories:

- runtime operations such as filesystem, network, process, environment, time,
  randomness, and FFI use;
- scoped grants such as filesystem and network scopes;
- execution constraints such as virtual filesystems, determinism, capture, and
  resource limits.

A set that mixes these categories cannot truthfully serve as both an effect row
and execution authority. In particular, proving that a callable's effects are
a subset of a declared row must not itself grant filesystem, network, FFI, or
provider access. Compile-stage authority must also not leak into runtime merely
because two effects have similar names.

Ownership has a corresponding gap. Copy/non-copy classification and runtime
consumed flags cannot prove that recovery, admission, and teardown authorities
are settled on failure, cancellation, suspension, or contained engine faults.
Those obligations need structural, path-sensitive verification on the final
outcome-explicit MIR.

The closed catalogs, stage brands, authority split, and three ownership classes
become part of persistent contracts and proof artifacts. They are therefore
the hard-to-reverse decision owned by this ADR rather than an implementation
detail of annotations or one backend.

## Decision

### 1. Effects are closed, canonical, and stage-separated

The runtime effect algebra is:

```text
OperationalEffectId =
    FsRead
  | FsWrite
  | NetConnect
  | NetListen
  | Process
  | Env
  | Time
  | Random
  | Ffi

RuntimeEffectAtom =
    Operation(OperationalEffectId)
  | Suspend
  | Remote(ResolvedProviderIdentity)

ClosedEffectRow<Runtime, CatalogVersion> =
    CanonicalSet<RuntimeEffectAtom>

ComptimeEffectAtom =
    Operation(OperationalEffectId)

ClosedEffectRow<Comptime, CatalogVersion> =
    CanonicalSet<ComptimeEffectAtom>
```

The compile-stage evaluator has the separately branded compile-time row above.
It does not include `Suspend` or `Remote` in the first version. There is no
implicit conversion between stages even when both rows contain the same
operational identifier. A runtime declaration cannot authorize compile-stage
evaluation, and a compile-stage grant cannot become runtime authority.

Rows are sorted, deduplicated, hash-covered, and catalog-versioned. Unknown
atoms and unsupported catalog versions reject at artifact load or admission.
The first version has no open string effects, wildcard effects, row
variables, or effect-row polymorphism in checked or persisted row facts;
generic signatures may bind effect parameters that close at instantiation
per §8.3, which is not row polymorphism in this algebra.

Every foreign call edge contributes `Operation(Ffi)`, including normalized C,
Python, and TypeScript adapter calls.
`Remote(ResolvedProviderIdentity)` names the exact resolved provider identity.
It does not imply `NetConnect`: a local provider may need no network, while a
remote provider contract may contribute `NetConnect` or other operational
effects separately.

Runtime failure, cancellation, suspension state, and engine faults remain
evaluator outcomes. They are not encoded as permission names or failure-shaped
effect atoms.

### 2. Effects, grants, scopes, constraints, and admission are different facts

The current permission vocabulary is split as follows:

| Category | Members |
|---|---|
| operational effects | `FsRead`, `FsWrite`, `NetConnect`, `NetListen`, `Process`, `Env`, `Time`, `Random`, `Ffi` |
| scoped grants | `FsScoped`, `NetScoped` |
| execution constraints | `Vfs`, `Deterministic`, `Capture`, `MemLimited`, `TimeLimited`, `OutputLimited` |

The exact Rust enum may migrate incrementally, but checked contracts and
persistent artifacts must expose these as distinct typed facts:

```text
DeclaredEffects<Stage>
RequiredOperationalGrants
RequiredScopes
ExecutionConstraints
ProviderAdmissionRequirements
```

An effect row describes possible behavior. A host grant authorizes an
operation. A scope narrows that grant. A constraint restricts the evaluator. A
provider admission proves that an exact provider or receiver accepts an exact
artifact. None is interchangeable with another.

Compile-stage host grants remain typed `ComptimeCapability` or
`ComptimeSecretGrant` authorities. Every admitted external read also produces
the glossary's `TrackedBuildInput`; the effect row records the operation while
the tracked-input hash records the reproducible dependency. Neither fact
substitutes for the other.

### 3. Effect composition and proofs are mechanical

Normalization resolves intrinsic and provider identities before effect
checking. The join operation is canonical set union. A callee, annotation
transform, adapter, or intrinsic contributes its closed row to the enclosing
callable contract before the effective-contract freeze.

Parameterized `Remote(ResolvedProviderIdentity)` atoms use exact
resolved-provider equality in the first version. Provider families, subtyping,
and wildcard admission do not participate in effect subset checking.

Contract comparison and final executable verification are different proof
moments. Contract-row subset evidence is available before a body or final MIR
exists:

```text
prove_contract_subset(
    subset: ClosedEffectRow<S, V>,
    superset: ClosedEffectRow<S, V>,
    contract: EffectiveContractIdentity,
) -> ContractEffectSubsetEvidence<S, V>
```

`ContractEffectSubsetEvidence` binds the canonical row hashes, stage, catalog
version, resolved callable relationship, and effective-contract identity. It
does not claim that a not-yet-checked body has those actual effects.

After body elaboration and semantic optimization, executable verification is:

```text
verify_actual_effects(
    mir: FinalMirIdentity,
    actual: ClosedEffectRow<S, V>,
    frozen_contract: EffectiveContractIdentity,
) -> VerifiedEffectProof<S, V>
```

`VerifiedEffectProof` binds final MIR identity, effective-contract identity,
the actual and allowed canonical row hashes, stage, and catalog version. Both
forms are evidence about behavior, not authority tokens. Admission separately
checks operational grants, scopes, constraints, and provider requirements.

Semantic optimization may erase implementation structure but must preserve
effect provenance. A post-freeze change to a call edge, provider identity, FFI
edge, suspension edge, or effect row invalidates `VerifiedEffectProof` and
reruns executable verification; it cannot be excused by earlier contract-row
evidence.

### 4. Ownership is a three-class structural property

Every value has one of three verifier classes:

| Class | Copy | Implicit discard | Path obligation |
|---|---|---|---|
| `Unrestricted` | allowed | allowed | none |
| `Affine` | forbidden | allowed | consumed at most once |
| `Linear` | forbidden | forbidden | consumed, returned, or transferred exactly once |

`Unrestricted < Affine < Linear`. Composite values inherit the maximum class of
their parts:

```text
class(product) = max(class(field)...)
class(enum) = max(class(payload)...)
class(ArgumentPack<Sig>) = max(Affine, class(slot)...)
```

An enum with any linear payload is linear and must be handled as a whole.
Exhaustive consuming match transfers the selected payload into exactly one
branch. Fixed products and enums may contain capabilities — ADR-015 §10's
`ObligationBatch<Branches>`, indexed by a statically known branch list, is
such a fixed product, not a dynamic collection. Dynamic collections of
affine or linear capabilities are rejected in the first version because
their element accounting is not yet structural.

The initial sealed capability classifications include:

- `Next<Sig>`: affine;
- `RecoveryObligation`: linear;
- `AdmittedExecution<Sig, P>`: linear;
- ADR-010's teardown continuation: linear;
- `DurableSupervisor` (amendment 2026-07-27): unrestricted. Construction
  and the acceptance surface are sealed (ADR-015 §6), but possession of a
  supervisor is not ownership of any obligation — duplicating or capturing
  the handle duplicates no authority over a transfer, so ordinary copy and
  closure capture apply.

`AdmittedExecution` is consumed by dispatch or explicit abort.
`RecoveryObligation` is consumed by settlement or by a proven transfer to a
durable supervisor. A teardown continuation is consumed by teardown or
transferred across an admitted suspension path. A runtime `consumed` flag may
remain as a debug assertion, never as the proof.

ADR-015 owns retry and durable-recovery sequencing:
`FailureIntent → RetryIntent → cleanup → Retry Commit`, including the linear
journal-backed recovery obligation, persistent budget/deadline, and exact-once
durable settlement. This ADR owns the structural classes; ADR-015 owns the
temporal and durable transitions that lawfully consume or transfer them.

### 5. Capability construction and consumption use resolved identity

Only compiler-sealed constructors and consumers identified by resolved
`IntrinsicId` may mint, split, settle, transfer, or destroy a capability.
Spelling, source shape, traits, casts, reflection, serialization hooks, and
user-defined homonyms confer no privilege.

Absent an explicit sealed transfer operation, affine and linear values cannot
be cloned, deep-cloned, serialized, formatted, captured by a duplicable
closure, stored in a shared cell or module global, or moved into a detached
task. Returning a capability is an explicit transfer to the caller. Passing it
to an ordinary function transfers it only when that function's resolved
ownership contract consumes the exact value.

### 6. The final outcome-explicit MIR must carry a capability proof

The capability verifier runs after annotation elaboration and semantic
optimization, on the same outcome-explicit MIR that ADR-010 freezes. It is
path-sensitive across products, enums, branches, calls, cleanup regions,
transfers, and suspension carriers.

For every reachable semantic edge:

```text
Completed
Failed
Cancelled
Suspended
ContainedFault
```

the verifier proves that each affine value is consumed at most once and each
linear value is consumed, returned, transferred, or evaluator-torn-down exactly
once. A consuming enum match transfers only the selected payload. Joins must
agree on the ownership state of every surviving value. Unreachable paths do not
excuse a reachable leak, and a generic capability proof is instantiated and
rechecked for each executable specialization.

Successful verification returns:

```text
CapabilityProof {
    mir_identity,
    outcome_graph_identity,
    capability_catalog_version,
    ownership_state_hash,
}
```

ADR-010's `RegionTeardownFreezeBoundary` requires this proof alongside
`VerifiedEffectProof` and its borrow, transfer, and teardown evidence. Any
later semantic change invalidates the affected proof. The interpreter, JIT,
remote dispatcher, and snapshot/resume path consume the same verified MIR and
cannot substitute backend-local capability policy.

### 7. Diagnostics and tooling project the same facts

Compiler and LSP facts distinguish declared effects, actual normalized effects,
missing host grants, scope mismatches, execution-constraint violations,
provider-admission failures, and capability-flow failures. Diagnostics name the
exact resolved operation or capability, its creation/transfer site, and the
outcome edge on which proof failed.

Tooling does not infer effects from source spellings or reconstruct ownership
from types after lowering. It projects the same closed rows, ownership classes,
and proof failures used by compilation.

### 8. Function types carry effect rows; higher-order flow closes at instantiation (proposed amendment 2026-07-27)

Shape monomorphizes generic execution and closes the one non-monomorphized
dispatch path — trait objects — at the trait declaration. That existing
execution model, not a new algebra, is the language's effect polymorphism.

#### 8.1 The row is a component of the function type

Every function type — a named callable used as a value, a closure, a foreign
callable, a function-typed parameter, field, or return — carries a closed
effect row of its stage as part of the type:

```text
fn(P...) -> R ! E        where E: ClosedEffectRow<Stage, CatalogVersion>
```

The row participates in type identity, unification, and subtyping. Two
function types that differ only in row are different types. Row subsumption
is subset: a value of row `E1` is usable where row `E2` is expected iff
`E1 ⊆ E2`. Rows are covariant exactly where returns are covariant and compose
with the existing contravariant-parameter judgment — for nested function
types, `fn(fn() -> T ! E1) -> U ! E2` accepts an argument typed
`fn(fn() -> T ! E3) -> U ! E4` iff `E1 ⊆ E3` and `E4 ⊆ E2`, the variance
flipping at each parameter nesting exactly as it does for types. Subsumption
is a compile-time fact with zero runtime representation change.

Where two function types must merge rather than one checking against the
other — reassignment of a mutable binding, collection-element joining,
branch joining — rows merge by §3's canonical union (the least upper bound),
not by unification failure. `var f = pure_cb; f = logging_cb;` types `f` at
the union row; it is never a row-shaped type error in unannotated code.

The call-through-value rule is a property of the row algebra, not of
inference alone: every call through a function-typed value contributes that
value's type row wherever effects are computed — during inference, during
contract checking, and in `verify_actual_effects`' derivation from final
MIR. A declared boundary row is discharged against its callback parameters'
declared rows by this same rule; no flow analysis narrows a function-typed
value below its type's row.

Trait methods declare their rows in the trait signature; every
implementation's actual row must be a subset of the declared row, checked at
impl elaboration. A call through a trait object — the one dispatch path
monomorphization does not close — joins the trait's declared row. In v1,
object-safe trait methods may not bind effect parameters; a trait object
over a trait with effect-parametric methods is rejected with a structured
diagnostic.

#### 8.2 Rows are declared at boundaries and inferred inside them

A declared row is required wherever ADR-011/ADR-012 already require a
declared contract: exported callables, annotated callables (the row is part
of `NormalizedDeclaredContract<Sig>`), foreign declarations (which contribute
`Ffi` per §1), function-typed members of public types, and any function value
crossing a module-public surface. `! {}` declares purity explicitly; an
omitted row at a boundary is a structured diagnostic with a
machine-applicable materialization fix (ADR-017), never a silent default.

Unexported, unannotated callables and local closures receive inferred rows,
mirroring ADR-011's allowance for inferred results. Inference is the join of
§3 under the call-through-value rule above: the union of the body's
operation edges, the rows of statically resolved callees, and the type rows
of invoked function values. Row inference over recursive and mutually
recursive callables is a monotone least-fixpoint domain of the discovery
worklist (bottom `{}`, join = canonical union, finite lattice): the
fixpoint converges and an intermediate under-approximation is never
observable outside the engine. A recursive row-inference cycle is
convergence work, not a semantic-cycle diagnostic under ADR-013's
acceptance item 4.

#### 8.3 Effect parameters bind at instantiation and never persist open

A generic callable signature may bind effect parameters, mirrored on
`TypeParamRef` as explicit `EffectParamRef` binders, and reference them in
function-typed parameters and in its own row:

```shape
fn map<T, U, effect F>(self, f: fn(T) -> U ! F) -> Array<U> ! F
```

This does not amend §1's ban: an effect parameter is a binder in a generic
signature, not an open row in the checked algebra. Every instantiation
substitutes closed rows before checking; ADR-010 §13 already requires exactly
this ("open rows must close before materialization").

The persistence rule is precise about what carries binders and what does
not. An effect-parameterized declared contract persists and freezes **with**
its explicit binders, exactly as generic type contracts persist with
explicit `TypeParamRef` binders — the generic definition's contract is a
schema, and callers of the generic definition check against that schema per
ADR-012 §1. What may never contain an unbound effect parameter is a closed
row **fact**: `ContractEffectSubsetEvidence` and `VerifiedEffectProof` are
minted per instantiation over fully substituted rows, and effect proofs
bind each executable specialization's own `FinalMirIdentity`, exactly as §6
already instantiates and rechecks the capability proof per specialization.
An unbound effect parameter inside a closed row fact, serialized artifact
row, or admission check is a compile error, not a wildcard.

Effect parameters instantiate only to closed rows of the same stage and
catalog version. They introduce no subtyping between stages, no provider
families, and no row arithmetic beyond §3's canonical union.

Surface syntax (user-ratified 2026-07-27): the `!` clause after the return
type — `fn f(p: T) -> R ! {FsRead}`, explicit purity `! {}`, binders
spelled `effect F` — as used throughout this ADR's and ADR-012's examples.
The coexistence with the `!!` error-context operator was considered and
accepted; the grammar slot is the return-type rule's tail.

#### 8.4 Grounding (2026-07-27)

The implementation starts from a known state:

- `Type::Function { params, returns }`
  (`crates/shape-runtime/src/type_system/types/core.rs:26`) carries no effect
  component, `TypeScheme` (`core.rs:33`) has no effect quantifier, the
  grammar has no effect-clause slot (`shape.pest:538`), and no effect or
  purity tracking exists anywhere in `type_system/`. The row extends the
  existing structural-equality, unification, and subtyping judgments
  (`constraints.rs:274`, `:439`, `:1704`) and the bidirectional
  closure-inference entry points (`inference/bidirectional.rs:535`).
- `PermissionSet` is already a clean canonical lattice
  (`BTreeSet`-backed `pure`/`union`/`is_subset`/`difference`,
  `crates/shape-abi-v1/src/lib.rs:1222`) and is already hash-covered as
  sorted names; the operational-effect alphabet of §1 reuses that shape
  rather than inventing a parallel one. The blob-level transitive fixpoint
  (`compiler_impl_initialization.rs:581`) computes the same closure the row
  needs, but post-typing over bytecode; the typed row becomes the single
  source of truth and the blob derivation must agree with it or be derived
  from it — two independent authorities is not an accepted end state.
- Two soundness caveats the row closes rather than inherits: current
  derivation is a string-keyed table over ~6 stdlib module paths with a
  `pure()` catch-all (`capability_tags.rs:25`) — "absent from the table" is
  not "proven pure" — and closures have no permission identity at all
  (closure bodies stamp the enclosing function's blob), while §8.1 requires
  per-closure-value rows. Both are why the row is checked in the type
  system, not retrofitted onto the blob table.
- The per-blob `required_permissions` and linker union remain the
  host-authority mechanism per §2; they are not the effect row and are not
  replaced by it.

## Consequences

- Effect checking is deterministic, persistable, and independent of the host
  authority that happens to run the program.
- Compile-stage and runtime authority cannot leak across stages.
- FFI behavior is visible in every foreign callable contract.
- Recovery, admission, and teardown obligations cannot disappear on exceptional
  evaluator paths or hide inside a permissive container.
- Some currently accepted code will reject until it explicitly settles or
  transfers capabilities.
- Dynamic collections of capabilities, open effect rows, and user-defined
  linear types require later designs rather than weakening the first
  verifier. (§8.3's instantiation-bound effect parameters and ADR-015 §10's
  fixed-product obligation batch are not instances of those open problems.)
- MIR must preserve effect, outcome, and ownership provenance through semantic
  optimization, increasing compiler implementation work but removing
  backend-local policy.
- (§8) Higher-order and generic code is effect-transparent without open rows:
  public HOFs declare effect parameters, every instantiation checks closed
  rows, and only boundary declarations are ever written by hand.

## Rejected alternatives

- **Use one `PermissionSet` for effects and authority.** A behavioral subset
  proof would accidentally become a grant, and scopes and constraints would
  remain semantically ambiguous.
- **Represent effects as open strings or accept unknown atoms.** Artifacts,
  admission, and cross-backend checks would become non-exhaustive and
  version-dependent.
- **Use one effect row across compile time and runtime.** This would allow
  compile-stage authority to authorize runtime behavior or vice versa.
- **Treat exact-once obligations as affine.** Silent discard is precisely the
  recovery, admission, and teardown bug class these capabilities exist to
  prevent.
- **Rely on runtime consumed flags.** They detect only executed double use and
  cannot prove settlement on every failure, cancellation, suspension, or fault
  path.
- **Classify only the outer nominal type.** A linear value could then disappear
  inside a product, enum, argument pack, or generic wrapper.
- **Encode failures as effects.** Effects describe possible operations;
  evaluator outcomes carry execution and cleanup state and have different
  composition rules.
- **Let each backend enforce ownership independently.** Divergent VM, JIT,
  remote, and resume semantics would defeat the proof boundary.

## Related decisions

- ADR-006: Value and Memory Model
- ADR-009: Strictly Typed Comptime and Annotations
- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms
- ADR-015: Recovery Episodes and Durable Obligation Journal
