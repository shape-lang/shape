# ADR-011–016 canonical execution rulings

Status: **RATIFIED 2026-07-27; enactment pending**

These rulings are the canonical implementation authority for the ADR-011/012
program. They compose ADR-011 through ADR-016 into dispatchable decisions and
replace the earlier patch-on-patch wording in this file. The accepted ADRs own
language and runtime semantics; this file fixes tracer choice, migration order,
ticket ownership, and evidence. If a summary, ticket, registry row, or
historical report conflicts with an accepted ADR, the ADR governs.

R1–R20 labels are preserved because tickets and reviews cite them. Their only
current meanings are the sections below. R20 records absorption and
supersession; it is not another semantic amendment.

No static source-path or match-count claim in this file is an acceptance
baseline. Enactment generates exact inventories from the selected repository
revisions, commits them, and makes them shrink-only. This prevents a moved file
or changed count from making the authority stale.

## Quality threshold: 9/10 versus 10/10

A decision or ticket is **9/10** only when its semantics are closed, one deep
module/interface owns the behavior, blocker and ticket ownership are correct,
documentation and Book obligations are committed with the slice, and its
acceptance criteria bite rather than restate intent.

It becomes eligible for **10/10** only after the implementation is landed at
the exact reviewed HEAD with a positive load-bearing witness, a deliberate
negative-control or mutation test, applicable compiler/LSP/VM/JIT/provider
parity, the required observability, performance, or fault evidence, and deletion
of the superseded authority. Design prose alone cannot score 10/10.
`NativeExecutionWitness` is additionally mandatory for a native-execution
claim; successful execution in JIT mode is not by itself native evidence.

## R1 — One bounded declaration-and-contract discovery query

`DiscoveryEngine` owns one explicit bounded worklist inside a tracked semantic
query. It joins two distinct typed domains:

- resolved declaration headers; and
- public contract contributions.

Every checked expansion returns an immutable delta containing those domains,
tracked dependencies, and complete provenance. Equal content republished under
one identity is idempotent. Different content under that identity is a
structured mutation/cycle error. Body elaboration cannot publish a header,
signature, effect, outcome, ownership rule, or lifecycle requirement after
effective-contract freeze.

Run-once accounting uses the complete six-component `ExpansionIdentity`:

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

The convergence state covers published identities and content, pending
expansions, and dependency edges. Bounds and diagnostics are Shape language
policy, not Salsa cycle policy. Worklist order, display names, spans,
allocation identity, and database-local ids never affect the snapshot.
Enactment generates the current round, expansion, and resource baselines from
the implementation rather than freezing old numeric claims in this document.

#94 is the route-completion and deletion capstone, not one oversized first
implementation. A `DiscoveryEngine` tracer and bounded module/item,
trait-default, and expression/generated/cross-module route slices precede it.
Compiler, artifacts, LSP, and later elaboration consume the same immutable
`DiscoverySnapshot`.

## R2 — Annotated callables require a normalized declared contract

`NormalizedDeclaredContract<Sig>` exists only when every call-visible
parameter and result has a resolved explicit declared type. `_` and body
inference do not satisfy that requirement. Normalization may apply only
resolution-independent declaration rules such as receiver normalization,
default materialization, and ABI-visible out-parameter/result transforms.
Resolved generic binders remain explicit `TypeParamRef` values.

Annotation selection and contract contributions never depend on body
inference. A source or generated annotation applied to an inferred-signature
callable produces a structured diagnostic at the declaration with related
application provenance. Ordinary unannotated callables may retain inferred
results and existing let-generalization behavior.

Contract elaboration publishes every externally visible delta before callers,
implementations, overrides, artifacts, and LSP facts check the effective
contract. Body/plan elaboration runs after freeze and cannot repair it. #95 and
#96 therefore do not introduce a pre-freeze inference SCC.

## R3 — Comptime is pure by default and enters through `ComptimeHost`

Every comptime execution enters one deep module:

```text
ComptimeHost.evaluate(
    CheckedComptimeRequest,
    ComptimeGrantSet,
) -> TrackedComptimeResult | ComptimeDiagnostic
```

An empty or omitted grant set is pure. No comptime path constructs an allow-all
VM or directly observes clock, randomness, environment, filesystem, process,
network, target configuration, or secrets.

A granted external operation uses a registered typed provider and records its
provider identity/version, normalized request, toolchain and target
configuration, public content digest, provenance and dependency edges,
resource limits, and freshness/offline-lock/reproducible-snapshot evidence.
Canonical sorted tracked-input identities and public digests are the only
external-input contribution to
`ExpansionIdentity.TrackedDependenciesHash`. A dependent digest change reruns
the exact comptime/discovery queries; an unrelated change demonstrates early
cutoff.

Secret grants are opaque provider authority. Secret bytes never become
ordinary comptime values and never enter query keys, dumps, logs, diagnostics,
virtual documents, dependency hashes, expansion output, artifacts, or
snapshots. Release artifacts reject live external data without reproducible
snapshot or lock evidence. No query that executes comptime is memoized before
this envelope is installed.

The first ticket migrates one production public-configuration tracer through
`ComptimeHost`; a capstone inventories and closes every remaining ambient
entry point before the general discovery query depends on memoized comptime.

## R4 — Closed, stage-branded effects are not host authority

ADR-014's closed algebra is binding. Runtime operational effects are:

```text
FsRead | FsWrite | NetConnect | NetListen | Process
| Env | Time | Random | Ffi
```

