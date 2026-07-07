export const meta = {
  name: 'wf3c-gc-phase2-barriers',
  description: 'WF-3C GC Phase 2 (ratified design §3.2 + §10 Phase 2): wire the RC barriers + candidate buffer, behind the default-off `gc` feature. Increment-barrier colors the target Black; decrement-barrier, after fetch_sub, frees now if the count hit zero (RC fast path unchanged) else colors the object Purple and appends it to the candidate buffer (once, via the buffered bit). Wire into clone_with_kind / drop_with_kind + the 3 interior-mutation sinks (SharedCell::set, var-field store, store-into-SharedCow-array) + give jit_write_barrier a real decrement-candidate body and jit_gc_safepoint a real poll. COLLECTION IS STILL A NO-OP this phase (only the buffer fills). Gate: on the Finding #31 reproducer the candidate buffer contains exactly the expected roots; RC fast path unchanged (rc==0 still frees immediately); feature-off strict no-op. Independent Opus verify + regression tests.',
  phases: [
    { title: 'Barriers', detail: 'increment=Black; decrement-to-nonzero=Purple+buffer; wire all sink sites + JIT bodies' },
    { title: 'Verify', detail: 'independent Opus: buffer holds expected roots on #31 repro; RC fast path + feature-off unchanged' },
    { title: 'Finish', detail: 'gates + buffer-content regression test + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w6-gc2'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave6/gc-phase2, off main — which already has GC Phases 0/1 merged). Build/test via: ' + DX + ' <cmd>. Ratified design: docs/design/real-gc-cycle-collection.md (§0 ratification, §3.1 metadata on the v2 header bits 4-6, §3.2 barriers, §3.3 collection [NOT this phase], §10 Phase 2).',
  '',
  'CONTEXT — what Phases 0/1 already landed (on main, behind default-off `gc` feature): GC color/buffered metadata on the v2 HeapHeader (crate::v2::heap_header, color bits 4-5, buffered bit 6) + gc_meta(ptr,kind) accessor + empty header-less GcSideTable + the single shared for_each_heap_child edge-enumeration primitive per v2 header carrier (TypedObject for_each_heap_child_edge, TypedArray for_each_typed_array_elem_ptr, Closure closure_immutable_heap_capture_edge). Files: crates/shape-value/src/{gc.rs,gc_visit.rs}.',
  '',
  'PHASE 2 — barriers + candidate buffer (behind `gc` feature; collection still a NO-OP):',
  '  - INCREMENT barrier: at clone_with_kind (and the JIT retain path), when the gc feature is on and the target is a cycle-capable header carrier, color the target BLACK (demonstrably in use). O(1).',
  '  - DECREMENT barrier: at drop_with_kind (and the 3 interior-mutation sinks: SharedCell::set at heap_value.rs, TypedObject var-field store at operations.rs, store-into-SharedCow-array), after the refcount fetch_sub: if the count hit ZERO -> free now exactly as today (RC fast path, UNCHANGED). If NONZERO -> color the object PURPLE and, if its buffered bit is not set, append its pointer to a candidate buffer and set buffered. O(1).',
  '  - JIT: give jit_write_barrier (today an unconditional ret) the same decrement-candidate logic; give jit_gc_safepoint (today polls a null flag) a real safepoint poll (the flag can stay unraised this phase — no collection yet). Keep the JIT hooks behind the gc feature / null-safe when off.',
  '  - The candidate buffer is per-VM (or per-runtime) transient state; add it (an ordered Vec of candidate pointers + the buffered-bit dedup). NO collection routine runs yet — the buffer just accumulates.',
  '',
  'CRITICAL — feature-off must stay a STRICT no-op and the RC fast path must be byte-identical: the barriers are additive #[cfg(feature=gc)] logic; the rc==0 free path is UNCHANGED (do not reorder or alter the existing free). The decrement barrier only ADDS the Purple+buffer step on the nonzero branch. Measure/argue there is no feature-off slowdown (the cfg-off path compiles to the same code).',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO collection / Drop-on-collect this phase (Phase 3; and per §0 the GC is memory-only — never runs Drop on cycle members). NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator). The barrier decides cycle-capable-carrier via HeapKind dispatch (not raw-bits probe). ADR-005 §1 preserved. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. All existing Drop/refcount tests byte-identical green feature-off AND feature-on (the barriers must not change observable Drop/refcount behavior — they only add color/buffer bookkeeping).',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule 2026-07-07): any behavior you fix/add needs a test; specifically add a buffer-content test (below). Any regression you find while wiring MUST get a reproducing test.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Barriers')
