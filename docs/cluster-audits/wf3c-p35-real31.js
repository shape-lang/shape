export const meta = {
  name: 'wf3c-p35-headerless-real31',
  description: 'WF-3C §3.5 completeness: make CollectCycles actually collect the REAL Finding #31 leak (a closure captured into a mutable array — `var arr=[]; arr.push(|| arr.len())`), which Phase 3a did NOT collect. Diagnose-first: map exactly which HeapKind carriers + which EDGES the real repro forms (closure MUTABLE OwnedMutable/Shared captures — currently only immutable captures are enumerated; the SharedCow/TypedArray element-holding-closure edge; any std-Arc header-less carrier), and the CORRECT memory-only free per carrier (raw dealloc for v2 header carriers vs the trickier std-Arc-backed SharedCell/HashMap where you cannot raw-free an Arc control block). Then extend the for_each_heap_child visitor + CollectCycles to collect the real cycle end-to-end (interpreter path), memory-only, with a bounded-refcount/RSS test. Std-Arc header-less carriers whose memory-only-free is genuinely unsafe THIS lane may stay leak-safe-deferred with an explicit boundary — but the real closure-in-array #31 MUST collect. Independent Opus valgrind-verified.',
  phases: [
    { title: 'Diagnose', detail: 'map the real #31 carriers/edges + correct memory-only-free per carrier type' },
    { title: 'Implement', detail: 'extend visitor + CollectCycles to collect the real closure-in-array cycle, memory-only' },
    { title: 'Verify', detail: 'independent Opus valgrind: real #31 collected (bounded), no premature/double/UAF, memory-only' },
    { title: 'Finish', detail: 'gates + real-#31 bounded-refcount regression test + documented residual boundary' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-gcp35'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/gc-p35-headerless, off main — has GC Phases 0/1/2/3a merged). Build/test via: ' + DX + ' <cmd>. Ratified design: docs/design/real-gc-cycle-collection.md (§0 memory-only; §3.4 shared visitor; §3.5 header-less side-table; §R2 CollectCycles).',
  '',
  'CONTEXT — Phase 3a (merged) built CollectCycles (Bacon-Rajan trial deletion, memory-only, valgrind-clean) in crate::gc (gc.rs) using for_each_heap_child (gc_visit.rs) + gc_meta color bits (v2 HeapHeader, from Phases 0/1/2). It collects cycles among v2 HEADER carriers whose edges the visitor enumerates: TypedObject fields, TypedArray elements, and Closure IMMUTABLE heap captures. It does NOT yet collect: (a) Closure MUTABLE captures (OwnedMutable/Shared interior — closure_immutable_heap_capture_edge only reads immutable ones), (b) the SharedCow/mutable-array element-holding-closure edge, (c) std-Arc header-less carriers (SharedCell/Reference/HashMap/HashSet/Deque) — these were LEAK-safe-deferred. All behind the default-off `gc` feature.',
  '',
  'THE TARGET: the REAL Finding #31 leak is `var arr = []; arr.push(|| arr.len())` — a closure that captures the mutable array `arr`, pushed INTO that same array. The closure→arr edge is a MUTABLE capture; the arr→closure edge is a mutable-array element store. Phase 3a does not collect this (confirmed leak-safe-not-collected). This lane must make CollectCycles collect THIS cycle end-to-end on the interpreter path, memory-only, and prove the leak is bounded (refcounts of the closure + array return to collectable / freed=N, not pinned forever).',
  '',
  'DIAGNOSE FIRST (no impl): reproduce the real #31 at the value/VM layer under `gc`; determine EXACTLY which HeapKind carriers and which EDGES the cycle comprises (is the array a TypedArray header carrier or a SharedCow/std-Arc carrier? is the closure capture OwnedMutable or Shared? what enumerates each edge today, and what is MISSING). For EACH carrier in the cycle, state the CORRECT memory-only free: v2 HeapHeader carriers (TypedObject/TypedArray/Closure) raw-dealloc via the existing _free_memory_only pattern (no Drop); std-Arc-backed carriers (if any are in this cycle) CANNOT be raw-freed (that corrupts the Arc control block) — for those, define the sound memory-only reclamation (break the cycle edge so the Arc count reaches 0 via the normal Arc path WITHOUT running the user Drop, OR if that is not soundly achievable this lane, mark that specific carrier leak-safe-deferred with the precise reason). The goal is the closure-in-array cycle collected; do not over-reach into unsound std-Arc raw-frees.',
  '',
  'IMPLEMENT: extend for_each_heap_child (gc_visit.rs) to enumerate the missing edges (closure MUTABLE captures via the closure capture layout / heap_capture_mask; the mutable-array element edge) — dispatching on HeapKind, NO is_heap/tag-decode/ValueWord/Bool-default, sharing the SAME enumeration the destructive Drop path uses (the §3.4 single-source-of-truth discipline — extend the shared primitive, do NOT add a divergent mirror walk). Extend CollectCycles / GcNode classification to trial + collect these carriers with the correct per-carrier memory-only free. Keep header-less std-Arc carriers that are genuinely unsafe-to-free-this-lane as a documented leak-safe boundary.',
  '',
  'CONSTRAINTS (CLAUDE.md + design, CRITICAL): behind `gc` feature (feature-off strict no-op). MEMORY-ONLY (no Drop on cycle members). §3.4 single-source-of-truth (Drop path + GC visitor share ONE enumeration per carrier — no mirror walk drift). NO forbidden patterns. RC fast path byte-identical. This DEALLOCATES — be maximally careful about premature-free/double-free/UAF, especially with the mutable-capture + shared-array edges. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Run the gc tests under valgrind (--error-exitcode=99) and REQUIRE exit 0.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a test that the REAL closure-in-array #31 cycle is collected (bounded refcount / freed=N), a live version with an external reference is NOT collected, and cycle members skip Drop.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['carriers', 'missing_edges', 'free_semantics', 'deferred'],
  properties: {
    carriers: { type: 'string', description: 'the exact HeapKind carriers in the real #31 cycle (array kind, closure capture kind)' },
    missing_edges: { type: 'string', description: 'which edges are NOT enumerated today + what enumerates them (layout/mask)' },
    free_semantics: { type: 'string', description: 'correct memory-only free per carrier (raw-dealloc header vs std-Arc cycle-break)' },
    deferred: { type: 'string', description: 'any carrier that must stay leak-safe-deferred this lane + why (or "none — real #31 fully collectable")' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no impl). Reproduce the real #31 under gc; map carriers/edges + correct per-carrier memory-only free. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Implement')
const IMPL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'real31_collected', 'single_source', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'visitor + CollectCycles extensions + per-carrier free, brief' },
    real31_collected: { type: 'boolean', description: 'the real closure-in-array #31 cycle is now collected (bounded refcount/freed=N) under interpreter' },
    single_source: { type: 'boolean', description: 'the new edges share the SAME enumeration the Drop path uses (no mirror walk)' },
    evidence: { type: 'string', description: 'captured: real #31 collected; live version survives; valgrind exit 0; check-no-dynamic EXIT' },
  },
}
const impl = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — IMPLEMENT. Extend the shared visitor + CollectCycles to collect the real closure-in-array #31, memory-only, per-carrier free as diagnosed. Run the gc tests under valgrind (exit 0 required). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C 3.5: collect the real closure-in-array Finding #31 (mutable-capture edges, memory-only)").',
  { label: 'implement', phase: 'Implement', effort: 'high', schema: IMPL_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'real31_collected', 'no_unsafe_free', 'no_behavior_change', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    real31_collected: { type: 'boolean', description: 'the real closure-in-array #31 is collected from your OWN scratch repro (bounded)' },
    no_unsafe_free: { type: 'boolean', description: 'NO premature-free/double-free/UAF (valgrind exit 0 on YOUR adversarial repros incl. mutable-capture + shared-array edge cases)' },
    no_behavior_change: { type: 'boolean', description: 'feature-off strict no-op; RC fast path byte-identical; Drop/refcount tests green both ways' },
    evidence: { type: 'string', description: 'your own valgrind run + live-cycle-survives control + Drop-skip probe; concise' },
  },
}
const verify = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\nIMPL: ' + JSON.stringify(impl) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. This DEALLOCATES the mutable-capture + shared-array edges — be maximally skeptical. From scratch: (1) the real `arr.push(|| arr.len())` cycle collects (bounded refcount/freed under gc)? (2) SAFETY under valgrind (--error-exitcode=99): construct premature-free/double-free/UAF probes for the mutable-capture + shared-array-element edges (external ref into the closure or the array; a subobject shared inside+outside; re-entrant collection); ANY unsafe free = REFUTED. (3) memory-only: a cycle member with an observable Drop skips it. (4) feature-off no-op + Drop/refcount tests green both ways. Any unsafe free, wrong Drop, or behavior change = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'residual', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-value/shape-vm (off + gc) + valgrind, brief' },
    test_added: { type: 'string', description: 'the real-#31 collection regression test + live-survives + Drop-skip' },
    residual: { type: 'string', description: 'any carrier still leak-safe-deferred (documented boundary)' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nIMPL: ' + JSON.stringify(impl) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add the real-#31 collection regression test (bounded) + live-survives + Drop-skip; document any residual leak-safe-deferred carrier; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-value + -p shape-vm (feature-off) + --features gc, and the gc tests under valgrind. Commit (git commit --no-verify -m "WF-3C 3.5 finalize: real Finding #31 collected + safety tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, impl, verify, finish }