A runtime row contains those operations plus `Suspend` and
`Remote(ResolvedProviderIdentity)`. A comptime row contains only separately
branded operational effects in v1. Rows are canonical sets, sorted,
deduplicated, hash-covered, and catalog-versioned. There are no open strings,
wildcards, row variables, or implicit stage conversions in v1. Every foreign
call edge contributes `Ffi`; `Remote(ResolvedProviderIdentity)` does not imply
network use.

The semantic facts remain distinct:

- declared/actual effects;
- required operational grants;
- required scopes;
- execution constraints; and
- provider-admission requirements.

There are two proof moments. Pre-body `ContractEffectSubsetEvidence` binds
stage, catalog version, callable relationship, canonical row hashes, and the
effective-contract identity; it binds no MIR and makes no claim about an
unchecked body. Effect joins and annotation contributions enter that effective
contract before freeze.

After #93 body/plan elaboration, annotation-free lowering, and semantic
optimization, `VerifiedEffectProof` binds actual and allowed rows to one
`FinalMirIdentity`. R6's `CapabilityProof` verifies that same final,
outcome-explicit MIR. ADR-010 freezes only after both final proofs. Neither
contract evidence nor a final MIR proof grants host authority.

The closed-effect contract ticket is blocked by #91 and #92 and is a native
blocker of #96. The post-#93 executable-verification slice implements the final
proof moment. #96 must not invent an annotation-local representation.

## R5 — Portable application and expansion identities are complete

`ApplicationIdentity` is a versioned, domain-separated canonical hash of:

- resolved annotation identity;
- resolved target identity;
- typed `ConstArgumentProduct` identity; and
- one occurrence identity.

A source occurrence uses only its ordinal among applications with the same
annotation, target, and typed arguments on that target in canonical lexical
order. It does not use a byte offset or source span.

A generated occurrence uses `GeneratingExpansionAnchor` plus a versioned
`GeneratedApplicationPath`. The anchor binds parent generator, application,
target, stage, and typed arguments while excluding dependency-content hashes.
The path is an explicit stable structural key; it is not an emission index.
Duplicate paths reject. An explicit indexed component is legal only when
position is intended semantic content. Inserting an unrelated generated
sibling preserves stable-path sibling identities.

Portable definition, application, expansion, intrinsic, provider, artifact,
transfer, episode, attempt, and MIR identities never contain Salsa ids, dense
opcodes, display spelling, allocation addresses, or source spans. Portable
round-trip and fresh-process equality are required wherever an artifact,
source map, journal, cache, or cross-module query claims stability.

## R6 — Capability ownership is structural on final MIR

The post-elaboration verifier classifies values structurally:

| Class | Copy | Implicit discard | Obligation |
|---|---|---|---|
| `Unrestricted` | yes | yes | none |
| `Affine` | no | yes | at most once |
| `Linear` | no | no | exactly once: consume, return, transfer, or evaluator teardown |

Products and enums inherit the maximum class of their contents. An exhaustive
consuming match transfers only the selected payload. Dynamic collections of
affine or linear values are rejected in v1.

The sealed initial classifications are:

- `Next<Sig>`: affine;
- `RecoveryObligation`: linear;
- `AdmittedExecution<Sig, P>`: linear; and
- ADR-010's teardown continuation: linear.

Only compiler-sealed consumers selected by resolved `IntrinsicId` may mint,
split, settle, transfer, or destroy these capabilities. Spelling, source shape,
traits, casts, reflection, serialization hooks, and homonyms confer no
privilege.

The verifier is path-sensitive over products, enums, branches, calls, cleanup
regions, transfers, and every reachable `Completed`, `Failed`, `Cancelled`,
`Suspended`, and `ContainedFault` edge of the optimized outcome-explicit MIR.
It returns `CapabilityProof`; ADR-010 requires that proof at the final freeze.
A runtime consumed flag is debug defense only.

## R7 — Failure intent precedes cleanup; evaluator-private Retry Commit follows

A typed failure-handler fold produces exactly one linear intent:

```text
FailureIntent<R, Sig> =
    Propagate(RuntimeFailure)
  | Recover(R)
  | Retry(RetryIntent<Sig, Scope, Attempt>)
```

`RetryIntent` is a request, not replay or execution authority. It owns an exact
replay-safe `ArgumentPack<Sig>`, `NotExecutedProof` or scoped
`ReplayEvidence`, optional typed post-cleanup backoff, and failed-attempt
provenance. Its pack must be disjoint from the failed attempt's teardown
closure or reconstructible from replay evidence.

The only re-entry order is:

```text
failure-handler fold
  -> linear RetryIntent
  -> teardown of every activated layer
  -> evaluator-minted CleanupComplete<Episode, Attempt>
  -> post-cleanup backoff
  -> budget/deadline check and attempt-permit consumption
  -> evaluator-private Retry Commit
  -> one fresh Next<Sig>
  -> next attempt
```

User code never receives `CleanupComplete`, never commits retry, and never
mints or receives a spare `Next`. Cleanup failure, expired deadline, exhausted
budget, or insufficient replay evidence produces a structured denial retaining
the original failure; no best-effort attempt starts.

Replay evidence feeds only `RetryIntent`; it never makes the failed attempt's
affine `Next` reusable. Retry Commit mints a fresh `AttemptId` and fresh `Next`
under the same episode. A remote retry also uses a fresh `TransferId` and new
admission. Any inaccessible escrow and `RecoveryObligation` from the failed
attempt must be settled or atomically transferred to the new transaction before
commit, so the retry cannot duplicate ownership.

## R8 — One `RecoveryEpisode` owns total attempts, deadline, and history

