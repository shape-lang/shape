# ADR-015: Recovery Episodes and Durable Obligation Journal

## Status

Accepted (2026-07-27)

Clarifies ADR-010, ADR-012, and ADR-014. In particular, a
`RecoveryObligation` is linear, not affine: every ordinary control-flow path
must settle it, return it, or transfer it with durable acceptance.

Proposed amendment 2026-07-27 (pending ratification): §8 gains a version
lineage clarification and §10 defines obligation batches for fan-out
settlement. §10 supersedes the current placeholder `join settle` runtime
behavior and the stale in-code claim that it returns per-task status values.

## Context

ADR-010 makes cleanup evaluator-owned and total over semantic outcomes.
ADR-012 adds typed failure handling, replay authority, recovery budgets, and
remote outcomes that may retain inaccessible ownership escrow. ADR-014 makes
the corresponding capabilities statically affine or linear.

Those decisions leave two temporal and durability questions that must have one
answer across the compiler, evaluator, snapshots, VM, JIT, and distributed
execution:

1. A failure handler runs before teardown, but retry may begin only after
   teardown. User code therefore cannot require a cleanup-completion token in
   order to express its retry request.
2. A process can fail while handing an uncertain remote attempt to a durable
   supervisor. Consuming an in-memory obligation before its receipt is durable
   could lose the only recovery owner.

Retry and remote recovery are related but distinct. Retry authorizes a new
semantic attempt. A recovery obligation owns unresolved state for an attempt
whose execution or ownership outcome is not yet settled. Neither a call ID,
timeout, transport retransmission, log message, nor retry policy can substitute
for the required evidence.

## Decision

### 1. One Recovery Episode owns all attempts, budget, and history

A `RecoveryEpisode` begins with the initial attempt and ends when the
invocation completes, recovers, propagates a terminal failure, is cancelled,
or transfers every unresolved obligation to a durable supervisor.

The episode owns:

- one stable episode identity;
- an append-only attempt history;
- the parent recovery budget;
- the effective argument pack and hook state for each attempt;
- the replay evidence selected for any proposed new attempt;
- cleanup-completion evidence for finished attempts; and
- any recovery obligations that remain linear owners of uncertain state.

User code cannot drive the episode loop. It can select a typed failure intent
and may narrow the budget, but only the evaluator starts an attempt, records
its outcome, completes its teardown, or commits a retry.

The initial invocation is attempt one. A retry is always a new semantic
attempt, never a transport retransmission or continuation of the failed frame.

### 2. FailureIntent precedes cleanup; Retry Commit follows cleanup

A typed failure-handler fold produces exactly one linear
`FailureIntent<R, Sig>`:

```text
FailureIntent<R, Sig> =
    Propagate(RuntimeFailure)
  | Recover(R)
  | Retry(RetryIntent<Sig, Scope, Attempt>)
```

`RetryIntent` is a request, not execution authority. It owns:

- an exact replay-safe `ArgumentPack<Sig>`;
- `NotExecutedProof<Sig>` or
  `ReplayEvidence<Sig, Scope, Attempt>`;
- a typed post-cleanup backoff plan, if any; and
- the provenance of the failed attempt and selecting handler.

Constructing the intent must prove that its argument pack is disjoint from the
failed attempt's teardown closure or can be reconstructed from the replay
evidence. Moving an owner into the intent removes it from the failed attempt;
the same owner cannot remain available to cleanup.

The evaluator completes the entire failed attempt before acting on a retry
intent:

```text
failure-handler fold
  -> linear RetryIntent
  -> teardown of every activated layer
  -> CleanupComplete<Episode, Attempt>
  -> post-cleanup backoff
  -> budget/deadline check and attempt-permit consumption
  -> evaluator-private Retry Commit
  -> one fresh Next<Sig>
  -> next attempt
```

`CleanupComplete<Episode, Attempt>` is evaluator-minted linear evidence for
one exact attempt. It cannot be supplied to user code, substituted across
episodes or attempts, or fabricated from a cleanup result.

A Retry Commit is evaluator-private. It consumes the retry intent, cleanup
evidence, replay evidence, and one attempt permit and authorizes exactly one
new attempt. Only then does the evaluator mint the new attempt's single
`Next<Sig>`. No source-visible operation returns a fresh `Next`.

