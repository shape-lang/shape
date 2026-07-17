# Verified Region Teardown and Callable Lifecycle: Implementation Plan

Status: planning only. This document sequences the accepted architecture into
small, reviewable increments. It does not authorize implementation before
ADR-009 is complete and its strict-typing artifacts are merged.

## 1. Goal

Build one general-purpose, typed lifecycle substrate that gives the VM, JIT,
cache, snapshot, and distributed runtime the same proof that every ownership
region and terminal frame exit gives each obligation one exact release,
transfer, or fault-containment disposition. The fast path lowers that proof to
direct code; it does not interpret a runtime cleanup list.

This is core execution infrastructure for collections, science, simulation,
stream processing, services, and finance libraries alike. Domain meanings such
as transaction settlement, exchange sessions, GPU synchronization, stream
unsubscribe, or simulation shutdown remain typed library finalizers. They do
not become language opcodes.

## 2. Hard boundaries

- ADR-009 completes first. This work consumes its strict callable/type/effect
  identities and must not edit, weaken, or race that ADR.
- The follow-on architecture ADR is the authority for semantics; this roadmap
  controls staging, not meaning.
- `ExecutionPolicy` exists for every execution and defaults to `Unbound`.
  Policy narrows scheduling choices; it never grants teardown capabilities.
- A missing, unsupported, malformed, or unverifiable teardown artifact never
  means an empty plan. The compiler recompiles from trusted source or refuses.
- Serialized bytes never carry execution authority. Only successful local
  admission mints `VerifiedRegionTeardownPlan`, resolved targets, and a
  placement lease.
- Backends consume the same verified plan. Neither VM nor JIT reconstructs
  ownership from slots, kinds, names, liveness, or emitted cleanup instructions.
- Ordinary scalar code has no plan lookup, dynamic action dispatch, allocation,
  armed bit, or cleanup branch at invocation time.
- Observable work follows Reverse Ownership-Entry Order. Only proven
  unobservable retirement may be reordered or batched.
- Structural Retirement only revokes source access and preserves the carrier;
  it is not Carrier Release. Finalization is optional semantic work; exact
  Carrier Release is mandatory representation retirement/deallocation.
  Finalizer failure cannot suppress later cleanup or replace the fixed primary
  outcome, and every terminal `Evaluation` preserves ordered Cleanup Evidence.
- Suspension and resumptive deoptimization transfer ownership and teardown
  state. Cancellation, abandonment deoptimization, and other terminal exits
  discharge it.
- `MustSettle` must settle, transfer, or return on every normal edge. Automatic
  finalization is fallback only on its declared failure/cancellation outcomes.

## 3. Current gaps and target seams