The evaluator owns one episode from initial attempt through completion,
recovery, terminal propagation, cancellation, or durable transfer of every
unresolved obligation. It records a stable episode identity, append-only
attempt history, effective packs and hook state, replay evidence, cleanup
evidence, parent budget, and outstanding recovery obligations.

Every replay-enabled episode has:

- `max_attempts`, including the initial attempt; and
- one absolute deadline shared by discovery, connection, admission, backoff,
  execution, cancellation, and reply waiting.

Default execution has `max_attempts = 1` and no replay authority. Source
`@retry(3)` means at most three re-executions and lowers to
`max_attempts = 4`. Tooling exposes both source retries and total attempts.
A replay contract without a finite enclosing or declared deadline rejects.
Hooks and providers may narrow but never reset or extend the parent bounds.

Backoff and jitter run after cleanup as ordinary typed callables whose effects
are part of the effective contract. Durable restore adopts the stricter of
recorded absolute deadline and last remaining duration.

## R9 — `RecoveryJournal` is the single durable recovery authority

A `RecoveryObligation` exists only after durable state records what it owns. It
is a linear handle to one journal entry, not a serializable in-memory record.
Escrow content is durable before a transition may name it; live credentials,
sessions, pointers, routes, provider handles, and leases are never escrow.

`RecoveryJournal` alone owns monotone sender/receiver transitions, escrow
inventory, acceptance, settlement, crash recovery, and compaction. Every
transition is generation-checked and idempotent by `TransferId`.
`DefinitelyNotExecuted` requires a durable receiver
`RejectedBeforeCommit` proof for the exact transfer and generation. Timeout,
disconnect, restart, assertion, or lease expiry alone leaves
`OutcomeUnknown`.

A `TransferId` identifies one attempt's ownership transaction. Transport
retransmission of that same attempt reuses the identifier and recorded payload;
a semantic retry never does. It remains in the same `RecoveryEpisode` but uses
the fresh `AttemptId`, `TransferId`, admission, and single-owner transition
required by R7.

`RemoteOutcome<R>` distinguishes completion, settled remote failure, confirmed
cancellation, proven non-execution, and uncertain outcome carrying inaccessible
escrow plus a linear `RecoveryObligation`. The transparent surface preserves
declared `R` and never resumes the caller with speculative ownership. The
recoverable surface returns the typed uncertainty and obligation. A convenience
`Result` is legal only after the contract proves uncertainty impossible or an
explicit policy settles/transfers the obligation.

Journal v1 is an append-only checksummed frame log with exclusive writer epoch,
durable blob-before-transition ordering, durable transition-before-receipt
ordering, valid-final-prefix recovery, corruption quarantine, and compaction
that preserves exactly one owner. `DurableSupervisor.accept` is sealed and
exhaustive:

```text
Accepted(TransferReceipt)
| Refused(RecoveryObligation, SupervisorError)
| AcceptancePending(PendingAcceptance)
```

Only durable `SupervisorOwned` state may return `Accepted`; ambiguous storage
returns a still-linear pending handle.

An expiring, non-serializable `AdmissionLease` authorizes entry only. Call Entry
Commit consumes it and mints a `PlacementCapabilityLease` that pins the exact
provider generation and teardown-capability closure for the whole frame.
Pre-entry expiry may yield durable non-execution proof; post-entry expiry is not
revocation. Cross-placement migration must admit the destination before fencing
and transferring the source.

Compatibility domains advance independently:

- **Recovery Journal v1** is the journal format;
- **wire protocol v3** is the next ownership-aware remote envelope; and
- **snapshot format v8** is the next snapshot format.

Wire v3 performs its fixed envelope and exact execution-ABI handshake before
ownership transfer; an old peer rejects before user entry. Snapshot v8 uses a
fixed magic/version/length/checksum header and cross-version refusal before
payload trust. Snapshot state references journal ownership; it never duplicates
the owner or serializes live provider capability. The implementation order is
wire-v3 envelope, journal v1, v3 ownership payload/Remote Dispatch, stable
obligation/lease runtime fields, then snapshot v8.

## R10 — `ArgumentPack<Sig>` is signature- and mode-indexed

`ArgumentPack<Sig>` is an opaque heterogeneous product over the normalized
call-visible contract. Defaults are already materialized and native out
parameters are not slots. Only the compiler may mint a call-site pack; a
trusted remote path may mint one only after exact schema, ABI, and ownership
transfer proofs.

`ParamDescriptor<Sig, I, T, Mode>` binds exact callable identity, ordinal,
type, and passing mode. Descriptors from another signature are incompatible.

| Mode | Projection | Replacement |
|---|---|---|
| `CopyInput` | copied `T` or scoped `&T` | owned `T` |
| `OwnedInput` | scoped `&T`, never extraction | owned `T` only when displaced `T` is discardable |
| `SharedBorrow` | compatible scoped `&T` | lifetime-compatible shared borrow |
| `ExclusiveBorrow` | scoped exclusive reborrow | lifetime-compatible exclusive borrow |

Replacement consumes and returns the same signature-indexed pack. A
non-discardable owned slot cannot be replaced in v1. There is no runtime
length, name lookup, integer indexing, iteration, dynamic collection, raw slot,
cast, structural equality, or public representation.

`comptime for p in params_of<Sig>` clones and specializes its body once per
parameter before ordinary checking. Every instance has an exact descriptor;
`where` filters select admissible instances, and a missing operation names the
offending parameter. Expansion has versioned arity and node budgets. #97 owns
this general pack/`Next` vertical, not an annotation-specific tuple substitute.

