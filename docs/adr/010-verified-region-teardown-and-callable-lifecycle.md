# ADR-010: Verified Region Teardown and Callable Lifecycle

## Status

Accepted (2026-07-16)

Implementation is sequenced after ADR-009 stabilization. This ADR does not
amend ADR-009; it defines the lifecycle and execution substrate on which later
typed language and library features may rely.

ADR-011 and ADR-012 (2026-07-25) clarify the semantic-identity and two-stage
annotation-elaboration boundaries. Annotation contract contributions freeze
before dependent callers are checked. A later checked annotation plan is still
not execution authority; it must lower to ordinary typed executable MIR and
pass this ADR's late freeze and admission.

## Context

Strict typing is insufficient if an execution backend cannot prove how every
owned value leaves every ownership region. A semantic type or broad runtime kind
does not identify an exact release operation when one type can have multiple
physical carriers. Likewise, a call signature that describes only parameter
types cannot prove which side owns arguments, captures, returned values, borrows,
or cleanup after every evaluator outcome.

The interpreter, native tier, suspension machinery, deoptimization, snapshots,
and distributed execution must not reconstruct these facts independently. That
would permit backend-specific leaks, duplicate releases, premature release while
dependent work is still running, or execution on a placement that cannot honor
a rare failure-path finalizer. Hash integrity alone would show that all parties
received the same artifact, not that the artifact is sound.

The architecture must also preserve Shape's performance objective. Scalar and
fully static functions must not pay for a generic runtime cleanup ledger, slot
scan, type switch, plan interpreter, or per-call verifier. Direct calls and
non-escaping closures must remain eligible for allocation-free lowering,
inlining, and ordinary native optimization.

This is a general-purpose language facility. Science, simulation, data systems,
services, devices, and other libraries may need lifecycle behavior, but their
domain meanings do not belong in the language core. The core owns typed
ownership, control, verification, and exact carrier operations; libraries own
domain policy and behavior through typed capabilities.

## Decision

### 1. One carrier for materialized callables

Every callable that crosses a first-class value boundary uses one canonical
owned, reference-counted callable carrier. This includes named functions and
closures used as locals, parameters, returns, captures, join values, or
container elements. Origin does not select a second physical representation or
release rule.

A statically resolved direct call is not materialized. Direct named calls and
proven non-escaping closures may remain direct, inlined, or stack-resident and
need not allocate the first-class carrier.

### 2. Teardown is total over ownership regions and outcomes

The compiler issues one backend-neutral `RegionTeardownPlan` for a function. It
covers every control-flow edge that exits one or more ordered ownership regions,
including fallthrough, loop exits, return and propagation, failure, catch,
cancellation, suspension, deoptimization, and contained engine faults. A
`FrameTeardownPlan` is only the terminal-exit projection for the root region; it
is never a second source of authority.

Each potentially live frame value has one proven disposition:

- owned and discharged by an exact action;
- borrowed and therefore not released by this frame;
- transferred through the edge; or
- inline and requiring no carrier release.

The plan is not published if any possible owner, carrier, outcome edge, action,
or transfer is unproven. Liveness, zero bits, slot position, broad semantic kind,
or a currently uninitialized state may optimize an authorized operation, but
none of them supplies release authority.

Every terminal frame exit is teardown-total. Completion transfers the return
value and discharges the remaining owners. Runtime failure, cancellation, and
abandonment deoptimization discharge their remaining owners. Suspension and
resumptive deoptimization instead transfer the intact frame, owners, plan, and
dynamic teardown state to the continuation. Compile-time fallback creates no
frame. A contained engine fault runs only its proven fault-safe subset; an
uncontained process or hardware loss is outside this guarantee.

### 3. Semantic outcomes are explicit before plan freeze

Final executable MIR retains stable ownership-region, exit-site, handler, and
effect provenance. From this metadata the compiler derives an exhaustive
`SemanticOutcomeEdgeGraph`. The graph expresses semantic successors; it does
not prescribe whether a backend uses branches, landing pads, exception tables,
or another mechanism.

ADR-012 contract elaboration first contributes effects, outcomes, ownership,
and lifecycle requirements to the effective callable contract before dependent
checking. After that contract freezes, annotation body/plan elaboration may
prove that layers compose and emit their ordinary typed control-flow and
cleanup structure. Neither stage can mint a `VerifiedRegionTeardownPlan` or
authorize VM/JIT execution. The elaborated Core/MIR participates in the same
whole-function analysis and late freeze as source-written code.