| Current source | Gap | Intended seam |
|---|---|---|
| `crates/shape-vm/src/mir/types.rs` and `mir/cfg.rs` | `Call` has only a success successor; lexical regions, unwind/cancellation/suspension outcomes, and ownership-entry ranks are absent. | Add compact stable region/owner/site/effect provenance and derive a `SemanticOutcomeEdgeGraph`; do not put backend cleanup blocks into source MIR. |
| `crates/shape-vm/src/mir/lowering/*` | Scope stacks primarily restore names; `?` and several effectful constructs lose lifecycle control flow before JIT analysis. | Emit region enter/leave provenance and exhaustive semantic outcome-site metadata during lowering. |
| `crates/shape-vm/src/mir/{solver,analysis,liveness,storage_planning,return_ownership}.rs` | Useful borrow/move/storage facts exist, but no fact alone authorizes teardown. | Feed exact facts into one post-analysis plan builder; keep existing analyses narrow and independently testable. |
| `crates/shape-vm/src/compiler/functions.rs` | MIR analysis and AST-to-bytecode drop emission are parallel paths; later closure/schema patches can change executable meaning. | Orchestrate a single late freeze after every semantic patch. Put new logic in focused modules rather than growing this legacy file. |
| `crates/shape-vm/src/bytecode/core_types.rs` | `MirFunctionData` is `serde(skip)` and therefore cannot authorize cached or remote execution. | Serialize a compact certificate and plan, never the original MIR, in each function artifact. |
| `crates/shape-vm/src/compiler/{mod.rs,compiler_impl_initialization.rs}` | `FunctionBlob` is finalized from bytecode fields without lifecycle evidence. | Require a frozen artifact before `FunctionBlobBuilder::finalize`; include it in canonical hash input. |
| `crates/shape-vm/src/bytecode/content_addressed.rs` and `crates/shape-vm/src/linker.rs` | Hashing covers current blob fields, while linking accepts ordinary deserialized blobs. | Add versioned lifecycle fields; admit blobs before linking; make the linker consume verified program artifacts and remap only derivative anchors. |
| `crates/shape-vm/src/bytecode/verifier.rs` and `executor/vm_impl/program.rs` | Current verification is narrow and warning-only at VM load. | Add a fail-closed lifecycle admission verifier. Existing opcode verification may remain separate, but new-ABI execution cannot continue after lifecycle failure. |
| `crates/shape-vm/src/{blob_cache_v2,bytecode_cache}.rs` | Cache reads deserialize values and blob cache keys use a bare content hash. | Separate untrusted byte storage from admitted entries and key admission by full `ArtifactKey`, verifier version, and action/carrier catalog hash. |
| `crates/shape-vm/src/compiler/*drop*`, `executor/trait_object_ops.rs`, and `executor/vm_impl/stack.rs` | User `DropCall` and raw carrier release are separate authorities; frame truncation walks numeric slots. | Derive VM epilogues from ordered Region Exit Recipes; use `truncate_stack` only as a low-level exact-release primitive where the plan proves its order and role. |
| aggregate implementations in `shape-value` and collection/table libraries | Aggregate child ownership follows physical representation or ad hoc release paths; there is no portable exact-child contract. | Validate sealed `AggregateLifecycleDescriptor`s and select specialized `AggregateTeardownKernel`s over declared current logical ownership order. |
| `crates/shape-jit/src/mir_compiler/{terminators,ownership}.rs` | normal `Return` and negative signal returns bypass total frame teardown; `emit_drop` does not reproduce user finalization. | Add plan-driven static epilogue lowering and route every terminal signal through an exit disposition. |
| `shape-value/src/v2/typed_array.rs`, VM call convention, and JIT `ffi/control/mod.rs` | Materialized callables may be inline function IDs, module IDs, or raw closure Arcs; dispatch and release depend on carrier shape. | Issue #58 supplies one owned raw first-class callable carrier and a prevalidated lifecycle capability. Direct calls remain unmaterialized. |
| `executor/async_ops/mod.rs` and `task_scheduler.rs` | Scope exit requests cancellation but does not prove termination/join before releasing borrowers. | Introduce affine Borrower Tokens and the sealed Scope Quiescence protocol. |
| `executor/osr.rs` and JIT deopt metadata | Live values are copied back, but teardown-plan identity and dynamic state are not transferred/reconstructed. | Carry plan hash, armed state, cursor, and logical-frame mappings through resumptive deopt; teardown only on abandonment. |
| `remote.rs` | Receiver recomputes blob hashes and permissions, but has no lifecycle admission, target resolution, or placement lease. | Extract receiver admission into a focused lifecycle module; keep `remote.rs` as orchestration. |
| VM/JIT/host result adapters | Cleanup failures and engine faults can lose the frozen primary result or travel through ambient/string channels. | Make `Evaluation<R>` the canonical typed terminal envelope with zero-allocation empty Cleanup Evidence and a flat evidence-preserving `EngineFaulted` variant. |
| `executor/{snapshot,vm_state_snapshot,time_travel}.rs` | Snapshots identify functions but do not bind dynamic teardown state to a verified plan hash. | Store only dynamic state plus exact artifact identity; refuse unsupported live obligations or mismatched restore. |
| `compiler/monomorphization/*` | Concrete specialization exists, while budget fallback can use a generic path with no lifecycle contract. | Check a parametric definition contract; freeze a concrete plan per executable specialization; require an explicit erased dictionary or refusal at fallback boundaries. |
| current AST/HOF inlining and JIT optimizer modules | Lifetime-changing transforms are not followed by a common teardown re-freeze. | Compose executable MIR and provenance before freeze; reverify the composite; permit true tail transfer only when the caller is empty and disarmed. |

Likely new focused modules are:

- `crates/shape-abi-v1/src/lifecycle.rs` for versioned portable descriptors,
  full canonical hashes, provider coordinates, and the sealed control algebra;
- `crates/shape-vm/src/mir/{regions,outcomes}.rs` for compiler provenance;
- `crates/shape-vm/src/teardown/{model,builder,certificate,verify,admission}.rs`;
- `crates/shape-vm/src/teardown/{aggregate,evidence,realization}.rs` for
  aggregate contracts, typed evidence, and executable refinement proofs;