If cleanup does not complete, the deadline expires, the budget is exhausted,
or replay evidence no longer covers the effective pack and effects, no Retry
Commit exists. The evaluator produces a structured outcome that retains the
original failure and the denial evidence. It never starts a best-effort
attempt.

If the retried attempt fails, the same failure-handler layer is invoked again
with the effective pack, updated hook state, attempt history, remaining budget,
and evidence for that failure.

### 3. RecoveryBudget has total-attempt and absolute-deadline bounds

Every replay-enabled episode has both:

- `max_attempts`, including the initial attempt; and
- one absolute deadline shared by discovery, connection, admission, backoff,
  execution, cancellation, and reply waiting.

Default execution has `max_attempts = 1` and no replay authority. It physically
cannot commit a retry.

The source form `@retry(3)` means at most three re-executions and therefore
lowers to `max_attempts = 4`. Diagnostics, artifacts, and tooling name the
source quantity `retries` and also expose the resulting total-attempt bound.
An explicit duration narrows the enclosing deadline. If retry is requested and
neither an enclosing contract nor the retry declaration supplies a finite
deadline, contract elaboration rejects it.

Hooks and providers may narrow the remaining attempt or deadline bounds by
taking the stricter value. They cannot reset an attempt count, start a
provider-local budget, or extend time. The attempt permit is consumed only
when the evaluator commits the next attempt; deadline expiry during backoff
starts no attempt.

Within one process, elapsed-time enforcement uses a monotonic clock. Durable
state also records an absolute deadline and the last observed remaining
duration. Restore or migration adopts the stricter remaining bound so wall
clock movement cannot extend the episode.

Backoff and jitter are ordinary typed callables invoked by the evaluator after
cleanup. Their clock, randomness, suspension, and other effects are part of
the effective callable contract.

### 4. A Recovery Obligation is a linear handle to durable state

A `RecoveryObligation` exists only after the runtime has durably recorded the
state it owns. It is a linear handle to one journal entry, not a detachable
in-memory record.

Before remote dispatch can make owned inputs inaccessible, the sender durably
records:

- the stable `TransferId`;
- exact invocation, artifact, lifecycle ABI, placement, and provider
  generation identities;
- canonical payload or content references;
- ownership roles, semantic ranks, schemas, and cleanup state;
- the complete escrow inventory; and
- the current recovery owner and state-machine generation.

Escrow blobs are durable before a journal transition may refer to them.
Live credentials, sessions, pointers, routes, capability leases, and provider
handles are never serialized into escrow.

The sender-side journal has these monotone states:

```text
PreparedOutbound
  -> InFlight
  -> Settled
     | OutcomeUnknown

OutcomeUnknown
  -> SupervisorOwned
  -> Settled
```

The receiver records one of:

```text
Absent
  -> RejectedBeforeCommit
     | CallEntryCommitted

CallEntryCommitted
  -> Settled
```

The journal may retain more detailed substates, but they cannot weaken this
algebra or create a path that restores sender ownership after Call Entry
Commit. Every transition is monotone, generation-checked, and idempotent by
`TransferId`. A duplicate transmission or recovery action replays the recorded
transition or receipt.

`DefinitelyNotExecuted` requires a durable receiver
`RejectedBeforeCommit` proof for the same transfer and generation. Timeout,
disconnect, process restart, provider assertion, or lease expiry alone leaves
the outcome uncertain.

### 5. RecoveryJournal is the single durable ownership authority

`RecoveryJournal` owns the state machine, escrow inventory, acceptance,
settlement, crash recovery, and compaction. Remote Dispatch, snapshots,
operator tooling, and supervisors consume this authority rather than
maintaining parallel recovery state.

Journal v1 is a versioned append-only log. Every frame binds:

- format magic and version;
- frame length and checksum;
- `TransferId`, prior generation, and next generation;
- the canonical transition and its bound identities; and
- references to already-durable escrow content.

The local journal adapter must:

- persist referenced blobs before the transition that names them;
- persist the transition before publishing its receipt;
- synchronize new directories and files according to the host durability
  contract;
- hold an exclusive writer epoch so a stale process cannot append;
- accept and truncate an incomplete final frame during recovery; and
- quarantine interior corruption or a non-monotone transition rather than
  guessing.

A valid-prefix recovery must reconstruct exactly one owner for every unsettled
transfer. Compaction preserves the latest authoritative state and referenced
escrow before retiring old frames. Storage exhaustion, permission failure, or
corruption never fabricates acceptance or settlement.

