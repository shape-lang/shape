# ADR-016: Executable Public Feature Documentation

## Status

Accepted (2026-07-27)

Composes with ADR-011 through ADR-015. Public documentation is part of a
feature's acceptance, and the Shape Book is the comprehensive executable
reference for every public language, standard-library, tooling, and
distributed-computing feature.

Proposed amendment 2026-07-27 (pending ratification): §10 adds the script-tier
coverage dimension and ceremony budgets. ADR-017 owns the language-side
ergonomic-parity and progressive-disclosure rules; §10 owns only their gate and
manifest representation.

## Context

Shape's implementation and Book live in separate repositories. Historically,
language and runtime slices could close after updating a selected page or
running a curated set of examples. The full Book gate was deferred until a
program-ending documentation ticket. That allowed several failures:

- a feature could be implemented without a durable record of which Book
  sections explain it;
- examples could be labeled non-runnable without a reason;
- a JIT result could be inferred from a VM run rather than executed;
- a Shape change could miss the shape-web workflow, while a shape-web change
  tested an unrelated Shape revision;
- documentation of distributed execution could show the happy-path call while
  omitting execution uncertainty, ownership escrow, retry evidence, cleanup,
  version refusal, security, and operator recovery; and
- the final Book ticket could become a late documentation dump rather than a
  verifier of documentation shipped with each feature.

Completeness cannot be inferred from page count or the number of green code
fences. Shape needs one inventory of public features, one coverage mapping into
the Book, and one cross-repository executable gate whose report binds the exact
source revisions it verified.

## Decision

### 1. Documentation is part of every public feature

A feature is not accepted as public or current until its documentation lands
with the same feature slice.

Every feature ticket that changes user-observable behavior must include:

- a stable public feature identity;
- the user model, syntax or callable surface, and canonical terminology;
- exact types, effects, permissions, ownership, lifecycle, and evaluator
  outcomes that users must understand;
- successful behavior plus failure, cancellation, cleanup, and unsupported-use
  behavior;
- VM and JIT behavior, including honest fallback or native-evidence claims;
- compiler and LSP diagnostics where applicable;
- runnable positive and negative examples;
- version, compatibility, migration, and deprecation behavior where
  applicable; and
- updates to both coverage manifests and the Book pages that own the feature.

Internal refactoring that changes none of those facts need not create public
documentation. An optimization becomes documentation-relevant when the
language promises its availability, fallback behavior, resource model, or
observability.

Documentation may be developed incrementally in the feature branch, but the
feature cannot close as "docs pending." A temporary unsupported surface must
be documented as unsupported with its structured diagnostic and owning issue;
silence is not a status.

### 2. Shape owns the PublicFeatureManifest

The Shape repository owns one canonical `PublicFeatureManifest`. It is the
complete inventory of user-observable features, not a prose table maintained
independently by the Book.

Each entry has:

- stable `feature_id`, independent of file path, heading, or source line;
- public name and feature family;
- status: planned, experimental, public, deprecated, or removed;
- semantic authority and owning source/ticket;
- supported surfaces and targets;
- required compiler, VM, JIT, LSP, CLI, and provider modes;
- effects, permissions, ownership/lifecycle, outcome, and compatibility
  dimensions requiring explanation;
- whether distributed-semantics coverage is required;
- required executable evidence classes; and
- supersession or removal identity where applicable.

Status controls which evidence is truthful:

- `public` and `deprecated` features require runnable evidence in every
  required mode. Deprecation adds migration/removal evidence; it does not make
  a still-supported mode optional.
- `experimental` features require runnable evidence in every declared
  supported mode and structured negative evidence for each declared limit or
  unsupported mode.
- `planned` features are not presented as current or usable. Their Book
  coverage is limited to an explicitly planned section, justified
  illustrative material, or a runnable structured-rejection example for an
  already parseable surface; a planned feature cannot carry successful
  execution evidence as if it were available.
- `removed` features retain a tombstone plus replacement/removal rationale and
  migration or structured-rejection evidence. They do not silently disappear.

Status is forward-only:

```text
planned -> experimental -> public -> deprecated -> removed
```

Reviewed forward shortcuts are allowed; backward demotion is not. A material
restart after removal receives a new `feature_id`, while the old identity
remains a tombstone. Bootstrap status is derived from the exact candidate
source, Book, release, and current-design evidence. Only a feature proven never
current may begin as `planned`. Ambiguity is a blocking inventory gap. A
previously current or currently broken feature cannot be relabeled `planned`
to escape its runnable evidence; it remains at its current/deprecated status
until repaired or explicitly removed.

The manifest records coverage obligations and points to semantic authority. It
does not duplicate type checking, runtime contracts, ADR text, or Book prose.

Adding or changing a public feature changes this manifest in the same slice.
Removing an entry requires an explicit removal or supersession transition;
deleting it to make a coverage gate green is forbidden.

A `feature_id` is permanent and is never reused for a different meaning.
Changing an identifier is an identity migration: the old identifier becomes a
tombstone naming the replacement. A manifest schema-major change carries a
complete old-to-new identity migration map, including removals; it cannot
reinterpret an existing identifier in place.

The manifest carries the status basis and the feature-identity migration
records needed for validation, but not exact source revisions or mutable
verification state. `BookTruthGate` compares the candidate manifest with the
previous accepted manifest and rejects a missing prior identity, backward
status transition, reused identity, incomplete migration, or ambiguous
bootstrap status.

Every public language construct, annotation, standard-library callable or
type, compiler-visible user behavior, CLI workflow, LSP behavior, execution
provider surface, snapshot/resume operation, and distributed operator workflow
must have an entry. Families may share documentation, but no member may be
absent from the inventory.

### 3. shape-web owns the BookCoverageManifest

The shape-web repository owns one canonical `BookCoverageManifest`. It maps
every non-removed `feature_id` to:

- owning Book pages and stable section identities;
- stable executable fence identities;
- positive, negative, and failure/diagnostic evidence;
- covered VM/JIT/provider/foreign/snapshot modes;
- the required semantic dimensions from `PublicFeatureManifest`;
- any deliberately illustrative material and its reason/authority citation;
  and
- total feature, section, and fence tombstones and schema-major identity
  migration records.

The coverage manifest may not invent a feature identity or weaken its required
dimensions. A public feature with no coverage entry, an unknown coverage
feature, or a required dimension with no owning section is a gate failure.

The coverage manifest contains no exact Shape or shape-web revision, counterpart
hash, attestation digest, or mutable “last verified” field. Exact candidate
revisions and all current evidence hashes belong only to the external
`PairCandidate`, adapter reports, and `PairAttestation`. This avoids reciprocal
and self-referential source commits.

Coverage is set-based, not count-based. Stable section and fence identities do
not contain line numbers or ordinal positions. Moving prose must not look like
removing and adding a feature; silently substituting one uncovered feature for
another at the same count must not pass.

Section and fence identities are likewise permanent and never reused.
Changing one creates an explicit tombstone/replacement mapping. A
BookCoverageManifest schema-major change supplies a complete identity migration
map so old reports and coverage cannot silently attach to new material.
The map is total over the prior feature, section, and fence identity sets and
marks every identity unchanged, replaced, or removed. The gate rejects missing
rows, accidental many-to-one reuse, and an old identity attached to new
meaning.

Complete inventories may be implemented in bounded waves only after their
exact stable rows and content hash are committed. The concrete wave breakdown
requires user ratification. An inventory remains open until every row has
exactly one child owner, all child and capstone native dependency edges exist,
prose blockers match those edges, and a tracker re-fetch plus graph audit
passes. A family label or estimated count is not a publishable wave.

### 4. Every distributed feature documents the complete semantic matrix

Any feature that performs, configures, observes, persists, or recovers
distributed execution must document every applicable row below:

| Dimension | Required public explanation and evidence |
|---|---|
| Invocation | Exact callable/argument/result surface and transparent versus recoverable projection |
| Effects and permissions | `Remote(ResolvedProviderIdentity)`, suspension, provider grants, destination/network authority, and refusal |
| Provider lifecycle | Provider selection, immutable generation, configuration, credential ownership, reload, and rebinding |
| Discovery and admission | Placement suitability, ABI/capability negotiation, admission lease, Call Entry Commit, and frame-lifetime placement authority |
| Execution certainty | Completed, settled failure, confirmed cancellation, DefinitelyNotExecuted proof, OutcomeUnknown, and why transport cause is not certainty |
| Ownership transfer | `TransferId`, inaccessible outbound escrow, Call Entry Commit, owned results, duplicate delivery, and restoration conditions |
| Retry | Replay evidence, idempotency/deduplication, total-attempt and deadline budget, cleanup-before-retry, and transport retransmission versus a new attempt |
| Time and cancellation | Discovery/connect/backoff/execution/reply deadline consumption, cancellation request versus confirmed cancellation, and partition behavior |
| Recovery | Linear Recovery Obligation, durable supervisor acceptance outcomes, transfer receipt, permanent uncertainty, settlement, and abandonment |
| Cleanup | Teardown-capability closure, cleanup evidence, remote finalization, and behavior on failure/cancellation |
| Persistence | Recovery Journal, snapshot barriers, restore/rebind rules, and journal/wire/snapshot version refusal |
| Compatibility | Execution ABI, wire protocol, artifact, schema, provider generation, and old/new peer outcomes |
| Security | Authentication, grants, secrets, escrow confidentiality, untrusted peers, and least-authority operator actions |
| Observability | Attempt/transfer identity, certainty, budget, lease, obligation age, escrow size, settlement, and structured diagnostics |
| Operations | Start/configure/inspect/recover/quarantine/settle procedures, storage/backpressure, crash recovery, and safe escalation |
| Degraded modes | Unsupported targets, clean fallback/refusal, unavailable supervisor/provider, storage exhaustion, and corrupted state |

One section may satisfy several rows and one row may span pages, but the
coverage manifest records the exact mapping. A happy-path `@remote` example is
not comprehensive distributed documentation.

Operator procedures are part of the public feature when a correct program can
enter a state that requires operator action. They must distinguish observation
from authority: a log or dashboard never constitutes settlement, revocation,
acceptance, or non-execution proof.

### 5. Book fences are explicitly classified

Every Shape code fence has a stable explicit identity and one classification:

- `runnable-gated`, with required modes and expected value, output, or
  diagnostic; or
- `illustrative-only`, with a nonempty reason and issue or semantic-authority
  citation.

Unclassified fences fail extraction. The exact illustrative set is ratcheted;
adding an illustrative fence or removing its reason requires explicit review.
An example is not illustrative merely because the implementation is broken.

VM/JIT parity means two real executions. A snapshot/resume fixture passes the
selected mode through both its initial and resumed execution. A harness cannot
clone, infer, or relabel one mode's result as another.

JIT mode proves semantic behavior under that mode, not necessarily native
execution. A fence claiming native execution must name the exact function or
realization and carry structured evidence of native installation, subsequent
native dispatch, zero covered fallback/deoptimization, and VM/JIT equality.

Negative examples assert a structured diagnostic identity and essential typed
payload, not only a mutable rendered sentence.

### 6. BookTruthGate is one versioned cross-repository interface

shape-web owns one versioned `BookTruthGate` command-line interface:

```text
book-truth-gate
  --shape-bin <path>
  --shape-sha <sha>
  --shape-web-sha <sha>
  --public-features <manifest>
  --book-coverage <manifest>
  --report <path>
```

The command:

1. validates both manifests, forward-only status, total identity migrations,
   and their complete bidirectional mapping against the previous accepted
   pair;
2. extracts the full Shape-fence universe;
3. rejects missing identities, classifications, reasons, or required coverage;
4. executes every runnable fence in every declared mode;
5. validates expected values and structured diagnostics;
6. validates native evidence where claimed; and
7. emits one machine-readable report binding the manifests, exact repository
   revisions, binary hash, harness version, toolchain, environment, and all
   per-fence results.

Exit success means:

- every non-removed public feature has complete required Book coverage;
- every coverage entry names a real public feature;
- every required semantic dimension is mapped;
- every runnable-gated fence passed every declared mode;
- every native claim has non-vacuous native evidence; and
- every illustrative-only fence remains explicitly justified.

A percentage threshold, curated subset, unchanged count, or report from an
uncommitted harness cannot satisfy the gate.

### 7. Exact pairs are promoted through external attestation

One immutable `PairCandidate` names exact Shape and shape-web revisions plus
the manifest, gate/harness, binary, and toolchain identities to verify. It is
stored on a protected coordination ref or release outside both named source
revisions. The Shape and shape-web required-check adapters consume the same
candidate and independently run the same BookTruthGate over that exact pair.
Neither adapter follows a moving branch or reads a reciprocal counterpart SHA
from either source revision.

After both reports agree, the coordinator emits one signed,
content-addressed `PairAttestation`. It binds the candidate, both reports,
exact revisions and content hashes, workflow identities, gate/environment
versions, and result. The attestation is immutable, contains no promotion
generation, and is stored outside the two named source revisions.

Promotion creates a signed, content-addressed `AcceptedPairTransition` outside
both source revisions. It names a monotone generation, the expected previous
transition digest, the selected attestation digest, `promote` or `rollback`,
actor, reason, policy identity, and nonce. A compare-and-swap updates the
protected accepted-pair pointer only from that expected predecessor. Every
transition is retained in an append-only audit. The selected transition is the
sole authority for the current pair; repository branches are staging and
history surfaces, not acceptance authority. A one-sided source merge cannot
make a cross-repository feature current.

A one-repository change pairs its candidate with the exact accepted counterpart.
A cross-repository feature stages and verifies both candidate revisions
together. Rollback creates a new, higher-generation transition selecting a
previously valid attestation; it never moves the generation backward or rewrites
an attestation or prior transition. Stale predecessors, concurrent writers, and
replayed transitions are rejected. Source rollback uses reviewed revert
revisions and a new candidate/attestation.

`BOOK-PAIR-PROMOTE` is the explicit native blocker for #90 and every later
ticket that changes public behavior. #90 is a capstone and blocks none of the
Book-bootstrap children. Once bootstrap promotion exists, each public ticket
must still land its documentation/manifests and promote a new exact-pair
attestation as its own acceptance evidence.

### 8. #23 is a capstone verifier, not a documentation phase

Every implementation ticket owns its documentation, coverage-manifest update,
and executable examples. Incremental feature gates use the same full
BookTruthGate, so a slice cannot regress unrelated public documentation.

#23 runs only after the typed surface and legacy deletion program is otherwise
complete. It verifies:

- complete feature inventory, including all pre-existing public features;
- complete feature-to-Book and Book-to-feature mapping;
- complete distributed-semantics matrices;
- zero unclassified fences;
- the exact enumerated illustrative set;
- 100% success for runnable-gated fences in every declared mode;
- current status/index language and no stale superseded mechanism presented as
  current; and
- a reproducible report from committed, exact Shape and shape-web revisions.

#23 may repair a defect it discovers, but it is not the scheduled owner for
documentation knowingly deferred by an earlier feature ticket. The capstone
should primarily verify evidence already landed with those features.

### 9. Coverage changes are reviewable semantic changes

A review of a feature change must be able to answer:

- which public feature identities changed;
- which Book sections and fences now cover them;
- which semantic dimensions were added, removed, or declared inapplicable;
- which executable modes and expected outcomes changed; and
- whether any illustrative exception or distributed operational burden grew.

Manifest diffs provide that view. Generated prose indexes and status tables
may project the manifests, but they do not become independent authority.

Deprecation documentation remains gated until the removal release no longer
claims the feature public. Removed features retain a tombstone mapping to their
replacement or removal rationale so stale Book coverage cannot silently
reattach to a new meaning.

### 10. Script-tier coverage and ceremony budgets are gated dimensions (proposed amendment 2026-07-27)

