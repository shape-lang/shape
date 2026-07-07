export const meta = {
  name: 'wf3c-gc-prep-phase0-1',
  description: 'WF-3C GC impl, Phases 0+1 ONLY (ratified design docs/design/real-gc-cycle-collection.md). Phase 0: add tri-color+buffered flag-bit constants + gc_meta accessor + empty header-less side-table type + the `gc` Cargo feature (all no-op, NO behavior change). Phase 1 (highest-risk lockstep): extract the DESTRUCTIVE Drop-side heap-child walks (TypedObjectStorage::_drop heap-mask, OwnedClosureBlock captures, container Drops, TypedArray elements) to a SINGLE shared for_each_heap_child edge-enumeration primitive that BOTH the Drop path and a new read-only GC visitor call — so they can never drift. Prove parity with the gc_barrier_debug BARRIER_COUNT==HEAP_WRITE_COUNT coverage harness. NO collector, NO barriers with real bodies yet. Independent Opus verify. Behavior-preserving prep that lands first for review safety.',
  phases: [
    { title: 'Phase0', detail: 'flag bits + gc_meta + side-table type + gc feature flag (no behavior)' },
    { title: 'Phase1', detail: 'single shared for_each_heap_child; Drop path + read-only GC visitor both call it' },
    { title: 'Verify', detail: 'independent Opus: no behavior change; edge parity proven; all Drop tests green' },
    { title: 'Finish', detail: 'gates + parity/coverage test + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w5-gcprep'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave5/gc-prep, off main). Build/test via: ' + DX + ' <cmd>.',
  '',
  'This implements ONLY Phases 0 and 1 of the RATIFIED cycle-collector design at docs/design/real-gc-cycle-collection.md (read §0 ratification + §3.1 metadata + §3.4 shared edge primitive + §10 Phase 0/1 + §7 metadata placement). These two phases are DELIBERATELY behavior-preserving prep — NO collector, NO real barrier bodies, NO Drop changes. They land first so the highest-risk piece (the visitor↔Drop-walk lockstep) is provable before any collector exists.',
  '',
  'PHASE 0 — metadata + accessors (NO behavior change):',
  '  - In crates/shape-value/src/heap_header.rs: add the tri-color (2 bits) + buffered (1 bit) flag-bit CONSTANTS in HeapHeader.flags bits 3-5. Do NOT touch FLAG_MARKED(0x01)/FLAG_PINNED(0x02)/FLAG_READONLY(0x04) (bits 0-2) or _pad (offset 7, element-type-stamped by TypedArray). Header layout + DATA_OFFSET==8 + all offset tests MUST stay byte-identical.',
  '  - Add a single gc_meta(ptr, kind) accessor (get/set color + buffered) that dispatches on HeapKind — NO new sum type projecting 1:1 to HeapKind (ADR-005 §1). For header carriers it reads/writes the flags byte; for header-less kinds it routes to the side table.',
  '  - Add the header-less side-table TYPE (address-keyed map holding {color, buffered, shadow_trial_count}) but leave it EMPTY/unused this phase.',
  '  - Add a `gc` Cargo feature (default OFF) gating the new code so feature-off is a strict no-op.',
  '',
  'PHASE 1 — single shared edge-enumeration primitive (the lockstep-critical work):',
  '  - Today the destructive Drop path enumerates a heap object\'s children in several places: TypedObjectStorage::_drop (heap_mask walk), OwnedClosureBlock capture layout, the mutable-container Drops (HashMap/HashSet/Deque variants), TypedArray element release. Extract the EDGE ENUMERATION (which slots are heap children, by HeapKind, via the object\'s own parallel-NativeKind track / heap_mask / capture layout) into ONE primitive: for_each_heap_child(ptr, kind, |child_ptr, child_kind|). The DESTRUCTIVE Drop path must be refactored to CALL this primitive (releasing each yielded child), and a NEW read-only GC visitor must call the SAME primitive (reading each child). Same edge set, one source of truth — they cannot drift.',
  '  - Dispatch on HeapKind/HeapValue ONLY. NO is_heap() probe, NO tag decode, NO ValueWord, NO parallel discriminator (all FORBIDDEN — CLAUDE.md). Use the existing parallel-NativeKind tracks / heap_mask / capture layout to find edges.',
  '  - Prove parity: wire the gc_barrier_debug harness (BARRIER_COUNT vs HEAP_WRITE_COUNT, or an equivalent edge-count assertion) so the read-only visitor yields EXACTLY the edge set the destructive Drop path releases. This parity gate is the deliverable that makes the later collector safe.',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO behavior change this lane (Drop semantics byte-identical; feature-off = no-op). NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator). ADR-005 §1 single-discriminator preserved (gc_meta + visitor dispatch on HeapKind, no new sum type). ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. HeapHeader offset/DATA_OFFSET tests unchanged. Do NOT implement the collector, barriers-with-real-bodies, or any Drop-on-collect (that is Phase 2+, and per §0 ratification the GC is memory-only — it will NEVER run Drop on cycle members).',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Phase0')