There is one late `RegionTeardownFreezeBoundary`: after executable MIR shape and
all borrow, move, escape, storage, carrier, effect, and transfer proofs are
complete, but before bytecode or native lowering. At that boundary the compiler
freezes the region plan and its verification certificate once. The portable
`SemanticArtifactHash` covers that semantic artifact and stable semantic site
identities, not backend layout.

Each final VM or native form has a distinct `ExecutableRealizationHash` and a
hash-covered `ExecutableTeardownRealizationBinding` mapping every semantic site
to its final executable expansion, recognized fusion, or proven elision.
Admission independently checks that binding against the decoded executable.
Cleanup blocks, offsets, landing pads, and epilogues are derivative evidence,
never semantic authority; an unrecognized post-freeze semantic transform
requires a newly frozen semantic artifact.

### 4. Observable order follows ownership, not layout

Teardown uses reverse successful ownership-entry order. Inner regions settle
before their parents; within a region, obligations settle in reverse order of
successful initialization or adoption. A move preserves the obligation's
established order, while adoption establishes its order in the receiving
region. Parameters, temporaries, and deferred captures receive compiler-defined
semantic positions.

Slot numbers, capture layout, allocation address, or backend traversal order do
not define observable semantics. Only transitively proven unobservable memory
retirement may be reordered, batched, or elided.

### 5. Owning aggregates use sealed descriptors and specialized kernels

Every concrete owning aggregate specialization has one sealed, versioned,
hash-covered `AggregateLifecycleDescriptor`. It fixes carrier and backing-store
layout, occupancy, logical ownership sites, exact child descriptors, suspension
class, kernel ABI, target identities, and capability closure. Runtime reflection,
recursive type discovery, raw callbacks, and universal per-element ledgers
cannot create teardown authority.

Children tear down in reverse current `AggregateLogicalOwnershipOrder`.
Traversal derives from existing canonical container state, such as sequence
indices or declared schema order, with no lifecycle-only rank metadata or
allocation. Otherwise the aggregate exposes ordinary semantic index/order,
proves child teardown transitively unobservable, or refuses observable child
finalization. Extraction transfers and disarms; replacement settles then adopts;
a semantic reorder transfers and re-adopts in the new order. Reallocation,
compaction, columnar transposition, and SIMD layout preserve logical order.

After quiescence and structural retirement, an aggregate's own finalizer may
receive a typed shared or exclusive borrow-only `AggregateFinalizationView` of
its intact children. It cannot move, replace, steal, or rearm them. After that
finalizer completes or is recorded abandoned, the specialized
`AggregateTeardownKernel` settles children, backing storage, and the outer
carrier in order. Trivial children erase or use proven bulk/vectorized release;
fixed aggregates unroll; synchronous runtime-sized aggregates use one direct
specialized loop. Only a possibly suspending kernel carries compact armed state
and a monotonic cursor. Verification proves exhaustive, non-overlapping,
exactly-once child and backing-store coverage. Performance gates include
trivial and resource-bearing `Table`, `Batch`, and map traversal and allocation.

### 6. The teardown action algebra is sealed

The versioned core action algebra contains only lifecycle control semantics:

1. quiesce dependent scopes;
2. structurally retire each exiting owner;
3. optionally run its typed synchronous or awaited finalization target; and
4. perform its mandatory, exactly-once, exact carrier release.

Structural retirement revokes ordinary source access and arms the verified
teardown guard while preserving the carrier for finalization. `Finalization` is
optional source-visible behavior and may have effects, fail, or suspend under
its declared contract. `Carrier Release` is the distinct mandatory exactly-once
representation retirement or deallocation through the exact carrier-correct
operation. Finalization success never stands in for release, and finalization
failure never skips release or later obligations.

Annotation `around`, state, and cleanup constructs create ordinary typed
regions, owners, effects, and finalization obligations in this sealed algebra.
Annotations and remote providers cannot add teardown opcodes, side ledgers, or
backend-only cleanup meanings.