- `crates/shape-vm/src/bytecode/lifecycle.rs` for artifact encoding/remapping;
- `crates/shape-vm/src/executor/{lifecycle,evaluation,scope_quiescence,
  remote_ownership}.rs`;
- `crates/shape-jit/src/mir_compiler/teardown.rs` and a focused deopt-lifecycle
  module;
- `crates/shape-value/src/v2/callable.rs` for the #58 carrier.

These names are planning targets, not a requirement to force abstractions before
the first tracer. New modules should remain below 500 lines; legacy files above
the maintainability limit receive narrow orchestration edits only.

## 4. Dependency spine

```text
ADR-009 accepted and merged
  -> portable lifecycle identities and action/carrier catalogs
  -> #58 canonical Materialized Callable carrier
  -> MIR ownership regions + semantic outcome edges
  -> aggregate lifecycle descriptors + logical ownership order
  -> Callable Lifecycle ABI + compatibility proof + atomic Call Entry Commit
  -> late plan freeze + verification certificate
  -> semantic artifact + executable realization binding + independent admission
  -> VM derived epilogues + Evaluation/Cleanup Evidence
  -> #56 non-callable tracer, then full callable-aware JIT teardown
  -> async quiescence + awaited suspension barrier
  -> placement leases + TransferId ownership transactions + snapshots
  -> generic contracts and erased dictionaries
  -> pre-freeze inlining and verified tail transfer
  -> default-on rollout under performance and adversarial gates
```

The spine permits early vertical slices but not authority shortcuts. In
particular, #56 may land non-callable catalog/preflight and a callable-free VM/JIT
tracer before #58. It cannot claim total teardown for a function that may own a
first-class callable until #58 removes the carrier ambiguity.

## 5. Phase 0 — Freeze prerequisites and fixtures

Dependencies: ADR-009 accepted, merged, and green.

- Pin post-ADR-009 APIs for canonical type, callable, effect, specialization, and diagnostic identities.
- Add small baseline fixtures for scalar, owned value, move, nested/early exits, `?`, finalizer, callable, cancel/suspend/deopt, generic, and remote paths.
- Record VM/JIT behavior, instructions, allocations, artifacts, compile/load latency, and native code. Define stable test-only owner/region/edge/site renderings.

Exit: no required identity remains a placeholder. If one is absent, amend the new architecture ADR; do not invent a parallel type system.

## 6. Phase 1 — Portable lifecycle descriptors and canonical hashes

Dependencies: Phase 0.

- Define versioned callable ABI/compatibility, aggregate, finalization-target,
  carrier/action, evidence-schema, plan, certificate, and remote-transfer wire
  descriptors in `shape-abi-v1` vocabulary.
- Use canonical domain-separated encoding: preserve semantic order, sort only sets/maps, reject duplicates/non-canonical bytes, and use full 256-bit hashes. Exclude Rust layout, pointers, local IDs, paths, and symbols.
- Seal the action algebra; separate portable descriptors from resolved targets; version action/carrier catalogs and bind catalog hashes into admission keys.

Tracer/exit: independent equivalent encoders produce the same golden hash; every semantic mutation changes it; unknown versions/actions refuse without a live compiler or registry.

## 7. Phase 2 — Issue #58: one first-class callable carrier

Dependencies: Phase 1 and ADR-009 callable identity.

- Make every Materialized Callable one owned raw refcounted object containing exact target, captures, release authority, and prevalidated lifecycle capability.
- Materialize named, closure, and module/provider targets only across value boundaries; keep direct calls and proven non-escaping closures allocation-free.
- Replace origin-dependent callable collection elements; align VM/JIT invocation, retain/release, GC, snapshot, and remote policy. Refuse inline function-ID bits in pointer-carrier slots.

Tracer/exit: one no-capture named function allocates nothing when called directly, materializes once when stored, and releases once; no value path branches on callable origin. Full #56 is now unblocked.

## 8. Phase 3 — Ownership regions and semantic outcome edges

Dependencies: Phase 2 for the full contract; non-callable prototypes may start earlier.