## R11 — Compile-stage construction is identity-first and typed

Generated declarations accept only canonical `TypeRef`, explicit
`TypeParamRef`, and a `GeneratedName` minted from an explicit source binder,
hygienic identity, or versioned `TypedNamePolicy`. Validating an independently
supplied string does not mint naming authority. Type references come from
resolved `TypeRef::of<T>()`, exact target reflection, explicit type parameters,
and total constructors for applied, callable, reference, record, tuple, union,
and nominal types. Every constructor uses the canonical interner.

Typed `ItemFn`/`ItemType` builders publish complete headers through
`DiscoveryEngine`. Rendering is presentation only; no rendered type, source
fragment, path string, display name, or raw symbol insertion may re-enter
semantic construction. Quasiquotation, if later added, lowers to the same
builders and identities.

#106 is the construction API plus one generated-item tracer. Separate
consumer-migration and parser-deletion tickets follow; the deletion is a direct
blocker of #110. #94 is a direct prerequisite because generated declarations
must enter stabilized discovery rather than a parallel table.

## R12 — Target support is conditional on one typed adapter matrix

Every application enters the common elaborator. An operation runs only when a
`CallableTargetAdapter` proves that normalized lifecycle ABI, closed effects,
outcomes, ownership, and requested body access compose for that exact target.
Unsupported combinations reject structurally; no clause silently disappears.

The first-version matrix is:

| Operation | Shape callable | `extern "C"` | Python/TypeScript | Field/parameter |
|---|---|---|---|---|
| identity, ordering, exact facts | yes | yes | yes | yes |
| declared contract | exact Shape | normalized visible | declared adapter | exact descriptor |
| contract-only generator | yes | yes | yes | metadata only |
| checked body inspection | yes | `OpaqueForeignBody` | `OpaqueForeignBody` | not applicable |
| around transform | yes | normalized call stub | normalized call stub | reject |
| async/suspend | declared contract | reject in v1 | declared contract plus `Suspend` | reject |
| remote placement | portable artifact | admitted library/symbol manifest | admitted extension/provider | reject |
| frozen facts/persistence | yes | yes | yes | yes |

Every foreign edge contributes `Ffi`. Native out parameters become the
normalized call-visible result. An around transform wraps the VM-owned
normalized stub, never raw C ABI or foreign source. #105 owns the executable
positive/rejection matrix across every cell.

## R13 — `@prompt` contributes a checked, consumable template

`@prompt` is an ordinary compile-stage contract annotation. It neither invokes
a model nor silently changes the implementation. It contributes
`CheckedPromptTemplate<Sig>` before effective-contract freeze by parsing a
ConstLift string against `NormalizedDeclaredContract<Sig>` and resolving every
required `ToPrompt` obligation for its exact parameters.

The first grammar permits literal UTF-8, `{identifier}` placeholders, and
escaped braces `{{`/`}}`; expressions and format specifications reject.
Placeholders resolve to exact `ParameterIdentity` plus source mapping. Unknown
names receive a structured diagnostic and nearest-name suggestion. Repeats and
unused parameters are legal.

A referenced parameter must support a shared borrow and resolved `ToPrompt`.
`ToPrompt` is pure, total, runtime-effect-free, evaluator-failure-free,
non-suspending, locale-independent, adapter-independent, and canonical UTF-8.
There is no blanket display bridge, and capabilities or compile-stage secrets
cannot implement it.

Acceptance requires a second ordinary annotation to consume the checked fact,
render live values through typed `ArgumentPack` projections and resolved
`ToPrompt`, then call `Next<Sig>`. No prompt-name recognizer, marker, model
call, or prompt-specific backend path is permitted. Post-freeze plan elaboration
may consume the frozen checked template, but it cannot select new trait facts
or change the frozen contract.

## R14 — Migration defaults new by identity and forbids bridges

The migration manifest is keyed by resolved semantic identity, never spelling.
It contains an explicit finite legacy set. An identity not listed there,
including every newly declared annotation or intrinsic, defaults to the new
semantic pipeline. It is never “legacy unless opted in.”

Each tracer slice moves identities out of the legacy manifest and lowers the
generated baseline. The set may only shrink. At final deletion the routing
authority itself disappears.

Growth-freeze baselines are generated from the exact enactment revisions for
all old authority classes: universal targets, string-backed metadata/type
construction, AST/name recognizers, pseudo-packs, marker substitution, raw
generated names, ambient builtin selection, duplicate LSP semantics,
annotation-specific backend paths, and stale tests/docs that assert those
mechanisms. New surfaces cannot increase a baseline.

No compatibility bridge may translate an old untyped plan, descriptor, string,
marker, or name-selected value into the new typed authority. A vertical slice
must enter through resolved typed inputs and delete its old consumer; an
anti-bridge ratchet detects reintroduction.

## R15 — Native claims require a fresh repro and `NativeExecutionWitness`

The JIT prerequisite starts by recreating the failure at the selected HEAD with
a minimized, committed, backend-general reproducer. Historical path, arity, or
capturing-closure descriptions are evidence to investigate, not proof of the
current defect. The ticket records the exact revision, toolchain, function
identity, tier threshold, observed dispatch path, and whether the current
result reproduces, changed shape, or is already fixed.

A general Core/MIR/JIT fix may run parallel to VM-side semantic slices, but no
slice may relabel interpreter fallback as native. A native claim requires a
structured `NativeExecutionWitness` binding:

- exact source/definition and MIR/realization identities;
- tier-up or native-install event for that realization;
- subsequent native dispatch on the covered path;
- zero covered fallback/deoptimization events;
- VM/native semantic equality; and
- exact Shape revision, binary, backend, toolchain, and witness schema version.