The invocation's primary `Completed`, `Failed`, or `Cancelled` outcome is fixed
before terminal teardown. Only the evaluator/host boundary materializes the
`Evaluation` envelope, which structurally carries typed, teardown-ordered
`CleanupEvidence`; its empty carrier allocates
nothing, and suspension transfers the unfinished builder affinely. Evidence-free
internal VM/JIT calls retain the ordinary direct-return ABI with no tag,
widening, copy, allocation, or cleanup branch. Finalization failures are evidence
and do not replace the primary outcome. A contained fault uses the
flat `EngineFaulted { primary, cleanup_evidence, fault, containment }` outcome,
preserving an optional already-frozen primary and committed evidence. `containment` is
`ReleasedExactly` only after proven quiescence and fault-safe exact release;
otherwise ownership is `Quarantined` under an outer containment owner.

Libraries provide typed finalization targets and evidence. Meanings such as
flush, close, simulation shutdown, or failure-path abandonment are library
behavior, not core action variants. Unsubscribe acknowledgements, device fences,
and GPU synchronization are quiescence evidence when they stop borrowers and
may be Finalization only after quiescence is independently complete. Plugins and
providers cannot add teardown opcodes, raw callbacks, or ownership semantics.

Ownership transfer is an edge disposition, not a cleanup action. A normal-path
protocol such as `MustSettle` remains a compile-time obligation to settle,
return, or transfer its owner; automatic finalization cannot fabricate evidence
that the protocol completed successfully. Its automatic fallback is permitted
only on the designated failure or cancellation outcomes.

Before the first finalizer that may suspend, an
`AwaitedTeardownSuspensionBarrier` seals ordinary frame execution, retires or
guards every remaining exiting owner whose storage can survive suspension, and
transfers sole authority over storage, cursor, evidence builder, lease, and
quiescence witnesses to one affine teardown continuation. Only declared
borrow-only finalizer views remain accessible. Fully synchronous plans erase
the barrier, continuation, phase state, and checks.

### 7. Static lowering is the ordinary execution model

A verified plan is a proof and lowering recipe, not a runtime list to interpret.
Backends lower it to direct carrier-specific actions and cost-selected inline or
shared epilogues. Empty, inline, borrowed, transferred, and statically disarmed
work disappears. Compact armed state and a resumable cursor exist only for
genuinely dynamic or suspending obligations.

Consequently, scalar functions and fully static kernels have zero lifecycle
dispatch overhead. There is no ordinary frame-wide slot scan, runtime type
switch, hash lookup during teardown, or per-invocation plan verification.

### 8. Published plans carry independently verifiable evidence

Every published `FunctionBlob` contains a versioned, immutable,
content-hash-covered `RegionTeardownArtifact`: the plan plus a compact teardown
verification certificate anchored by stable semantic site IDs. It contains
enough stable ownership-and-effect evidence to replay owner creation, movement,
borrowing, transfer, finalization, release, ordering, and all semantic outcomes
without serializing the original MIR. Backend epilogue layout is not part of
the semantic artifact.

Artifact integrity, publisher provenance, and semantic soundness are separate
checks. Before execution, the receiving runtime independently and fail-closedly
replays the semantic certificate and then the final realization binding against
the decoded executable and its exact action/carrier catalog. Success mints a
non-serializable `VerifiedRegionTeardownPlan` required by both interpreter and
native execution. Missing evidence, opaque effects, unsupported versions, or
mismatches require trusted recompilation or refusal; they never mean empty work.

Semantic admission may be cached only by `SemanticArtifactHash`, verifier
version, and the checked action/carrier-catalog subhash of the exact Execution
ABI. Final executable admission additionally keys on `ExecutableRealizationHash`;
neither check adds per-invocation cost.

### 9. Generics are parametric at definition and concrete at execution

A generic definition carries a checked but non-executable parametric teardown
contract. It describes region structure, ownership transfer, order, and required
capabilities under the definition's bounds. It never chooses an unresolved
carrier operation.

Complete type and const substitution produces one concrete executable body and
one concrete teardown artifact for that specialization. The ordinary
specialized path uses direct exact-carrier actions. Only intentional erasure
boundaries, such as existentials, dynamic trait values, or an explicit code-size
fallback ABI, may use a closed typed teardown capability dictionary. Missing or
mismatched dictionary entries refuse execution. Distributed identities are
stable content identities, never process-local type IDs, table positions, or
vtable pointers. The enclosing erased carrier retains a statically known release
operation. Concrete plans may be deduplicated only when layout, callable ABI,
effects, finalizers, and carrier actions match exactly.

### 10. Finalization identity is portable; execution authority is local