Shape's ergonomic contract — script-feeling entry-level code over a strict
core — is a public feature and therefore receives the same executable-evidence
treatment as any other public behavior. ADR-017 defines the language rules;
this section fixes their manifest and gate representation.

`PublicFeatureManifest` gains the semantic dimension `script-tier` and the
evidence class `ceremony-budget`. Because no accepted manifest pair exists
yet, this is a prepublication revision of the draft contracts owned by
`PF-CONTRACT` and `BOOK-CONTRACT`; it triggers no schema-major identity
migration. After the first accepted pair, changing either enum follows the
ordinary schema-migration rules of §2 and §3.

A feature whose entry lists the `script-tier` dimension must map it to at
least one `runnable-gated` fence whose fence metadata declares
`ceremony: none`. The gate verifies that declaration mechanically: the
compiler reports, as a structured fact, the count of explicit effect-row
declarations, ownership or linearity annotations, and capability ceremony the
fence required; `ceremony: none` passes only when that count is zero.
Reviewer judgment does not substitute for the mechanical check, and a fence
cannot satisfy the dimension by moving its ceremony into hidden setup code:
the fact covers the whole compiled fence unit.

`BookCoverageManifest` fence entries may declare a ceremony budget:
`max_lines` and `max_explicit_annotations`. A designated flagship-fence set —
enumerated and ratcheted exactly like the illustrative-only set of §5 — must
declare budgets. The gate fails on exceedance. Tightening a budget is an
ordinary change; loosening one, removing one, or removing a fence from the
flagship set requires explicit review with a nonempty reason, and the change
is part of the reviewable manifest diff of §9.

Structured diagnostics may carry a stable concept identity (owned by the
diagnostic catalog, not by the Book). The coverage manifest maps each concept
identity referenced by gated negative fences to the Book section that teaches
it; an unmapped concept identity in gated evidence is a coverage failure.
Concept identities follow the same permanence and tombstone rules as feature
identities.

Budgets are acceptance gates for designated flagship documentation, not a
general style linter. They do not apply to fences outside the flagship set,
and they measure the documented user experience, never internal
implementation size.

## Consequences

- Public documentation work is distributed across feature slices instead of
  concentrated in a late cleanup wave.
- Shape owns the feature inventory; shape-web owns how the Book satisfies it.
- The full Book becomes executable acceptance evidence without pretending that
  every explanatory fragment is runnable.
- Distributed features must explain the failure, ownership, security, and
  operator model, not only invocation syntax.
- Cross-repository reports become reproducible because they bind exact commits
  and the tested binary.
- Feature tickets become somewhat larger, but their public behavior is
  reviewable and usable when they close.

## Rejected alternatives

- **Document features in #23 after implementation.** This allows architecture,
  diagnostics, and examples to drift until the engineers with current context
  have moved on.
- **Use the Book alone as the feature inventory.** Missing features are
  invisible if the documentation repository defines its own denominator.
- **Keep only a pass percentage.** The denominator can shrink or hard examples
  can be reclassified without demonstrating comprehensive coverage.
- **Treat every fence as runnable by default.** Unreviewed classification hides
  whether prose is illustrative, incomplete, or intended as executable truth.
- **Allow `runnable=false` without a reason.** A broken implementation can be
  hidden as documentation.
- **Run only shape-web CI.** Shape runtime changes would not gate the Book.
- **Test shape-web against Shape's moving main branch.** The report would not
  identify the implementation revision whose behavior it claims.
- **Call JIT mode native evidence.** Interpreter fallback can preserve output;
  native claims require actual dispatch evidence.
- **Document distributed calls only from the programmer's happy path.**
  Operators and owners still face uncertainty, escrow, deadlines, recovery,
  compatibility, and security states.
- **Duplicate semantic contracts in documentation manifests.** The manifests
  map coverage to authority; they do not replace typed compiler/runtime facts
  or ADRs.

## Related decisions

- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms
- ADR-013: Incremental Semantic Queries and Tracked Comptime
- ADR-014: Closed Effects and Static Capability Ownership
- ADR-015: Recovery Episodes and Durable Obligation Journal