The witness comes from the runtime/JIT authority, not parsed log prose.
Fallback is acceptable only when the public feature contract and Book say so.
One infrastructure ticket lands the witness schema, collector, false-positive
controls, and Book-facing projection. A separate post-callable-carrier ticket
authors the fresh capturing-HOF repro and, only if still red, repairs the
general first-class-callable site. #97's native close criterion consumes both;
an annotation ticket does not own the general JIT repair. Annotation-aware JIT
exceptions remain forbidden.

## R16 — #91 is a narrow immutable Salsa seam

#91 adopts the Salsa crate and pins one exact compatible release, feature set,
lockfile entry, supported Rust version, cancellation/snapshot ownership, and
initial query-memory budget. Salsa owns revision storage, dependency recording,
red-green validation, early cutoff, local interning, and concurrent reads.

The first production slice publishes only resolved `DefinitionIdentity`,
normalized base contract, deterministic diagnostics, and source provenance for
`fn add(a: int, b: int) -> int` plus one call site. Compiler and LSP consume
the same `CallableFacts` content identity.

The stop line is binding: #91 does not migrate annotations, generated symbols,
method tables, discovery, comptime, typed Core/MIR, or backend state.
`BytecodeCompiler`, programs, mutable expression/function stacks, journals,
backend caches, and VM/JIT state are never Salsa inputs, tracked values, or
query-owned mutable state. `BytecodeEmitter` remains an ephemeral mutable
consumer of immutable semantic facts.

Salsa ids are local accelerators, never portable identities. Shape owns
language cycles and diagnostics. Acceptance includes fresh-session fact
identity plus comment-only, body-only, signature, import-retarget, alias, and
local-shadow edit traces with declared rerun/early-cutoff expectations.

## R17 — Named active tracers are fixed and non-vacuous

Implementers do not substitute convenient or deleted examples:

- **#91:** `fn add(a: int, b: int) -> int` and one call site;
- **#92:** active declared `__native_ptr_size` through ordinary resolution and
  `IntrinsicCatalog`; deleted `__into_*`/`__try_into_*` families are invalid;
- **#95:** repeatable `@tag(label: string)` for typed arguments, source and
  generated `ApplicationIdentity`, multiplicity, and canonical order; and
- **#96:** `@requires_env(name: string)` contributing a closed runtime `Env`
  effect before caller and LSP checking.

Each tracer requires a positive import-alias case, negative same-spelled
homonym, direct inspection of the resolved fact, and an observable downstream
consumer. Equal output or a matching rendered diagnostic alone does not prove
the producer is load-bearing.

## R18 — Full `IntrinsicCatalog` rollout blocks program close, not unrelated deletion

#92 migrates only the active tracer. A separate inventory ticket generates the
complete live selector inventory across checker, evaluator, comptime, typed
Core/MIR, bytecode, VM, JIT, artifacts, access checks, declarations, and LSP.
It distinguishes genuine intrinsics from ordinary language operations and dead
code, assigns every genuine row to exactly one family, and freezes shrink-only
raw-name/origin/AST/unspellable/ambient-selection baselines.

Family waves then migrate the inventory-defined JSON/schema/serialization,
native/FFI/pointer, vector/matrix, math/statistics, series/rolling,
random/sampling, language-construction, and residual runtime/lifecycle groups.
A zero-row family is omitted rather than closed vacuously. Every row resolves a
portable `DefinitionIdentity` first; the versioned catalog validates exact
type, effects, ownership, lifecycle, stage, target, and access contract before
minting `ResolvedIntrinsic`. Backend dispatch consumes intrinsic identity, not
source spelling.