A finalization target in a published plan is a hash-covered portable descriptor.
It binds the exact callable lifecycle ABI, effects, evidence schema, and
synchronous or awaited class to either a verified Shape artifact or a sealed
native capability coordinate. Native coordinates use versioned contracts,
provider releases, and opaque operation identifiers—not symbols, paths,
function pointers, or registry positions.

Portable hashes are canonical, domain-separated full digests rather than
truncated reflection identities. A reproducible-execution policy may require a
portable Shape realization or an attested native environment; semantic
capability compatibility alone does not claim bit-for-bit reproducibility.

Admission resolves and cross-checks each descriptor, then mints a
non-serializable `ResolvedFinalizationTarget` pinned to the artifact or provider
generation. Interpreter dispatch uses a dense resolved slot and native code may
use a relocation or direct call. Catalog replacement or provider unload
invalidates future admission and reusable caches, but cannot revoke a target
under an active pinned placement lease. The old generation drains, remains
retained, or is fenced into explicit abandonment. An alternative realization is
legal only when predeclared with the same ABI, effects, evidence, suspension,
cancellation, and observable lifecycle semantics and selected during admission,
never during teardown.

### 11. Placement admits the complete lifecycle before execution

An artifact's `TeardownCapabilityClosure` transitively includes all Shape
finalizer artifacts, sealed native operations, exact releases, effects,
permissions, evidence schemas, schedulers, quiescence and cancellation support,
providers, and devices reachable on any outcome edge. Rare failure paths are
included.

The selected receiver verifies and resolves the complete closure before a frame
exists, then mints a frame-lifetime `PlacementCapabilityLease` that pins those
bindings. Admission refusal is `DefinitelyNotExecuted`. Same-placement
deoptimization retains the lease; migration must admit the destination before
ownership transfers. No teardown capability may be discovered or silently
substituted after execution starts.

`ExecutionPolicy` always exists and defaults to `Unbound`. Unbound lets the
scheduler choose among already authorized, suitable placements; it grants no
permission, provider, device, or execution authority and cannot bypass
admission.

### 12. Dependent work must prove quiescence

Dependent work that borrows an owner receives an affine `BorrowerToken` bound to
the scope, borrowed owners, execution domain, and provider or isolation
generation. Scope exit first seals further admission, then resolves every token
as joined, definitely not admitted or executed, isolation revoked, or
transferred with a typed acceptance receipt. Only then may borrowed owners be
finalized or released.

Providers supply sealed witnesses for these fixed outcomes; they do not define
new quiescence states. Local joins, nested-scope receipts, device fences, stream
unsubscribe acknowledgements, and remote certainty evidence are possible
witnesses. A cancellation request, timeout, disconnect, or abort handle alone
is not proof that execution stopped. Unknown work remains suspended, transfers
to an accepting recovery supervisor, or crosses proven isolation revocation.
Suspension and resumptive deoptimization transfer tokens intact. Functions with
no dependent scope carry no quiescence state; serialized copies do not borrow
sender memory.

### 13. Calls compose through one lifecycle ABI

Every callable has a versioned, hash-covered `CallableLifecycleABI` describing:

- receiver invocation mode: shared, exclusive, or consume-once;
- exact parameter and capture types and carriers;
- inline, owned, shared-borrow, and exclusive-borrow boundary roles;
- an inline, transferred-owned, or provenance-bearing reborrowed return;
- exhaustive evaluator outcomes; and
- the effect contract.

`ArgumentPack<Sig>`, `Next<Sig>`, failed-attempt capabilities, and exact return
types from ADR-012 are indexed projections of this ABI. They cannot widen,
erase, or reconstruct the contract from a name or a homogeneous runtime
collection.

Its stable lifecycle ABI hash is an exact type-level identity distinct
from the exact function hash, implementation, plan, dependencies, and placement
lease and remains exact. Higher-order compatibility keeps ownership, return,
outcome, and mandatory lifecycle structure exact while permitting a separately
replayable proof that the actual normalized closed effect row is a subset of the
permitted closed row. Open rows must close before materialization. Placement
admits the actual capability closure and binds the direct target, so verified
calls perform no row comparison, proof replay, dictionary lookup, or capability
check. Physical layout and retain/release elision remain outside this ABI.