- Assign stable function-local IDs to regions, owners, successful entry/adoption events, and region-leaving executable sites; preserve move/borrow/transfer/finalization/carrier provenance through final MIR.
- Attach compact success, propagation, catch, failure, cancellation, suspension, deopt, and contained-fault metadata and derive one explicit `SemanticOutcomeEdgeGraph`.
- Cover loops, fallthrough, break/continue, return, `?`, handlers, and nesting. Reject opaque effects; neither bloat ordinary CFG with every unwind block nor hide outcomes in backend tables.
- Derive aggregate order from existing canonical container state while modeling
  adoption, extraction, replacement, and semantic reorder as ownership events.
  Pure relocation preserves that order without lifecycle-only metadata or
  allocation; otherwise prove child teardown unobservable or refuse observable
  child finalization.

Tracer/exit: a nested loop yields deterministic recipes for every exit without slot ordering; every producer/outcome is anchored or plan publication refuses.

## 9. Phase 4 — Callable Lifecycle ABI and atomic entry

Dependencies: Phases 1 and 3.

- Bind receiver mode, exact parameter/capture type and carrier, boundary role, return transfer/reborrow provenance, evaluator outcomes, and effects; keep its hash distinct from `FunctionHash`.
- Separate argument preparation from atomic entry. Before `Entered` the caller owns inputs; at commit, owned inputs and borrower tokens transfer. `DefinitelyNotExecuted` changes nothing; `OutcomeUnknown` transfers to recovery.
- At commit adopt a consuming receiver first, then `Own` parameters in
  declaration order. Preserve nested capture order; adopt an owned return at
  the caller success edge only after callee teardown. These are static
  ordinals, including under inlining and remote entry, never runtime rank data.
- Keep exact ABI hashes as identity. Prove higher-order compatibility separately:
  structural lifecycle roles and outcomes match exactly, while the normalized
  actual closed effect row may be a subset of the permitted closed row. Bind a
  replayable proof to both ABI hashes, rows, verifier/algebra version, and
  capability catalog; close generic rows before executable materialization.
- Verify composition for direct/indirect, recursive, higher-order, async, callback, erased, and remote calls. Materialized targets carry a prevalidated capability; direct calls erase checks.

Tracer/exit: injected failure immediately before/after entry proves exactly one owner; caller/callee compose without body inspection or teardown-time lookup.

## 10. Phase 5 — Late freeze, plan builder, and certificate

Dependencies: Phases 3 and 4 plus every semantic MIR back-patch.

- Freeze after final MIR, ownership/borrow/storage/carrier/transfer proof, and lifetime-changing optimization, but before either backend.
- Build one immutable region tree with interned exit recipes; the frame plan is only its root projection. Classify obligations and assign exact actions plus reverse-entry rank.
- Validate each concrete `AggregateLifecycleDescriptor`: exact child/carrier
  layout, occupancy, backing-store release, suspension class, target IDs, and
  capability closure. Select an erased/bulk, direct-unrolled, or specialized
  reverse-logical-order kernel; add a cursor only when an action can suspend.
- Give an aggregate finalizer only its admitted borrow-only intact-child view
  after quiescence/retirement. Then tear down children in reverse current
  logical order, backing storage, and outer carrier; mutation/stealing is not a
  finalizer capability.
- Emit the compact region/owner/event/outcome/anchor/block-witness certificate, not MIR. Separate static plan from dynamic armed/cursor/evidence state; statically known work has none.
- Freeze primary outcome before teardown; every outcome-qualified recipe keeps
  retirement, optional authorized finalization, and mandatory exact release
  distinct. Emit ordered typed evidence records by semantic action ordinal.

Tracer/exit: moved return + nested owner + success/failure produces deterministic bytes; changing one carrier to `Unproven` publishes neither plan nor backend artifact.

## 11. Phase 6 — Artifact integration and independent admission

Dependencies: Phase 5.

- Produce a portable `SemanticArtifactHash` over canonical executable MIR,
  plan/certificate, lifecycle ABI, typed targets/evidence schemas, and semantic
  dependencies. Separately hash each VM/native realization over that semantic
  hash, executable/relocations, resolved dependencies, backend/ISA/ABI,
  lowering/verifier versions, and its realization binding.
- Map each deterministic semantic MIR site to a final executable site, ordered
  expansion, recognized fusion, or proved `Elided`. Independently decode/lift
  the final executable and replay this refinement; unrecognized post-freeze
  transforms require a new semantic artifact, and every JIT version is
  quarantined until admitted.
- Replay blob-local owners linearly across every outcome, checking coverage, exactly-once transfer/release, order, ABI, target/effect/evidence identity, and catalogs; mint a non-serializable verified plan only on success.
- Link only verified artifacts. Every semantic admission cache key is exactly
  `SemanticArtifactHash` plus verifier version and the checked carrier/action-
  catalog subhash of the exact Execution ABI ID. Backend admission additionally
  keys by executable realization hash and pins resolved-provider generations.