The catalog capstone proves every row has one owner, deletes parallel selectors
and adapters, round-trips portable identities through a fresh process, and
reaches the generated ratchet targets. Full rollout is a blocker of the program
completion gate (#23). It does **not** block #110's universal annotation
descriptor/string/tooling deletion unless the inventory names a specific
direct consumer.

## R19 — Public features and Book coverage are complete, executable manifests

Every public slice lands documentation and coverage with the behavior. Shape
owns a complete `PublicFeatureManifest`; shape-web owns a complete
`BookCoverageManifest`. Stable feature, section, and fence identities are not
paths, headings, line numbers, or ordinal positions; they are never reused.
Changing an ID creates a tombstone/replacement, and a schema-major change
carries a complete identity migration map.

The public manifest covers every public language construct, annotation,
stdlib callable/type, compiler-visible behavior, CLI/LSP workflow, provider,
snapshot/resume operation, and distributed operator workflow. It records
status, authority, supported targets/modes, semantic dimensions, distributed
coverage requirement, and required evidence. The Book manifest maps every
non-removed feature to owning sections/fences, positive/negative/failure
evidence, modes, dimensions, illustrative exceptions, and total
feature/section/fence identity migrations. Neither source manifest stores exact
source revisions or mutable verification state; external pair evidence owns
those facts. Mapping is complete and bidirectional, not count-based.

Status fixes the evidence contract. `public` and `deprecated` require runnable
evidence in every required mode; `experimental` requires every declared
supported mode plus structured negative limits; `planned` is not presented as
current and permits only planned-section/illustrative or structured-rejection
evidence; `removed` retains a tombstone plus migration or rejection evidence.
Status moves only forward from planned through experimental, public,
deprecated, and removed; reviewed forward shortcuts are allowed. Bootstrap
status is evidence-derived, ambiguity blocks, and a previously current or broken
feature cannot be demoted to planned.

Every Shape fence has a stable identity and is either `runnable-gated` with
declared modes/outcome or `illustrative-only` with a nonempty reason and
authority/issue. The illustrative set is enumerated and ratcheted. VM/JIT
parity means two executions; snapshot/resume passes the selected mode through
both initial and resumed execution rather than copying one result. Native prose
additionally requires R15's witness.
Distributed features cover ADR-016's full invocation, authority, provider,
admission, certainty, ownership, retry, deadline/cancellation, recovery,
cleanup, persistence/versioning, security, observability, operations, and
degraded-mode matrix.

Complete inventories may expand only from committed exact rows into a
user-ratified concrete wave graph. The inventory cannot close until every row
has one child, child/capstone native edges and prose agree, and a tracker
re-fetch plus graph audit passes.

The authoritative `BookTruthGate` validates both manifests, executes every
declared mode, and reports both exact repository SHAs and all evidence inputs.
Both CI adapters consume one immutable external `PairCandidate` and run that
same pair. A signed content-addressed external `PairAttestation` binds both
reports and contains no promotion generation. A signed content-addressed
`AcceptedPairTransition` monotonically names its expected predecessor, selected
attestation, action, actor/reason/policy, and nonce; protected-pointer
compare-and-swap plus an append-only audit makes it the sole current-pair
authority. No source revision contains a reciprocal/self revision pin.
Rollback creates a higher-generation transition selecting a prior attestation;
source reverts form a new candidate pair. Branches remain staging/history.

At the 2026-07-27 audit, an enhanced harness was committed on shape-web's
`adr009-c3-annotations` feature branch while accepted shape-web main carried a
different earlier harness. The feature-branch harness is therefore versioned
evidence, not authoritative main evidence, and main-side harness code does not
satisfy ADR-016 merely by existing. Authority begins only when
`BOOK-PAIR-PROMOTE` lands the complete manifests, gate, dual adapters, drift
guard, attestation, and rollback proof. It is a native blocker of #90 and every
later public ticket. #23 remains the final verifier, not a deferred
documentation phase.

## R20 — Absorbed and superseded formulations

This section makes earlier wording noncompetitive while preserving citation
labels. The following old formulations are explicitly superseded:

- partial `ExpansionIdentity` shorthand and hard-coded discovery counts: R1/R5
  now require complete identities and generated baselines;
- inferred or merely syntactic base returns: R2 requires resolved normalized
  explicit declarations for annotated targets;
- default comptime VM construction: R3 requires `ComptimeHost`;
- a mixed “16 permissions” bitset: R4 uses ADR-014's closed stage-branded
  algebra and separates grants/scopes/constraints/admission;
- generated application emission indices: R5 uses stable structural paths;
- outer-nominal or runtime-flag capability checks: R6 requires structural
  path-sensitive verification on final outcome-explicit MIR;
- a user-visible retry operation consuming cleanup evidence, or a decision
  constructor consuming budget: R7/R8 reserve cleanup evidence and Retry Commit
  for the evaluator and consume an attempt permit only at commit;
- an infallible supervisor receipt, reused snapshot serialization, or
  evidence-free lease-expiry restoration: R9 requires `RecoveryJournal`,
  exhaustive acceptance, journal v1, wire v3, and snapshot v8;
- mode-erased packs: R10 preserves exact passing and ownership modes;
- string-backed typed construction or a later parsing bridge: R11/R14 forbid
  both;
- uniform body/around support on every target: R12's adapter matrix is
  conditional;
- validation-only prompt markers or display formatting: R13 requires a checked
  consumable template and resolved `ToPrompt`;
- default-legacy routing: R14 defaults every unlisted/new identity to the new
  path and forbids old-to-new bridges;
- stale JIT source-path claims or “JIT mode passed” nativity: R15 requires a
  fresh repro and `NativeExecutionWitness`;
- moving mutable compiler/backend state into Salsa: R16 keeps #91 narrow and
  facts immutable;
- deleted conversion intrinsics as tracers: R17 fixes active exemplars;
- blocking #110 on every intrinsic family: R18 blocks #23/program close and
  only direct consumers;
- fence percentages, stale universe counts, or an uncommitted/unpaired harness:
  R19 requires complete manifests and the exact two-repository gate; and
- the earlier R7/R8 reconciliation as a separate amendment: it is absorbed by
  R7–R9 and ADR-015.

The former amendment bundle and checklist are historical input only. The
canonical enactment sequence below replaces them.

## #90 authority enactment — ten required steps

#90 closes only after all ten steps have committed evidence. No step is a prose
promise and no later ticket may silently inherit an uncommitted prerequisite.
#90 is a capstone: it blocks none of these child slices, and
`BOOK-PAIR-PROMOTE` directly blocks it.

1. **Commit the complete authority set.** Land ADR-011 through ADR-016, the ADR
   index, canonical glossary/current-design amendments, and this rulings file
   on the selected Shape integration base. Cross-links resolve and no current
   document presents `HookDecision`, marker substitution, pseudo-packs,
   universal targets, string metadata, ambient comptime, or name-selected
   intrinsics as current authority.

2. **Bind an exact candidate pair.** Record immutable Shape and shape-web SHAs,
   worktree intent, manifests, harness, binary, and toolchain in an external
   `PairCandidate`, never “current main,” a relative HEAD, a moving branch, or
   reciprocal/self SHA or mutable verification fields in the named source
   revisions.

3. **Commit the E6 salvage/quarantine disposition.** Give every paused E6 commit
   a durable salvage, rewrite, evidence-only, or reject disposition. Preserve
   independently valid deletion/identity evidence; reject universal targets,
   string-backed annotation/type carriers, pre-elaboration generators, and
   bridges into the replacement architecture. Make stale #20/#22/#83 work
   unclaimable while retaining useful acceptance cases in replacement tickets.

4. **Generate and commit migration baselines.** At the exact Shape SHA,
   mechanically inventory discovery producers, comptime entry points,
   annotation identities/routes, intrinsic selectors, universal/string
   descriptors, generated-type parser consumers, duplicate LSP semantics,
   annotation/backend exceptions, stale tests, and old documentation claims.
   Store stable semantic owners and generated counts/hashes; later slices may
   only reduce their assigned legacy sets.

5. **Make new identity the default and install anti-bridge guards.** Commit the
   explicit finite legacy identity manifest, default every unlisted/new identity
   to the resolved typed pipeline, reject old untyped values at the new
   boundary, and make every legacy growth baseline fail CI. Do not create an
   adapter that makes an old path look migrated.

6. **Amend live issue bodies and native edges.** After each required breakdown
   approval, apply the ratified tracer-bullet graph: narrow #91; active #92,
   #95, and #96 tracers; stable application identity; `ComptimeHost`; effect
   prerequisite; split discovery; capability/retry/journal/versioning; typed
   construction; target/prompt/LSP slices; intrinsic family waves; native
   witness; manifests and two-repository gate. Re-fetch every live issue body,
   state, label, comment, and dependency endpoint; prove prose blockers equal
   native edges. An inventory closes only after exact user-ratified child waves,
   capstone edges, and the re-fetch audit exist.

7. **Fix compatibility authority and migration order.** Make the tracker and
   current design name Recovery Journal v1, wire protocol v3, and snapshot v8
   as independent domains and preserve their binding order. No skipped/defaulted
   ownership fields or deserialize-to-discover-version path is claimable.

8. **Land complete coverage manifests.** Generate and review Shape's complete
   `PublicFeatureManifest` and shape-web's complete `BookCoverageManifest`,
   including every pre-existing public feature and fence, not only ADR-011–016
   additions. Commit never-reused feature/section/fence identities and total
   schema-major migration maps, without source-revision verification fields.
   Derive bootstrap status from evidence; reject ambiguity/backward demotion.
   Enforce full modes for public/deprecated, declared modes plus limits for
   experimental, non-current evidence for planned, and tombstone/migration or
   rejection evidence for removed.

9. **Promote the cross-repository gate.** Run both exact-pair adapters from one
   external `PairCandidate`, guard drift, and store one signed content-addressed
   `PairAttestation` outside both revisions. Create a signed monotone
   `AcceptedPairTransition`; compare-and-swap its expected predecessor into the
   protected pointer and append-only audit. Prove rollback by a newer transition
   selecting a prior attestation; source reverts require a new pair.

10. **Publish one reproducible close record and truthful frontier.** Record the
    authority commits, exact repo SHAs, issue-body/edge audit, E6 disposition,
    baseline and manifest hashes, CI workflow identities, promoted attestation
    digest/generation, remaining blockers, and calculated native ready frontier.
    #90 does not close if `BOOK-PAIR-PROMOTE` or any evidence is local-only,
    uncommitted, count-only, or tied to another pair.

## Program-wide slice rule

Every user-observable slice owns its semantic fact, compiler and LSP
projections, applicable VM/JIT/provider behavior, negative diagnostics,
incremental invalidation, documentation, both manifest updates, Book examples,
and shrink-only deletion evidence. Runtime/distributed slices additionally own
fault injection, outcome/ownership certainty, cleanup, compatibility, security,
operator recovery, and observability evidence appropriate to their contract.

The final program closes only when the old authority is deleted, all full
catalog/manifest gates pass at exact committed revisions, and #23 verifies the
already-landed evidence. A compatibility adapter, deferred Book phase,
interpreter-fallback nativity claim, or percentage/count substitute is not a
completion path.

---

# Addendum 2026-07-27 — R21–R25 (open rulings ANSWERED; enactment pending)

Everything above this line is the ratified 2026-07-27 authority and is
unchanged. The rulings below were drafted the same day at user direction,
grounded in a six-lane code scout at main `7e343c20`, adversarially
reviewed by two independent passes, and their five open decision points
were answered by the user the same day (Q1 B, Q2 A, Q3 C, Q4 as
recommended, Q5 B — recorded in
`docs/program/workstreams/ratification-grill.md`). Enactment go-ahead was
given by the user 2026-07-27 (same day, in-session): the authority set
committed on the #111 baseline and the workstream tickets published per
`docs/program/workstreams/publication-plan.json`. The only approved deltas to the
frozen tracker are the Q2 set: two new edges (into #110 and #143) and four
scope-by-reference expansions (#112, #113, #143, #163), applied at
publication with re-fetch audit. All other tickets enter the graph only
through the manifest's atomic expansion protocol.

## R21 — Function-type effect rows close at instantiation (ADR-014 §8)

Effect rows are components of function types; subsumption is subset;
boundaries declare, interiors infer; generic signatures bind `EffectParamRef`
binders that substitute to closed rows before checking, freezing, or
persistence. ADR-010 §13's "open rows must close before materialization" is
the governing clause; per-specialization effect proofs mirror the
per-specialization capability proofs of ADR-014 §6.

Ownership: the row-in-type work (`Type::Function` row component, unification
and subtyping extension, `EffectParamRef`) precedes and feeds
EFFECT-CONTRACT (#143); #143 must not land a contract-row representation the
type system cannot express. That precedence is a new native blocker edge
into published #143, part of the disclosed ratification-time delta in
`docs/program/workstreams/README.md` — approved by the user 2026-07-27
(grill Q2) and applied at publication, not by silent prose. Surface syntax
is user-ratified: the `!` clause and `effect F` binders (grill Q4a). The named tracer is a generic
`fn apply<T, effect F>(f: fn() -> T ! F) -> T ! F` with: a positive
subsumption case (pure closure passed where `{FsRead}` accepted), a negative
boundary case (closure row exceeding a declared boundary row rejects with the
materialization fix), and an instantiation-closure case proving no unbound
effect parameter survives into any persisted fact. Two standing soundness
caveats close with this work and may not be inherited: the `pure()`
catch-all in the string-keyed permission table is not a purity proof, and
closures currently have no per-value permission identity.

## R22 — Obligation batches for fan-out settlement (ADR-015 §10)

`join settle` over uncertainty-capable branches yields typed per-branch
settled outcomes plus at most one linear `ObligationBatch` — a single
verifier-visible owner whose element accounting is journal state. Sealed
consumers only: whole-batch supervisor transfer with one batch receipt
binding per-entry receipts, settlement waiting, drain of settled entries,
explicit partition. `join all` over uncertainty-capable branches is a
compile error naming `join settle` unless uncertainty is proven impossible.

Ownership: batch semantics land after JOURNAL-CORE (#154) and with the
wire-v3 ownership payloads (#153 lineage); compile-time batch obligation
tracking extends the checked-body emission authority
(`async_drop_context.rs`), never a parallel tracker. The current opaque
task-group carrier, its false `{status, value/error}` in-code claim, and the
placeholder `[TaskGroup:Settle(2)]` assertion are deleted by the
implementing slice. The batch carrier is journal-backed, not a future;
snapshot-v8 treatment follows ADR-015 §8.

## R23 — Ergonomic parity and the script tier (ADR-017, ADR-016 §10)

Every slice adding required public ceremony names its ergonomic counterpart
and cannot reach `public` status without it or a dated user waiver. The
script tier is a gated feature set with mechanical zero-ceremony
verification; ceremony budgets attach to a ratcheted flagship-fence set.
Fixes are single-sourced at the diagnostic emitter through an appended
structured-edit field; the LSP's eleven parallel validators and its
message-scraping fix-extractors become generated shrink-only baselines. Quasiquote is
v1 surface of the typed construction API and is the typed alternative that
unblocks deleting the retained `parse_type_annotation_payload` string arm;
its ratified spelling is `quote { ... }` with `${hole}` splices, and the
supervisor scope's ratified spelling is `with supervisor` (grill Q4b/Q4c).
The `var` force-`SharedCow` flag retires so the smart default is real before
it is documented.

## R24 — The performance charter is gated and measured (ADR-018)

A committed comparison suite with pinned reference runtimes carries the
charter target — user-ratified 2026-07-27 (grill Q5) as per-category bars
per ADR-018 §1's table (numeric ≥ 1.5×, collections and strings/JSON
≥ 1.0×, allocation-heavy ratcheting from 0.8× to 1.0× post-arena, startup
≥ 5×), with one recorded post-baseline calibration. Sequenced
levers: per-function deopt granularity with a whole-program-bail ratchet to
zero and tiered CLI default; closure-nativity widening and the HOF carrier
unification — PERF-CLOSURE-NATIVE explicitly assumes the R15
"post-callable-carrier repro/repair ticket" role, with #97 and #146
consuming its witness exactly as R15 specifies (the historical
arity/`todo!()` description is fixed and stale; the fresh-repro rule
stands); MIR-level retain/release pair cancellation
over existing solver facts; BCE matcher widening; value-escape analysis and
the single allocation seam as independent slices; region arenas only after
both prerequisites and the ADR-010 pipeline. Every dead analysis in
`shape-jit/src/optimizer/` receives a wire-or-delete disposition; no third
state. Every optimization ticket lands with before/after measurements on the
committed suite; benchmark files remain immutable.

## R25 — Polyglot depth (ADR-019)

The stub channel (`register_types`) becomes load-bearing; foreign bodies are
checked at compile time through `ComptimeHost` toolchain providers with
tracked toolchain/body/stub/environment digests, promoting — not forking —
the existing LSP foreign-diagnostics pipeline. Foreign environments are
declared and locked; `ForeignEnvironmentDigest` joins the foreign-function
content hash (a deliberate, coordinated hash break — sequenced with the
artifact-persistence lane). Zero-copy buffer sharing is a negotiated
versioned vtable capability with call-scoped views under the stated
in-process trust model. The foreign-ref carrier follows ADR-005/006
single-discriminator discipline with snapshot refusal semantics. Foreign async
follows the ruled two-step (grill Q3 = C): transitional rejection, then
fast-tracked offload parity per ADR-019 §5 — the feasibility scout sized
it days-scale (Python 3–5 days, both languages 1.5–2.5 weeks) because it
copies Shape's own shipped eager-offload-plus-resolve-at-await pattern;
true interpreter suspension remains a separate runtime-wide item and no
foreign-specific suspension machinery may precede the general one. The
repair belongs to the existing TARGET-PYTHON (#163) / TARGET-TYPESCRIPT
(#164) scope plus the POLY-ASYNC-OFFLOAD ticket, without edge changes; it
also fixes the spawned-async foreign-call bug and the extension-instance
aliasing hazard the scout surfaced. The per-call Python module
re-execution is an implementation defect fixed in the same lane.