Call entry is an atomic ownership commit. Before `Entered`, argument ownership
remains with the caller and no declared callee borrower token exists. At
`Entered`, owned inputs become exactly one callee obligation and declared
borrows mint their borrower tokens.
`DefinitelyNotExecuted` leaves the caller unchanged. `OutcomeUnknown` transfers
the attempt and obligations to recovery rather than speculatively restoring
owners. Completion transfers only the declared return disposition; failure and
cancellation settle the callee plan, while suspension transfers it.

At commit, a consuming receiver is adopted first and owned parameters in
declaration order, independent of named-argument spelling or preparation order;
borrowed inputs receive no rank and callable capture ranks persist. An owned
return remains under transfer authority until callee teardown completes, then
receives a fresh caller-region rank. A reborrowed return keeps its origin and no
owner rank. These are static semantic ordinals with no ordinary runtime rank
metadata and remain explicit under inlining and remote entry.

A source `Result<T, E>` is an ordinary completed value. It is not an evaluator
failure merely because it contains `Err`.

### 14. Optimization composes proofs before freeze

Inlining occurs before the region teardown freeze boundary. The optimizer
clones executable MIR with its region, owner, outcome, and effect provenance;
substitutes the accepted callable lifecycle boundary; remaps identities; and
then freezes and independently verifies one new composite plan. It never
concatenates already-lowered cleanup epilogues as semantic authority.

Backends may erase disarmed work, fuse continuations, share byte-identical
cleanup suffixes, or outline cold chains when order and ownership identity are
preserved. Inlining cost includes cleanup code, certificate and deoptimization
metadata, and capability closure. Deoptimization metadata reconstructs every
logical inlined frame's verified plan and dynamic state.

A true frame-eliminating tail transfer is legal only after explicit argument and
scope transfers leave the caller plan empty and disarmed, with no caller-frame
borrow, borrower token, result adaptation, outcome transformation, or
unadmitted placement obligation. The callable return, outcome, and effect ABIs
must match exactly, the callee lease must accept every transferred obligation,
and deoptimization must not recreate an eliminated owner. Otherwise the
operation is an ordinary call or an inline candidate. Shape does not promise
general constant-space tail calls by building a hidden runtime cleanup chain.

### 15. Snapshots and distribution preserve plan identity

Published semantic plans and certificates travel with their content-addressed
function artifacts and are verified by the executing receiver. Native and
interpreter epilogues remain rebuildable local caches.

Snapshots store only genuinely dynamic armed state, cleanup cursor, and ordered
evidence, tied to the exact function-artifact and plan hash. They do not copy the
static plan. A snapshot is refused when a live cleanup, borrower, provider, or
recovery obligation lacks a versioned provider-neutral restoration contract.
Live handles, credentials, provider grants, and process-local resolved targets
are never serialized.

Remote Own inputs use one placement-bound `TransferId`. Serialization first
moves them into inaccessible sender escrow; it does not transfer ownership.
Receiver decode plus `CallEntryCommit` atomically reconstructs receiver-local
owners and persists the canonical payload/content reference, roles, ranks,
lifecycle state, and a durable receiver recovery owner before acceptance receipt
visibility or execution. That receipt lets the sender record transfer and run
exact Carrier Release on escrow carriers without Finalization; only a durable
fenced pre-commit rejection may restore them. Retries reuse the same ID and
payload. Owned results mirror this escrow, durable recovery, commit, and receipt
ordering. Migration durably prepares an inaccessible, inactive destination
candidate; durably fences the source; atomically activates destination owners
and continuation with preserved ranks; and only then publishes its receipt.
Crash recovery between steps never yields two active semantic owners. Permanent
uncertainty quarantines escrow and its affine recovery obligation; timeout or
partition never resurrects moved owners.
Direct local calls have no journal, serialization, or receipt cost.

A `PortableContinuationArtifact<Sig>` may be produced from an exact resolved
callable authority after its artifact, captures, callable lifecycle ABI,
effects, permissions, and teardown-capability closure verify. Converting an
affine `Next<Sig>` consumes its local-call authority; an ordinary callable
follows its declared invocation mode. The result is portable evidence, not
execution authority. A receiver separately validates it against a chosen
placement, pins the required realization and
`PlacementCapabilityLease`, and only then mints a non-serializable
`AdmittedExecution<Sig, P>`. Dispatch consumes that single-attempt authority.
Neither value is reconstructed from a function name, wrapper marker, raw
function ID, or serialized local handle.

## Core and library boundary

The core standardizes ownership regions, semantic outcomes, verification,
ordering, transfer, quiescence states, structural retirement, exact carrier
release, callable lifecycle composition, and execution admission. These are the
minimum semantics required for VM/JIT parity and safe local or distributed
execution.