The journal exposes structured inspection of unsettled transfer identity,
state, age, placement, provider generation, escrow inventory, and next legal
actions. It does not expose owned values or secrets as ordinary logs.

### 6. DurableSupervisor is sealed and acceptance is exhaustive

`DurableSupervisor` is a sealed host capability. Shape code may receive and
select a granted supervisor, but ordinary user code cannot implement the
durability contract or mint a `TransferReceipt`.

Its conceptual acceptance surface is:

```text
accept(RecoveryObligation) -> AcceptOutcome

AcceptOutcome =
    Accepted(TransferReceipt)
  | Refused(RecoveryObligation, SupervisorError)
  | AcceptancePending(PendingAcceptance)
```

`Accepted` is returned only after the journal's `SupervisorOwned` transition
and its escrow references are durable. The receipt binds the transfer,
artifact, placement, provider generation, inventory, journal generation, and
new recovery owner. Repeating acceptance for the same committed transfer
returns the same receipt.

`Refused` returns the still-linear obligation to its caller. No ownership
handoff occurred.

`AcceptancePending` is the fail-closed result of an ambiguous storage outcome.
It carries a linear handle for the same transfer and may only be resolved,
returned, or handed to another durable recovery owner. It does not permit
retry, escrow restoration, or assumption that acceptance failed.

An accepted receipt releases the caller from owning the uncertain recovery
work; it does not claim that the remote attempt itself succeeded. The
supervisor remains responsible until the journal records a settled outcome or
typed terminal abandonment evidence.

### 7. Admission expiry and frame-lifetime placement authority are distinct

An `AdmissionLease` is receiver-minted, non-serializable, and expiring. It
authorizes one admitted execution to cross Call Entry Commit before its
deadline.

Call Entry Commit consumes the admission lease and mints a
`PlacementCapabilityLease`. That lease pins the verified teardown-capability
closure and provider generation for the entire frame lifetime. It does not
expire merely because wall time passes. Same-host deoptimization retains it;
cross-placement migration must prepare and admit the destination before
fencing and transferring the source.

If the admission lease expires before Call Entry Commit, the receiver may
produce a durable `RejectedBeforeCommit` proof. If execution may already have
entered, expiry is not revocation. Terminating or recovering an entered
placement requires explicit isolation-revocation or settlement evidence.

### 8. Journal, wire, and snapshot formats version independently

Recovery Journal v1, wire protocol v3, and snapshot format v8 are separate
compatibility domains. Advancing one does not silently advance or validate the
others.

Wire v3 introduces a fixed outer envelope and compatibility handshake parsed
before any version-specific payload. Ownership-aware Remote Dispatch is sent
only after both peers accept v3 and the exact execution ABI. A v2 peer rejects
before user code or ownership transfer and yields version-mismatch
`DefinitelyNotExecuted` evidence. Recovery-obligation, escrow, and lease fields
are v3-only; they are not defaulted when absent.

Snapshot v8 begins with a fixed magic, version, length, and checksum header
parsed before version-specific snapshot decoding. A v8 runtime refuses v7 and
a frozen v7 reader refuses v8 before either trusts a changed payload. Live
obligations persist journal identities, generations, receipts, deadlines, and
restorable escrow references; they do not duplicate the journal owner or
serialize live provider capabilities.

The implementation sequence is binding:

1. land the outer wire-v3 envelope, handshake, and old/new refusal tests;
2. land Recovery Journal v1 and its state algebra;
3. add v3 ownership/recovery payloads and build Remote Dispatch on them;
4. stabilize the runtime obligation and lease fields; and
5. introduce snapshot v8 and its cross-version refusal fixtures.

No compatibility path may use a skipped field, `serde(default)`, dynamic map,
or attempted deserialization of the new payload in order to discover its
version.

Version lineage (clarified 2026-07-27): wire v1 and wire v2 both exist in the
shipped tree (`WIRE_PROTOCOL_V1`/`WIRE_PROTOCOL_V2`,
`crates/shape-wire/src/lib.rs:56`/`:60`); v2 added Execute/Validate/Auth/Ping
messages and JSON framing and is served by `serve`, while the legacy
`wire-serve` command still reports v1 — a known reporting split to be
resolved by the wire-v3 slice. Historical planning documents separately used
"wire protocol v2" to mean typed serialization
(`crates/shape-vm/src/V2_STAGE6_GATE.md`); that conflicting usage is retired
and must not be cited as protocol lineage. Wire v3 is therefore the next free
protocol number and the first ownership-aware envelope, exactly as this
section specifies.