const B_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'sites_wired', 'buffer_correct', 'rc_fastpath_unchanged', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    sites_wired: { type: 'string', description: 'which sites got increment/decrement barriers (clone/drop/3 sinks/JIT), brief' },
    buffer_correct: { type: 'boolean', description: 'on the Finding #31 repro the candidate buffer holds exactly the expected cycle-root candidates' },
    rc_fastpath_unchanged: { type: 'boolean', description: 'rc==0 still frees immediately, byte-identical; feature-off is a strict no-op' },
    evidence: { type: 'string', description: 'buffer-content observation on #31 repro + Drop/refcount tests green feature-on/off; check-no-dynamic EXIT' },
  },
}
const b = await agent(CTX + '\n\nPHASE 2. Wire the increment (Black) + decrement-to-nonzero (Purple+buffer) barriers at clone_with_kind/drop_with_kind + the 3 interior-mutation sinks + the JIT hooks; add the transient candidate buffer. Collection stays a no-op. Prove the buffer holds the expected roots on the Finding #31 reproducer and the RC fast path + feature-off are unchanged. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C Phase 2: RC barriers + candidate buffer (gc feature, no collection)").',
  { label: 'barriers', phase: 'Barriers', effort: 'high', schema: B_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'buffer_correct', 'no_behavior_change', 'no_forbidden', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    buffer_correct: { type: 'boolean', description: 'candidate buffer holds exactly the expected roots on #31 repro (you re-checked from scratch)' },
    no_behavior_change: { type: 'boolean', description: 'feature-off strict no-op; RC fast path byte-identical; all Drop/refcount tests green feature-on AND off' },
    no_forbidden: { type: 'boolean', description: 'no is_heap/tag-decode/ValueWord/Bool-default; carrier check is HeapKind-dispatched (grep the diff)' },
    evidence: { type: 'string', description: 'your own from-scratch buffer-content check + Drop tests + grep; concise' },
  },
}
const verify = await agent(CTX + '\n\nBARRIERS: ' + JSON.stringify(b) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) on the Finding #31 reproducer, the candidate buffer contains exactly the expected cycle-root candidates (and NOT acyclic objects that hit rc==0 — those must be freed, never buffered). (2) ZERO behavior change: feature-off is a strict no-op, the rc==0 free path is byte-identical, all Drop/refcount tests green feature-on AND feature-off. (3) grep the diff for is_heap/tag_bits/ValueWord/Bool-default and confirm the cycle-capable-carrier decision is HeapKind-dispatched, not a raw-bits probe. Any wrong buffer contents, any behavior change, or any forbidden pattern = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'buffer_test', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-value/shape-vm tests (feature-off + feature-on), brief' },
    buffer_test: { type: 'string', description: 'the candidate-buffer-content regression test name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nBARRIERS: ' + JSON.stringify(b) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add a regression test that asserts the candidate buffer holds exactly the expected roots for a known cycle (and that an acyclic decrement-to-zero frees + is NOT buffered); no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-value and -p shape-vm (feature-off) + --features gc. Commit (git commit --no-verify -m "WF-3C Phase 2 finalize: barriers + candidate buffer + buffer-content test").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { b, verify, finish }
