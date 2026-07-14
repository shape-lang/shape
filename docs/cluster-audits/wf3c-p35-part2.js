export const meta = {
  name: 'wf3c-p35-part2-bound31',
  description: 'WF-3C §3.5-part2: fully BOUND Finding #31 by reclaiming the two std-Arc residual carriers part1 left leaking — B (Arc<HeapValue::ClosureRaw>, the closure value) and D (Arc<SharedCell>, the Shared-capture cell), ~112B/iter. You CANNOT raw-free a std-Arc (corrupts the control block), and B\'s OwnedClosureBlock::Drop unconditionally cascades into release_typed_closure(C) which would re-enter part1\'s already-raw-freed block C (UAF). So this needs a real reclamation model. DESIGN-FIRST: strongly prefer a model that does NOT violate the SharedCell mid-life-kind-change lockstep invariant (closure_layout.rs:157-160) — evaluate break-one-edge-then-let-normal-Arc-RC-cascade (likely REPLACING part1\'s raw-free-of-C so the normal OwnedClosureBlock::Drop frees C+walks captures exactly once) vs neuter-then-drop (violates the invariant → LAST resort, must surface for user ratification). If the safe model requires overriding a ratified invariant OR user sign-off, STOP and surface — do not override unilaterally. This is the highest-risk memory-safety change in the GC series (drives real Arc counts, may replace a raw-free): adversarial valgrind verify is mandatory. Memory-only (no user Drop on cycle members). gc feature off by default. Independent Opus.',
  phases: [
    { title: 'Design', detail: 'safe B+D reclamation model; invariant-respecting; does it replace part1 raw-free-of-C; any user-ratification needed' },
    { title: 'Implement', detail: 'reclaim B+D (only if no blocking user decision); #31 fully bounded' },
    { title: 'Verify', detail: 'independent Opus valgrind: #31 bounded (freed==4 / RSS flat), no UAF/double-free, no invariant break' },
    { title: 'Finish', detail: 'gates + #31-fully-bounded regression test (freed==all / bounded RSS over N iters) + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-gcp35b'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/gc-p35-part2, off main HEAD — has §3.5-part1 merged bd82fa6f). Build/test via: ' + DX + ' <cmd>. Design: docs/design/real-gc-cycle-collection.md (§0 memory-only; §3.4 single-source visitor; §3.5 header-less).',
  '',
  'CONTEXT — §3.5-part1 (merged) established the real Finding #31 is a 4-node cycle: A = TypedArray<CallableArrayElem> (v2-header), B = Arc<HeapValue::ClosureRaw> (std-Arc, the pushed closure value), C = TypedClosureHeader block (v2-header, owned by the OwnedClosureBlock inside B\'s HeapValue), D = Arc<SharedCell> (std-Arc, the Shared capture cell; back-edges to A). for_each_heap_child already traces all 4 edges (single-source). CollectCycles currently frees the two v2-header carriers A + C memory-only via raw dealloc (free_v2_typed_array_memory_only + dealloc_typed_closure_no_drop), and LEAVES B + D leaking (~112B/iter) because a std-Arc cannot be raw-freed and B\'s OwnedClosureBlock::Drop cascades into release_typed_closure(C) → re-enters the already-raw-freed C (UAF). So #31 is reduced but NOT bounded.',
  '',
  'GOAL: fully BOUND #31 — over N iterations of the closure-in-array garbage cycle, total RSS stays flat (all 4 nodes reclaimed per collection, no per-iter residual), memory-only, no premature/double/UAF.',
  '',
  'THE KEY INVARIANT (do NOT break lightly): closure_layout.rs:157-160 — SharedCell\'s `kind` companion MUST stay lockstep with `value`; "Mid-life kind changes" are forbidden. A neuter-SharedCell-kind-then-drop reclamation VIOLATES this. PREFER a model that does not.',
  '',
  'DESIGN candidates to evaluate (Phase 1):',
  '  (1) BREAK-ONE-EDGE + NORMAL-RC-CASCADE: for a White (garbage) closure sub-cycle, instead of raw-freeing C, drive B\'s real Arc<HeapValue::ClosureRaw> strong count to 0 by dropping the cycle-held share(s); the normal OwnedClosureBlock::Drop then frees C AND walks/releases its captures (including the D edge) exactly once, and D\'s SharedCell::Drop releases its A edge. This likely REQUIRES removing part1\'s raw-free-of-C (else double-free) and instead breaking exactly ONE cycle edge so the remaining RC cascade tears down A/B/C/D via the normal Arc paths without double-free and without re-freeing A raw. Establish which single edge to break and the exact ordering. Confirm no user Drop trait finalizer runs on cycle members (the runtime carriers here have no user Drop; memory-only preserved). This is the FAVORED model.',
  '  (2) NEUTER-THEN-DROP: neuter D\'s SharedCell kind to a scalar so its Drop no-ops the A edge, then drop the Arc — VIOLATES the closure_layout.rs:157-160 lockstep invariant → only if (1) is unworkable, and then STOP + surface for user ratification (do not implement an invariant override unilaterally).',
  '',
  'DESIGN-FIRST (Phase 1, no impl): choose the safe model; specify exactly which part1 behavior it changes (does it replace raw-free-of-C? raw-free-of-A?), the precise edge-break + RC-drop ordering, why it cannot double-free or UAF, and whether it needs to override any ratified invariant or user sign-off. If a blocking user decision is required, say so explicitly (the workflow will STOP).',
  '',
  'CONSTRAINTS (CLAUDE.md + design, CRITICAL): behind `gc` feature (feature-off strict no-op). MEMORY-ONLY (no user Drop on cycle members). §3.4 single-source-of-truth (reuse the SAME enumeration primitives; no mirror walk). NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator). RC fast path byte-identical. This drives REAL Arc counts + may replace a raw-free — maximally careful about double-free / UAF / premature-free. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Run gc tests under valgrind (--error-exitcode=99), REQUIRE exit 0.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a test that the real closure-in-array #31 cycle is FULLY reclaimed (all 4 nodes; refcounts of B and D reach 0; freed count == all cycle members) and that over N iterations total live allocation stays flat (bounded, not linear); a live externally-referenced cycle still survives; cycle members skip user Drop.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Design')
const DESIGN_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['model', 'changes_to_part1', 'safety_argument', 'needs_user_ratification', 'ratification_detail'],
  properties: {
    model: { type: 'string', description: 'chosen reclamation model (break-edge-cascade vs neuter) + the exact edge-break/RC-drop ordering' },
    changes_to_part1: { type: 'string', description: 'what part1 behavior changes (replace raw-free-of-C? of A? keep?)' },
    safety_argument: { type: 'string', description: 'why no double-free / UAF / premature-free; memory-only preserved' },
    needs_user_ratification: { type: 'boolean', description: 'true if the safe model requires overriding a ratified invariant or a user decision' },
    ratification_detail: { type: 'string', description: 'if true: exactly what invariant/decision + why unavoidable; else "none"' },
  },
}
const design = await agent(CTX + '\n\nPHASE 1 — DESIGN ONLY (no impl). Choose the safe B+D reclamation model, prefer the invariant-respecting break-edge-cascade. State precisely what changes vs part1, the safety argument, and whether a user ratification is required. Do NOT commit.',
  { label: 'design', phase: 'Design', effort: 'high', schema: DESIGN_SCHEMA })