### 9. Verification is state-machine and crash based

Acceptance requires more than a successful retry example.

Retry verification covers:

- the exact failure-handler, cleanup, backoff, commit, and re-entry order;
- exactly one cleanup for every activated layer and attempt;
- cross-attempt and forged-evidence rejection;
- ownership disjointness between retry packs and teardown;
- total-attempt and deadline exhaustion;
- cancellation and cleanup failure before commit; and
- identical VM/JIT attempt, state, cleanup, and outcome traces.

Journal verification injects a crash or ambiguous error before and after every
blob write, frame write, synchronization, receipt publication, settlement,
and compaction boundary. Restart must recover exactly one owner and either the
prior or next complete transition, never a fabricated intermediate state.

Distributed verification covers duplicate delivery, duplicate acceptance,
receiver restart, sender restart, partition, stale writer epoch, lease expiry,
wrong-generation rejection proof, permanent uncertainty, and eventual
settlement. Owned test values have observable finalization so duplicate
restoration or silent loss cannot pass vacuously.

Operational acceptance includes bounded storage/backpressure and structured
metrics for pending obligations, oldest age, escrow bytes, acceptance state,
settlement outcome, corruption, and recovery failure.

### 10. Obligation batches aggregate fan-out uncertainty (proposed amendment 2026-07-27)