- Bind snapshots to artifact hash/plan version and store only allowed dynamic state.

Tracer/exit: mutations to semantic site mapping, executable, plan, certificate,
ABI, checked catalog subhash, provider generation, or snapshot identity refuse
before a frame; local/cache/remote admission is identical and never warning-only.

## 12. Phase 7 — VM-derived epilogues

Dependencies: Phase 6.

- Derive ordinary VM branches/opcodes or shared epilogues from recipes; they are caches, never authority. Route every region/frame exit through an exit disposition.
- Transfer returns first; replace numeric frame-slot order with exact recipes.
  Structural Retirement guards a still-live carrier; Carrier Release performs
  mandatory exact representation release after finalization or abandonment.
- Keep evidence-free internal VM/JIT calls on their unchanged direct-return ABI:
  no envelope tag, widening, copy, or cleanup branch. Construct `Evaluation<R>`
  only at the evaluator/host boundary; when evidence is possible preserve the
  frozen primary and ordered typed evidence, with explicit discard policy.
- Represent invariant failure flatly as `EngineFaulted { primary,
  cleanup_evidence, fault, containment }`. Only trusted fault-safe structural
  actions may run after borrowers resolve or proven isolation revocation;
  otherwise quarantine ownership with a typed receipt, never run provider/user
  finalizers, and never claim speculative release.
- Shadow-compare legacy `DropCall` during migration, but refuse disagreement once the new ABI executes.

Tracer/exit: one non-callable function, then nested/failure paths, proves exact outcomes, order, and counts; new-ABI VM entry requires a verified plan and all terminal exits are total.

## 13. Phase 8 — JIT static lowering and issue #56 closure

Dependencies: Phases 2, 6, and 7.

- Refuse the whole native function before publication on any `Unproven` obligation. Empty work emits nothing; fixed work becomes direct carrier calls; only identical suffixes may share/outline.
- Route normal Return and every negative/native signal through plan epilogues. Use resolved relocations/direct calls—no target hash, catalog/string lookup, or plan iteration.
- Carry logical-frame plan identity and dynamic state in deopt metadata: resumptive deopt transfers, abandonment tears down.
- Emit the executable realization binding from final native sites and admit each
  optimized code version independently before publication.
- Stage #56 through non-callable catalog/refusal, callable-free tracer, differential proof, then #58 callable/finalizer/failure/deopt coverage.

Tracer/exit: scalar/string/collection/callable locals match VM counts/order on success, failure, guard deopt, cancellation, and finalizer failure; both tiers refuse the same unsupported function.

## 14. Phase 9 — Borrower Tokens, async finalization, and quiescence

Dependencies: VM/JIT plan execution and lifecycle ABI.

- Mint affine tokens for admitted tasks, callbacks, streams, devices, and remote borrowers, binding owners and domain generation.
- Seal admission, then accept only Joined, DefinitelyNotAdmitted/Executed,
  IsolationRevoked, or receipt-backed Transfer. Valid realizations include a
  local join, nested-scope receipt, device fence, stream-unsubscribe
  acknowledgement, or remote certainty/fenced-lease witness; cancellation
  request, timeout, connection loss, and provider booleans are not witnesses.
- Quiesce before finalization/release. Before the first may-suspend finalizer,
  atomically seal ordinary frame execution, guard every remaining exiting owner,
  and transfer storage, cursor, evidence builder, and witnesses to one affine
  teardown continuation. Resume cannot restore ordinary locals; synchronous
  plans erase the barrier, continuation, phase flag, and checks.
- Transfer unknown external outcomes to an affine recovery supervisor, remain suspended, or prove isolation revocation; enforce the Cleanup Snapshot Barrier.

Tracer/exit: a cancelled child holding a borrow blocks release until join; suspension between finalization/release restores without duplication; unresolved borrowers can never authorize release.

## 15. Phase 10 — Placement leases and distributed execution

Dependencies: Phases 6 and 9.

- Compute the transitive capability closure across rare outcomes, releases, finalizers, effects, permissions, scheduler/quiescence, providers, devices, and evidence.
- Resolve Shape targets by verified `ArtifactKey` and native targets by versioned opaque provider coordinate; pin generations and mint dense VM slots/native relocations.
- Bind destinations, resolved targets, sessions, leases, and obligations to one
  immutable provider generation. Reload publishes a new generation; live work
  keeps the old generation until drain or typed abandonment and never silently
  switches realization.