// GATE: if the safe model requires overriding a ratified invariant / user decision, STOP and surface.
if (design && design.needs_user_ratification) {
  log('DESIGN requires user ratification: ' + (design.ratification_detail || '(unspecified)') + ' — STOPPING for supervisor to surface. No impl this run.')
  return { design, halted_for_ratification: true }
}

phase('Implement')
const IMPL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'fully_bounded', 'no_invariant_break', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the B+D reclamation + any part1 raw-free replacement, brief' },
    fully_bounded: { type: 'boolean', description: 'all 4 nodes reclaimed; over N iters total live allocation flat (not linear)' },
    no_invariant_break: { type: 'boolean', description: 'SharedCell lockstep invariant (closure_layout.rs:157-160) NOT violated' },
    evidence: { type: 'string', description: 'captured: B+D refcounts reach 0; N-iter allocation flat; valgrind exit 0; check-no-dynamic EXIT 0' },
  },
}
const impl = await agent(CTX + '\n\nDESIGN: ' + JSON.stringify(design) + '\n\nPHASE 2 — IMPLEMENT the chosen model. Fully bound #31 (all 4 nodes reclaimed, N-iter allocation flat), memory-only, invariant intact. Run gc tests under valgrind (exit 0 required). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C 3.5-part2: fully bound Finding #31 (reclaim std-Arc closure-value + SharedCell residuals, memory-only)").',
  { label: 'implement', phase: 'Implement', effort: 'high', schema: IMPL_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'fully_bounded', 'no_unsafe_free', 'no_invariant_break', 'no_behavior_change', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    fully_bounded: { type: 'boolean', description: 'from YOUR OWN repro: all 4 nodes reclaimed; N-iter live allocation flat' },
    no_unsafe_free: { type: 'boolean', description: 'NO double-free/UAF/premature-free (valgrind exit 0 on YOUR adversarial probes: external ref on B, on D, re-entrant collect, partial-live)' },
    no_invariant_break: { type: 'boolean', description: 'SharedCell lockstep invariant intact; no forbidden pattern' },
    no_behavior_change: { type: 'boolean', description: 'feature-off strict no-op; RC fast path byte-identical; Drop/refcount tests green both ways' },
    evidence: { type: 'string', description: 'your own valgrind + N-iter allocation-flatness measurement + live-survives control; concise' },
  },
}
const verify = await agent(CTX + '\n\nDESIGN: ' + JSON.stringify(design) + '\nIMPL: ' + JSON.stringify(impl) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. This drives REAL Arc counts and may replace a raw-free — the highest UAF/double-free risk in the GC series. From scratch: (1) is #31 FULLY bounded — all 4 nodes reclaimed, and over N (e.g. 100k) iterations does total live allocation stay FLAT (measure it), not grow ~112B/iter? (2) SAFETY under valgrind (--error-exitcode=99): probes = external Arc ref on B, external Arc ref on D, a subobject shared inside+outside, re-entrant/partial-live collect; ANY double-free/UAF/premature-free = REFUTED. (3) SharedCell lockstep invariant (closure_layout.rs:157-160) intact + no forbidden pattern. (4) feature-off no-op + Drop/refcount tests green both ways. Any unsafe free, invariant break, unbounded residual, or behavior change = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-value (off + gc) + valgrind, brief' },
    test_added: { type: 'string', description: 'the #31-fully-bounded (all-nodes + N-iter-flat) + live-survives + Drop-skip tests' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nIMPL: ' + JSON.stringify(impl) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add the #31-fully-bounded regression test (all 4 nodes reclaimed + N-iter allocation flat) + live-survives + Drop-skip; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-value (feature-off) + --features gc, and the gc tests under valgrind. Commit (git commit --no-verify -m "WF-3C 3.5-part2 finalize: Finding #31 fully bounded + safety tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { design, impl, verify, finish }
