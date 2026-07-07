export const meta = {
  name: 'wf3c-phase3a-collectcycles',
  description: 'WF-3C GC Phase 3a (design §R2 / §10): implement CollectCycles — the Bacon-Rajan synchronous trial-deletion 3-pass over the Phase-2 candidate buffer — behind the default-off `gc` feature. MarkRoots(MarkGray) -> ScanRoots(Scan/ScanBlack) -> CollectRoots(CollectWhite), using the TRUE HeapHeader.refcount + the Phase-1 for_each_heap_child visitor. Header carriers color via gc_meta flags (v2 bits 4-6); header-less (SharedCell/Reference/containers) via GcSideTable shadow_trial_count seeded from Arc::strong_count. MEMORY-ONLY reclaim per §0 ratification: CollectWhite frees cycle-member MEMORY and runs NO Drop/finalizer on cycle members (matches Rust Rc-cycle semantics). Single-thread, runs at the same-thread safepoint (full cross-worker rendezvous = 3b). This is the FIRST phase that deallocates — the verify must adversarially rule out premature-free / double-free / use-after-free. Independent Opus verify + regression tests.',
  phases: [
    { title: 'Collect', detail: 'Bacon-Rajan 3-pass trial deletion; memory-only free; header + side-table' },
    { title: 'Verify', detail: 'independent Opus: #31 + 3 sinks collected; NO premature-free/double-free/UAF; no Drop on cycle members' },
    { title: 'Finish', detail: 'gates + collection regression tests (leak bounded, live survives) + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-gc3a'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/gc-phase3a, off main — has GC Phases 0/1/2 merged). Build/test via: ' + DX + ' <cmd>. Ratified design: docs/design/real-gc-cycle-collection.md (§0 ratification — MEMORY-ONLY, no Drop on cycle members; §3.3 CollectCycles; §R2 refined; §3.5 header-less side-table).',
  '',
  'CONTEXT — Phases 0/1/2 (merged, behind default-off `gc` feature): GC color/buffered metadata on the v2 HeapHeader (color bits 4-5, buffered bit 6) + gc_meta(ptr,kind) accessor + empty GcSideTable (crate::gc / gc.rs); the shared for_each_heap_child edge-enumeration visitor per v2 carrier (gc_visit.rs — TypedObject/TypedArray/Closure); RC barriers (increment->Black at clone_with_kind; decrement-to-nonzero->Purple+buffer at drop_with_kind + the 3 interior-mutation sinks) filling a thread-local CandidateBuffer with buffered-bit dedup. Collection is currently a NO-OP.',
  '',
  'PHASE 3a — implement CollectCycles (the collection engine; still single-thread; runs when invoked at a same-thread safepoint):',
  '  Standard Bacon-Rajan synchronous trial-deletion over the candidate buffer, using the TRUE HeapHeader.refcount as the count and for_each_heap_child to enumerate outgoing heap edges:',
  '  1. MarkRoots: for each buffered candidate, MarkGray it. MarkGray(s): if not Gray, color Gray; for each heap child t: TRIAL-DECREMENT t refcount, then MarkGray(t). (Purple candidates that are Black with rc==0 are freed + dropped from the buffer.)',
  '  2. ScanRoots: Scan(s) each candidate. Scan(s): if Gray — if rc>0 (an EXTERNAL reference survives) -> ScanBlack(s) (re-increment children, color Black); else color White and Scan children.',
  '  3. CollectRoots: CollectWhite(s) each candidate, clearing buffered. CollectWhite colors Black, recurses to children, and FREES White nodes.',
  '',
  '  HEADER-LESS kinds (SharedCell/Reference + HashMap/HashSet/Deque): you cannot trial-decrement a std-Arc strong count without dropping, so use a SHADOW trial-count in GcSideTable, seeded from Arc::strong_count, and do the trial arithmetic on the shadow. Header carriers use the real HeapHeader.refcount + gc_meta color bits.',
  '',
  'MEMORY-ONLY RECLAIM (§0 ratification — CRITICAL): CollectWhite frees the MEMORY of a White (cycle-garbage) node WITHOUT running that node\'s Rust Drop / any user finalizer — a cycle member with a Drop impl must NOT have Drop run (identical to Rust Rc/Arc cycles). Free the raw v2-raw allocation directly (do not route through the normal Arc drop that would recursively release + finalize). Non-cycle children whose refcount reaches 0 during the collection ARE freed via the normal path (their Drop runs) — only the cycle members themselves skip Drop.',
  '',
  'TRIGGER: expose CollectCycles to run at the same-thread safepoint (the Phase-2 dispatch gate can call it when the flag is raised / buffer exceeds a threshold); for tests, also allow explicit invocation. The FULL cross-worker STW rendezvous is Phase 3b — do NOT build it here; single-thread collection at a same-thread safepoint is this phase.',
  '',
  'CONSTRAINTS (CLAUDE.md + design, CRITICAL): behind the `gc` feature (feature-off strict no-op). MEMORY-ONLY (no Drop on cycle members). NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator); child enumeration is the HeapKind-dispatched for_each_heap_child. ADR-005 §1 preserved. The RC fast path (rc==0 immediate free) stays byte-identical. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. This phase DEALLOCATES — be maximally careful about premature-free (an object with a live external ref must NEVER be freed), double-free, and use-after-free.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): add collection tests — the Finding #31 cycle + the 3 interior-mutation-sink cycles are collected (leak bounded), AND a live cycle with an external reference is NOT collected (no premature free), AND an acyclic object is unaffected.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Collect')
const C_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'collects_cycles', 'no_premature_free', 'memory_only', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the CollectCycles 3-pass + side-table shadow-count + memory-only free, brief' },
    collects_cycles: { type: 'boolean', description: 'Finding #31 + the 3 sink cycles are collected (leak bounded) when gc runs' },
    no_premature_free: { type: 'boolean', description: 'a cycle with an external reference survives; acyclic objects unaffected' },
    memory_only: { type: 'boolean', description: 'cycle members are freed memory-only — no Drop/finalizer runs on a cycle member' },
    evidence: { type: 'string', description: 'captured: #31 RSS bounded under gc; live-cycle survives; check-no-dynamic EXIT; feature-off no-op' },
  },
}
const c = await agent(CTX + '\n\nPHASE 3a. Implement CollectCycles (Bacon-Rajan 3-pass, memory-only, header + side-table shadow-count). Prove: Finding #31 + the 3 sink cycles collect (bounded RSS/refcount), a live cycle with an external ref survives (no premature free), cycle members skip Drop. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C Phase 3a: CollectCycles Bacon-Rajan trial deletion (gc feature, single-thread, memory-only)").',
  { label: 'collect', phase: 'Collect', effort: 'high', schema: C_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'collects', 'no_unsafe_free', 'no_drop_on_cycle', 'no_behavior_change', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    collects: { type: 'boolean', description: 'Finding #31 + 3 sink cycles collected from your OWN scratch repro (bounded RSS/refcount)' },
    no_unsafe_free: { type: 'boolean', description: 'NO premature-free (external-referenced object survives), NO double-free, NO use-after-free — adversarially checked' },
    no_drop_on_cycle: { type: 'boolean', description: 'a cycle member with an observable Drop does NOT run Drop (memory-only), a non-cycle child freed during collection DOES' },
    no_behavior_change: { type: 'boolean', description: 'feature-off strict no-op; RC fast path byte-identical; all Drop/refcount tests green feature-on AND off' },
    evidence: { type: 'string', description: 'your own from-scratch repros incl. a live-cycle-survives control + a Drop-observability probe; concise' },
  },
}
const verify = await agent(CTX + '\n\nCOLLECT: ' + JSON.stringify(c) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. This phase DEALLOCATES — be maximally skeptical about memory safety. From scratch: (1) Finding #31 + the 3 sink cycles collect (bounded refcount/RSS under gc)? (2) SAFETY: does a cycle that STILL has an external reference survive (NOT freed)? does an acyclic object survive? try to construct a premature-free / double-free / use-after-free (external ref into a cycle, a shared subobject held both inside and outside the cycle, re-entrancy during collection) — run under the strongest sanitizer/Miri you can; ANY unsafe free = REFUTED. (3) does a cycle member with an observable Drop correctly SKIP Drop while a non-cycle child freed during collection runs its Drop? (4) feature-off strict no-op + all Drop/refcount tests green both ways. Any unsafe free, wrong Drop behavior, or behavior change = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-value/shape-vm (feature-off + feature-on), brief' },
    tests_added: { type: 'string', description: 'collection regression tests: cycle collected + live-cycle survives + acyclic unaffected + no-Drop-on-cycle-member' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nCOLLECT: ' + JSON.stringify(c) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add collection regression tests: the #31 cycle + 3 sink cycles collected (bounded), a live-referenced cycle NOT collected (no premature free), acyclic unaffected, a cycle member skips Drop; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-value and -p shape-vm (feature-off) + --features gc. Commit (git commit --no-verify -m "WF-3C Phase 3a finalize: CollectCycles + collection safety tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { c, verify, finish }