- Receiver verifies artifacts/closure and mints a frame lease before entry. `Unbound` chooses only among suitable placements; fallback is only a predeclared equivalent realization selected at admission.
- Put `Own` inputs into inaccessible sender escrow under a stable `TransferId`;
  serialization transfers nothing. Receiver acceptance atomically creates local
  owners and durably persists payload bytes/content reference, roles/ranks,
  lifecycle state, and a recovery owner before emitting its receipt or executing.
- Same-ID retries deduplicate/replay receipts. Owned results use the symmetric
  result escrow/commit/receipt protocol. Migration (1) durably prepares an
  inaccessible, inactive destination candidate, (2) durably fences the source,
  (3) atomically activates destination owners and continuation with preserved
  ranks, then (4) publishes the receipt. Recovery resumes from the durable step.
- Only durable fenced `RejectedBeforeCommit` restores escrow. Permanent
  uncertainty leaves escrow and its affine Recovery Obligation quarantined
  under a durable supervisor until typed revocation/loss/fault evidence; timeout
  or restart never resurrects the sender owner. Direct local calls have no
  journal, serialization, or receipts.

Tracer/exit: a receiver missing a cancellation-only finalizer refuses as DefinitelyNotExecuted; accepted execution performs no teardown-time catalog/network lookup and survives provider catalog rollover.

## 16. Phase 11 — Generics and intentional erasure

Dependencies: stable builder/verifier, lifecycle ABI, and specialization hashes.

- Check a non-executable parametric contract on generic definitions; after full type/const substitution freeze one concrete plan as part of the ordinary specialization.
- Normalize and close effect rows at concrete materialization. An unresolved
  higher-rank row needs exact scheme equality or refusal; compatibility never
  weakens exact ownership/outcome roles or changes callable identity.
- Use closed erased dictionaries only at explicit existential/`dyn` or declared code-size ABIs. Budget exhaustion chooses that ABI or refuses, never executes an unresolved template.
- Use stable hashes, not local `ConcreteType` IDs/vtable pointers; deduplicate only with exact layout, ABI, effects, finalizers, and release behavior.

Tracer/exit: scalar specialization erases cleanup, owned specialization calls direct release, and `dyn` uses one validated dictionary dispatch; every executable generic has a concrete plan or explicit erased ABI.

## 17. Phase 12 — Inlining, tail transfer, and cleanup code size

Dependencies: all semantic plan inputs and concrete specialization.

- Inline executable MIR plus region/owner/outcome/effect provenance before freeze, substitute lifecycle ABI, remap IDs, and verify one composite plan.
- Cost unique cleanup chains, certificate/deopt metadata, and capability closure; share only verified byte-identical suffixes.
- Tail-transfer only after explicit transfers leave the caller empty/disarmed, with no borrow/token/adaptation, matching outcomes/return ABI, accepted placement, and no deopt-recreatable owner.
- Reconstruct every logical inlined frame's plan/state on deopt; otherwise do not inline.

Tracer/exit: forced deopt inside an owned inlined callee reconstructs order; an empty caller tail-jumps while a caller with observable finalization remains an ordinary call.

## 18. Cross-cutting proof gates

Every phase closes with focused unit proofs before the next integration slice.
The full rollout requires all of the following:

### Semantic and differential

- Exhaustive region-edge fixtures: fallthrough, branch join, loop iteration,
  break/continue, return, `?`, catch/propagation, cancellation, suspension,
  resumptive/abandonment deopt, terminal failure, and contained fault.
- Owner states: never initialized, initialized, moved, adopted, transferred,
  borrowed, returned, conditionally armed, and dynamically repeated.
- VM/JIT differential assertions for primary outcome, secondary cleanup
  evidence, finalizer order, carrier release count, borrower resolution, and
  deopt reconstruction—not output alone.
- `Table`, `Batch`, and map fixtures cover mutation, reorder, compaction,
  relocation, pre-child finalization, and exact child/backing coverage while
  deriving order solely from their existing canonical state or refusing.
- Call fixtures permute named/prepared arguments and effect subsets while
  proving canonical ranks, exact structural roles, exact ABI identity, and no
  admitted-call compatibility checks.
- Evidence-free internal-call ABI snapshots prove identical return layout/code:
  no tag, widening, copy, cleanup branch, or envelope before the host boundary.