const P0_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'no_behavior_change', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'flag consts + gc_meta + side-table type + gc feature, brief' },
    no_behavior_change: { type: 'boolean', description: 'true iff feature-off is a strict no-op and header offset tests unchanged' },
    evidence: { type: 'string', description: 'check-clean EXIT + header offset tests green; captured' },
  },
}
const p0 = await agent(CTX + '\n\nPHASE 0 ONLY. Add the flag-bit constants + gc_meta accessor + empty header-less side-table type + the gc Cargo feature (default off). Confirm feature-off is a no-op and HeapHeader offset/DATA_OFFSET tests are byte-identical. ' + DX + ' just check-clean EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C Phase 0: GC metadata + gc_meta accessor + gc feature (no behavior)").',
  { label: 'phase0', phase: 'Phase0', effort: 'high', schema: P0_SCHEMA })

phase('Phase1')
const P1_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'primitive', 'drop_refactored', 'parity_proven', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    primitive: { type: 'string', description: 'for_each_heap_child signature + where it lives, brief' },
    drop_refactored: { type: 'string', description: 'which destructive Drop walks now call the shared primitive' },
    parity_proven: { type: 'boolean', description: 'true iff the read-only visitor yields exactly the Drop-path edge set (coverage harness green)' },
    evidence: { type: 'string', description: 'parity/coverage result + all Drop tests green + check-no-dynamic EXIT; captured' },
  },
}
const p1 = await agent(CTX + '\n\nPHASE 0: ' + JSON.stringify(p0) + '\n\nPHASE 1 ONLY. Extract for_each_heap_child as the single shared edge-enumeration primitive; refactor the destructive Drop walks to call it; add the read-only GC visitor that calls the SAME primitive; prove edge parity via the coverage harness. Drop semantics MUST stay byte-identical (all existing Drop/refcount tests green). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C Phase 1: shared for_each_heap_child edge primitive + read-only GC visitor + parity gate").',
  { label: 'phase1', phase: 'Phase1', effort: 'high', schema: P1_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'no_behavior_change', 'edge_parity', 'no_forbidden', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    no_behavior_change: { type: 'boolean', description: 'Drop semantics byte-identical; feature-off no-op; all Drop/refcount tests green (re-run yourself)' },
    edge_parity: { type: 'boolean', description: 'read-only visitor yields exactly the destructive Drop edge set (you re-checked)' },
    no_forbidden: { type: 'boolean', description: 'no is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator; ADR-005 single-discriminator intact (grep the diff)' },
    evidence: { type: 'string', description: 'your own from-scratch checks: offset tests, Drop tests, parity, grep; concise' },
  },
}
const verify = await agent(CTX + '\n\nPHASE0: ' + JSON.stringify(p0) + '\nPHASE1: ' + JSON.stringify(p1) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) confirm ZERO behavior change — HeapHeader offset/DATA_OFFSET tests byte-identical, feature-off is a no-op, all existing Drop/refcount tests green. (2) confirm the read-only visitor and the destructive Drop path enumerate the SAME edge set (the parity gate is real, not vacuous — check it actually exercises TypedObject + closure + container + TypedArray edges). (3) grep the diff for is_heap/tag_bits/ValueWord/synthesize_value_word/Bool-default/a new sum type projecting to HeapKind — any hit = REFUTED. Any behavior change, parity gap, or forbidden pattern = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'parity_test', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    parity_test: { type: 'string', description: 'the parity/coverage test name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nPHASE1: ' + JSON.stringify(p1) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure a persistent parity/coverage test exists (read-only visitor edge set == Drop-path edge set) and NO new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast. Commit (git commit --no-verify -m "WF-3C Phase 0/1 finalize: GC prep + edge-parity gate").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { p0, p1, verify, finish }