Fan-out is the canonical distributed pattern, and its results are structurally
linear: N remote calls may each end `Uncertain` carrying a
`RecoveryObligation`. ADR-014 rejects dynamic collections of affine or linear
values in v1 because their element accounting is not structural. This section
resolves the tension without weakening the verifier: aggregation is
evaluator-owned, and the user-visible aggregate is one linear value whose
element accounting lives in the journal, which already guarantees exactly one
owner per unsettled transfer. Because the join's branch list is statically
known, `ObligationBatch<Branches>` is a fixed product over that list —
already permitted by ADR-014 §4 ("fixed products and enums may contain
capabilities") — not a carve-out from the dynamic-collection ban.

`join settle` over branches whose outcomes can be uncertain produces:

- one typed, per-branch settled outcome for every branch that reached a
  settled state — completion value, settled failure, confirmed cancellation,
  or proven non-execution, typed per branch (branch types need not be
  homogeneous); and
- at most one `ObligationBatch`: a single linear handle aggregating every
  unresolved obligation from the join. A branch whose outcome entered the
  batch is marked by a typed `BranchRef` into it (defined below), never by
  an independently owned linear value in the branch slot.

`ObligationBatch<Branches>` rules:

- it is one linear value indexed by its join's branch list; the ownership
  verifier accounts for it as a whole, and user code never holds a dynamic
  collection of individual obligations;
- each aggregated obligation remains its own journal entry with its own
  `TransferId`, generation, and escrow inventory; the batch is a handle over
  that set, not a merge of it. The journal frame algebra of §5 is unchanged:
  batching is an in-memory, episode-scoped grouping. Restore does not
  persist the grouping — a valid-prefix recovery reconstructs per-transfer
  owners per §5, and the evaluator re-forms one fresh batch over the
  episode's unsettled entries. Splits are likewise in-memory; a crash
  mid-split is harmless because entry-level ownership never depended on the
  grouping;
- typed drain uses unforgeable branch descriptors,
  `BranchRef<Branches, I, R>`, mirroring `ArgumentPack<Sig>`'s
  signature-indexed discipline (ADR-012 §4): a settled entry drains to its
  exact branch type `R` through its descriptor. `BranchRef` is affine — a
  branch cannot be drained twice — and holding a `BranchRef` does not make
  the per-branch settled product linear;
- only sealed consumers selected by resolved intrinsic identity (ADR-014 §5)
  operate on the batch: whole-batch supervisor transfer, settlement waiting,
  drain, and explicit split into two batches partitioning the entry set.
  The batch is consumed exactly once: when it is empty, fully transferred,
  or returned;
- whole-batch transfer has an exhaustive typed outcome:
  `BatchAccepted(BatchReceipt)` — legal only after an atomic multi-entry
  `SupervisorOwned` transition whose per-entry preconditions are
  generation-checked, with the batch receipt binding every per-entry
  receipt — or `BatchRefused(ObligationBatch, SupervisorError)`, returning
  the still-linear batch, or
  `BatchAcceptancePending(PendingBatchAcceptance)`, the fail-closed
  ambiguous-storage result that must resolve at the same supervisor per
  §6's rules. An entry that settles while transfer is being prepared is
  removed from the batch before the transition — its settled result remains
  drainable — so the multi-entry transition never covers a settled entry
  and a supervisor never "recovers" completed work;
- settlement waiting on a batch requires a finite deadline: contract
  elaboration rejects an unbounded batch wait exactly as §3 rejects replay
  without a deadline. Deadline expiry settles nothing (§4's rule stands);
  it returns the still-linear batch to the waiter with structured timeout
  evidence. On frame cancellation, the batch's consumer on the `Cancelled`
  edge is the evaluator: it transfers the batch to the enclosing recovery
  owner — the nearest supervisor scope, or the episode's fail-closed
  recovery-pending state — so no reachable outcome edge leaves the batch
  unconsumed;
- the batch is journal-backed, not a suspended future: snapshot treats its
  entries by §8's rules — journal identities, generations, and receipts
  persist — and the current opaque task-group carrier (unserializable,
  value-free) is superseded by this design.

`join all` over branches whose contracts admit uncertainty is a compile
error whose diagnostic names `join settle`, unless the contract proves
uncertainty impossible per ADR-012 §7's `Result` projection rule.
Compile-time obligation tracking for batches extends the existing
checked-body emission authority for drop obligations across suspension
points — the mechanism that already fail-closed rejects unsettled
obligations at suspension and names `MustSettle` as its planned
relaxation — rather than introducing a parallel tracker.

Grounding (2026-07-27): today's `join settle` drops every task result and
returns an opaque id bag with a fresh type variable
(`crates/shape-vm/src/executor/async_ops/mod.rs:698`); per-task outcomes are
reachable only from Rust through the `get_result` accessor
(`crates/shape-vm/src/executor/task_scheduler.rs:313`), and the in-code
comment promising a `{status, value/error}` array
(`executor/task_scheduler.rs:445`) is false in shipped code. The surface
defined here is therefore greenfield; the single placeholder assertion
(`tools/shape-test/tests/async_concurrency/join_strategies.rs:289`)
is deleted with the implementing slice. The emission authority to extend is
`crates/shape-vm/src/compiler/checked_body/async_drop_context.rs`.

## Consequences

- Failure handlers remain ordinary typed code while cleanup-before-retry becomes
  structurally true.
- Retry authority, cleanup evidence, and attempt permits cannot be confused or
  reused across attempts.
- A remote outcome can remain uncertain without losing the one recovery owner.
- The local v1 supervisor is operationally substantial: it is a durable
  ownership subsystem, not an append call around the snapshot store.
- Wire and snapshot compatibility become explicit refusal contracts rather
  than field-defaulting conventions.
- Long-running admitted frames do not lose cleanup authority because a
  pre-entry deadline elapsed.

## Rejected alternatives

- **Expose `retry(CleanupComplete, pack)` to a failure handler.** The handler
  runs before the evaluator can mint cleanup evidence.
- **Let Retry contain multiple attempts.** That hides the evaluator loop,
  per-attempt cleanup, and budget consumption.
- **Count only retries internally.** Total-attempt bounds are less ambiguous;
  source retry counts can lower explicitly.
- **Accept an obligation with a one-way infallible function.** Storage failure
  or process death could consume the only linear owner without a receipt.
- **Reuse snapshot files as the obligation log.** Current snapshot persistence
  does not define the transactional owner transitions, synchronization, or
  idempotence a recovery journal requires.
- **Treat lease expiry as non-execution proof.** An entered receiver may still
  own and execute the attempt; expiry without fencing proves nothing about it.
- **Add recovery fields to the existing unversioned message payload.** Old and
  new peers could disagree after ownership became inaccessible.
- **Restore absent obligation fields with defaults.** Missing ownership
  authority must cause refusal, never fabrication.

## Related decisions

- ADR-010: Verified Region Teardown and Callable Lifecycle
- ADR-011: Resolved Semantic Identity and Typed Elaboration
- ADR-012: Verified Annotation Elaboration and Callable Transforms
- ADR-013: Incremental Semantic Queries and Tracked Comptime
- ADR-014: Closed Effects and Static Capability Ownership