- Model-check or exhaustively enumerate small CFGs and compare builder output to
  an independent reference ownership machine.

### Adversarial admission

- Delete/duplicate/reorder an ownership event, outcome edge, recipe action,
  transfer, release, target, dependency, or block witness.
- Forge carrier kind, aggregate occupancy/order, lifecycle ABI/effect proof,
  evidence ordinal/schema, provider generation, TransferId receipt, semantic or
  realization hash/binding, checked catalog subhash, cursor, and placement lease.
- Feed non-canonical encodings, unknown actions, cycles, impossible ranks,
  mismatched call commits, and success-only capability closures.
- Require exact fail-closed diagnostics and prove zero frame/code/cache
  publication on rejection.

### Fault and lifecycle

- Inject finalizer failure/suspension, provider refusal, cancellation races,
  engine faults before/within cleanup, deopt at every action boundary, snapshot
  interruption, remote partition, and provider-generation rollover.
- Prove exact release continues after ordinary finalizer failure and never runs
  twice. A contained fault releases only after quiescence or proven isolation
  revocation; otherwise it produces typed quarantine without provider/user work.
- Crash at every input/result commit and migration state-machine step, replay
  same-ID requests, and restart either side; prove recovery resumes from durable
  state with never two active semantic owners and no restored or lost owner.
- Run ownership-sensitive suites under Miri/sanitizers where supported, plus
  long-running refcount and cancellation soak tests.

## 19. Performance gates

The baseline comes from Phase 0 and is compared on pinned hardware/toolchain.
Initial hard budgets are:

- Empty-plan scalar kernels: zero added allocations, zero plan/dictionary/target
  loads, zero indirect cleanup calls, and no extra terminal branch in emitted
  native code. Throughput regression must be within 1% median and 2% p95 after
  noise qualification.
- Fixed owned kernels: no runtime plan iteration or hash/catalog lookup; at most
  the proven armed check plus one direct action sequence per live obligation.
- Benchmark `Table`/`Batch`/map mutation, compaction, and teardown: order derives
  from existing state with zero lifecycle-only allocation/metadata; trivial
  children bulk/vectorize, fixed aggregates unroll, and dynamic aggregates use
  one specialized loop without reflection or per-element target dispatch.
- Evidence-free internal calls retain the direct-return ABI with zero tag,
  widening, copy, allocation, or cleanup branch; synchronous barriers erase,
  admitted calls do no proof lookup, and local calls do no transfer journaling.
- Verification: linear in executable sites plus certificate edges, once per
  semantic/backend admission; verified cache hit is O(1) in the appropriate
  semantic or realization identity. No per-call replay.
- Artifact overhead: plan plus certificate stays below 10% median and 20% p95 of
  function blob bytes on the representative corpus. Outliers require a named
  code-size analysis, not silent budget relaxation.
- Compile/load overhead: lifecycle build plus admission stays below 10% median
  and 20% p95 of the corresponding baseline stage. Measure cold and cached
  separately.
- Cleanup suffix sharing may reduce code size only when machine sequences are
  identical and order remains unchanged. Any benchmark win obtained by
  weakening verification or observable ordering is invalid.
- Distributed execution performs all capability resolution before entry. No
  teardown path may perform network discovery or catalog lookup.

If a budget fails, keep the semantic gate and optimize the representation or
lowering. A deliberate budget change requires measurements and an explicit ADR
amendment.

## 20. Rollout and compatibility

1. Build plan/certificate in compiler shadow mode and compare against fixtures;
   shadow results grant no execution authority.
2. Mint a new execution-ABI identity for artifacts carrying the contract.
   Legacy artifacts remain a distinct ABI; they are not upgraded by assuming an
   empty plan.
3. Require admission for new-ABI local VM execution; retain the prior runtime
   only for explicitly old-ABI artifacts during the compatibility window.
4. Enable JIT for a verified scalar/non-callable allowlist, then broaden by
   exact carrier/finalizer class as differential and performance gates close.
5. Enable async dynamic state, snapshots, and distributed placement only after
   their witnesses/leases are independently admitted.
6. Make new-ABI generation default only after the corpus has no unexplained
   refusal or performance regression. Remove the old-ABI execution path in a
   separately reviewed cleanup.

Telemetry may count compile refusal, admission failure, plan size, verification
time, outlined epilogues, and dynamic-state use. It must never convert a
verification error into a warning or fallback after a frame has started.

## 21. Explicit non-goals