Libraries define resource protocols, finalization bodies, evidence schemas,
settlement goals, device or provider integrations, scheduling preferences, and
domain APIs. A library may consume every facility in this ADR without adding a
domain type, operation, scheduler policy, or teardown opcode to the core.

## Consequences

- VM, native, suspended, deoptimized, snapshotted, and remote execution share
  one lifecycle authority and one independently checked call boundary.
- Missing cleanup knowledge becomes a compile-time or admission refusal instead
  of a backend-specific leak-compatible fallback.
- Ordinary typed kernels retain direct calls, static epilogues, allocation-free
  non-materialized callables, and zero overhead when no lifecycle work exists.
- Distribution is designed into artifact identity, verification, capability
  admission, quiescence, and migration rather than retrofitted around local
  pointers or backend cleanup.
- The compiler and verifier become more complex, and published artifacts carry
  compact ownership/effect evidence. Concrete generic specialization may
  increase code size; explicit erased ABIs are the controlled fallback.
- Observable finalization order becomes stable language semantics. Optimizers
  must prove that reordered or elided retirement is unobservable.
- Provider-dependent frames pin their admitted bindings for their lifetime.
  Provider replacement therefore invalidates dependent caches rather than
  silently changing teardown behavior.
- General tail-call elimination is intentionally conditional on an empty,
  disarmed caller plan.

## Sequencing

ADR-009 plus the identity and annotation corrections in ADR-011/ADR-012
complete upstream. Work on this ADR must not recreate their semantic
elaboration inside teardown or a backend, and upstream annotation work must
not claim this ADR's final execution authority.

Issue #58 establishes the canonical first-class callable carrier and its exact
lifecycle authority. Issue #56 may establish the general region-plan pipeline,
exact non-callable carrier catalog, fail-closed preflight, verification, and
static teardown lowering independently. Until #58 is complete, any function
that could own a materialized callable must refuse the affected executable path;
partial cleanup that skips callable values is forbidden. Teardown-total callable
support in #56 therefore depends on #58.

Rollout must preserve one semantic plan for both interpreter and native tiers.
Backend enablement is gated by exact proof coverage and performance checks,
including zero overhead for empty/scalar paths and no plan interpretation in
ordinary hot code.

## Rejected alternatives

- **A frame-only cleanup plan.** Region-leaving edges inside a frame can release
  or transfer owners before terminal return; a root-frame projection cannot
  authorize those edges.
- **Backend-reconstructed cleanup.** Independent interpreter and native analyses
  recreate semantic drift and cannot support trustworthy cached or remote
  artifacts.
- **A runtime slot scan, generic action list, or universal ownership ledger.**
  These add hot-path work, cannot infer exact carrier authority from layout, and
  obstruct native optimization.
- **Partial native cleanup.** Releasing only recognized carriers leaves normal
  return leak-compatible and may mask refusal for an unproven owner.
- **Multiple first-class callable carriers.** Origin-dependent representations
  make exact ownership and release ambiguous at joins and erased boundaries.
- **Eager allocation for every callable.** Direct calls and proven non-escaping
  closures need no first-class carrier.
- **Trusting a hash, signature, or compiler provenance as proof of soundness.**
  Integrity and authorship do not prove ownership totality or exact-once release.
- **Per-invocation plan verification.** Verification is an admission operation
  cached by semantic identities; repeating it would add needless runtime cost.
- **Executable unresolved generic plans.** Exact layouts, effects, finalizers,
  and carrier actions exist only after substitution; ordinary specialization
  must not degrade to runtime kind dispatch.
- **Arbitrary cleanup callbacks or plugin-defined opcodes.** They bypass the
  sealed ordering, effect, fault, suspension, and ownership model and pull
  domain policy into the core.
- **Lazy capability discovery during teardown.** Execution cannot safely begin
  if a terminal or rare failure edge may require an unavailable action.
- **Treating cancellation as quiescence.** A request or timeout does not prove
  that dependent work can no longer access an owner.
- **Unconditional tail jumps with deferred cleanup.** A hidden cleanup chain is
  a runtime ledger and changes observable caller-versus-callee teardown order.
- **Deferring distribution concerns.** Serialized pointers, local registries,
  and implicit capabilities would make later remote execution either unsafe or
  a second incompatible architecture.