- No finance, table, query, simulation, GPU, transaction, or stream policy in
  the core action algebra.
- No internal Arrow dependency or distributed query scheduler is implied by
  this lifecycle work.
- No arbitrary plugin teardown opcode, raw callback, symbol lookup, serialized
  pointer, or provider-authored lifecycle state.
- No general runtime frame scan, interpreted cleanup vector, deferred tail-call
  cleanup chain, or GC as a substitute for exact ownership.
- No promise of constant-space tail calls while observable caller cleanup
  remains.
- No snapshot of live non-portable resources without a versioned provider-
  neutral replay contract and exactly one restorable owner.
- No redesign of the borrow checker, storage planner, or type system where their
  existing facts are sufficient inputs. They do not become teardown authority.

## 22. Suggested ticket graph

| Ticket | Scope | Blocks on | Unblocks |
|---|---|---|---|
| RTP-00 | ADR-009 identity audit and baselines | ADR-009 | all |
| RTP-01 | Canonical lifecycle/aggregate/evidence/transfer descriptors | RTP-00 | RTP-02, RTP-06, RTP-07 |
| RTP-02 | Exact action/carrier catalogs and checked subhash | RTP-01 | RTP-03, RTP-06, RTP-09 |
| RTP-03 | #58 canonical callable carrier | RTP-02 | RTP-04, RTP-07, RTP-15 |
| RTP-04 | MIR region/owner/event/semantic-site provenance | RTP-00, RTP-03 | RTP-05, RTP-06, RTP-09 |
| RTP-05 | Semantic Outcome-Edge Graph | RTP-01, RTP-04 | RTP-07, RTP-09 |
| RTP-06 | Aggregate existing-state order/refusal and kernels | RTP-01, RTP-02, RTP-04 | RTP-09, RTP-14 |
| RTP-07 | Lifecycle ABI, canonical cross-call ranks, entry commit | RTP-03, RTP-05 | RTP-08, RTP-09, RTP-17 |
| RTP-08 | Closed-row effect compatibility proof | RTP-01, RTP-07 | RTP-09, RTP-18, RTP-21 |
| RTP-09 | Plan builder, aggregate recipes, and late freeze | RTP-02, RTP-04-RTP-08 | RTP-10, RTP-13 |
| RTP-10 | Certificate, semantic sites, realization binding/mutations | RTP-09 | RTP-11 |
| RTP-11 | Semantic/realization hashes and admission verifier | RTP-01, RTP-09, RTP-10 | RTP-12, RTP-14, RTP-16 |
| RTP-12 | SemanticArtifactHash-keyed cache/link/load plumbing | RTP-11 | RTP-14, RTP-16, RTP-18 |
| RTP-13 | Evaluation/evidence/fault carrier and direct-return ABI | RTP-09 | RTP-14, RTP-15, RTP-17 |
| RTP-14 | VM epilogues, aggregate kernels, fault-safe cleanup | RTP-06, RTP-11-RTP-13 | RTP-15, RTP-17 |
| RTP-15 | #56 JIT teardown, terminal signals, realization admission | RTP-03, RTP-10, RTP-14 | RTP-16, RTP-17 |
| RTP-16 | Resumptive/abandonment deopt lifecycle/evidence state | RTP-12, RTP-15 | RTP-19, RTP-22 |
| RTP-17 | Borrower Tokens, quiescence, awaited suspension barrier | RTP-07, RTP-13-RTP-15 | RTP-18, RTP-19 |
| RTP-18 | Target resolution, pinned generations, placement lease | RTP-08, RTP-11, RTP-17 | RTP-19 |
| RTP-19 | Durable input/result escrow and restartable migration | RTP-16-RTP-18 | RTP-20 |
| RTP-20 | Snapshot/restore of dynamic teardown/evidence state | RTP-12, RTP-16, RTP-19 | rollout |
| RTP-21 | Parametric contracts, closed effects, erased dictionaries | RTP-08, RTP-09, RTP-11 | RTP-22 |
| RTP-22 | Pre-freeze inlining, deopt reconstruction, tail transfer | RTP-16, RTP-21 | rollout |
| RTP-23 | Differential/adversarial/fault/performance gates | starts RTP-00; closes each ticket | default-on rollout |

Each ticket should be a tracer-bullet slice with one producer, one consumer, one
refusal proof, one mutation/adversarial proof, and one measured fast-path claim.
Do not group the entire VM, JIT, or remote rollout into a single implementation
ticket.
