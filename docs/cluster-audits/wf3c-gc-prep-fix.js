export const meta = {
  name: 'wf3c-gc-prep-correct-complete',
  description: 'Correct + complete WF-3C GC Phase 0/1 (prior run was PARTIAL/merge_ready:false). Two fixes: (A) Phase 0 put GC color bits on the WRONG header (v1 crates/shape-value/src/heap_header.rs) — the runtime carrier is the v2 header crates/shape-value/src/v2/heap_header.rs, where bit 3 is FLAG_CLOSURE_CAPTURES_DROPPED; move GC color to v2-header bits 4-5 + buffered bit 6. (B) Phase 1 only structurally unified TypedObject; TypedArray + Closure destructive release are still parallel MIRROR walks with empirical-only parity (the exact §3.4 drift hazard). Reroute BOTH through the same shared yield-edges + per-kind-release-helper pattern so all header carriers share ONE source of truth. Container-internal edges (HashMap/HashSet/Deque, closure OwnedMutable/Shared) explicitly deferred to the Phase 3.5 side-table with a documented boundary. Independent Opus verify.',
  phases: [
    { title: 'RetargetHeader', detail: 'move GC metadata from v1 header to v2 header bits 4-6; gc_meta reads/writes v2 flags' },
    { title: 'CompleteParity', detail: 'reroute TypedArray + Closure destructive release through the shared primitive' },
    { title: 'Verify', detail: 'independent Opus: correct header, all 3 carriers single-source, no behavior change' },
    { title: 'Finish', detail: 'gates + parity tests for all 3 carriers + documented container deferral' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w5-gcprep'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave5/gc-prep, HEAD 4cf13685 — has the PARTIAL Phase 0/1). Build/test via: ' + DX + ' <cmd>. Ratified design: docs/design/real-gc-cycle-collection.md (§0 ratification, §3.1 CORRECTED carrier note, §3.4 shared primitive, §7 metadata placement).',
  '',
  'The prior Phase 0/1 run was PARTIAL (merge_ready:false). Two concrete corrections, then it merges:',
  '',
  'CORRECTION A — WRONG HEADER. Phase 0 added the GC color/buffered constants + GcColor + gc_meta to crates/shape-value/src/heap_header.rs (the V1 header). But the RUNTIME carrier is crates/shape-value/src/v2/heap_header.rs (the V2 header) — every v2-raw carrier (TypedObject, TypedArray, Closure, StringV2, DecimalV2, TraitObject) embeds crate::v2::heap_header::HeapHeader (33 refs); the v1 header has ZERO runtime-carrier uses. So the GC metadata is on a struct the runtime never reads. FIX: move the GC color/buffered constants + GcColor + the header-carrier branch of gc_meta onto the V2 header. On the v2 header .flags (offset 6): bits 0-2 = FLAG_MARKED/PINNED/READONLY, bit 3 (0x08) = FLAG_CLOSURE_CAPTURES_DROPPED (already taken). Put GC COLOR in bits 4-5 (0x10|0x20) and BUFFERED in bit 6 (0x40); bit 7 free. This avoids the FLAG_CLOSURE_CAPTURES_DROPPED collision with NO reconciliation. Remove the now-misplaced additions from the v1 header (leave the v1 header byte-identical to main). Keep the `gc` Cargo feature default-off; feature-off still a strict no-op; v2-header offset/DATA_OFFSET tests byte-identical.',
  '',
  'CORRECTION B — INCOMPLETE STRUCTURAL PARITY. Phase 1 made ONLY TypedObject a true single source of truth (drop_fields + the visitor both call TypedObjectStorage::for_each_heap_child_edge, releasing via release_one_field). But the TypedArray destructive path (release_v2_typed_array / drop_array_heap) and the Closure destructive path (release_typed_closure, three-mask) do NOT call the shared primitive — they are parallel MIRROR walks whose parity is proven only by an empirical test. That is exactly the drift hazard §3.4 mandates eliminating STRUCTURALLY. FIX: extend the same yield-edges + per-kind-release-helper pattern to TypedArray and Closure — the destructive release path must ENUMERATE via the shared primitive and RELEASE each yielded edge via a per-kind release helper, so the read-only GC visitor and the destructive Drop path share ONE enumeration function per carrier and cannot drift. Preserve byte-identical Drop semantics (enumeration order, filters, Miri provenance of each release unchanged); the generic TypedArray<T> and layout-driven closure release make this delicate — do it carefully, do NOT change what gets released or in what order.',
  '',
  'SCOPE BOUNDARY (document, do not silently skip): container-INTERNAL edges (HashMap/HashSet/Deque values) and closure OwnedMutable/Shared INTERIOR captures are header-less std-Arc participants — their edge enumeration belongs to the Phase 3.5 side-table mechanism (§3.5). It is CORRECT to defer them here, but you MUST document the boundary explicitly (a comment + the finish note) so it is a known deferral, not a silent parity gap. The header carriers (TypedObject, TypedArray, Closure immutable captures, and any other v2 HeapHeader carrier with heap children) MUST all be single-source-of-truth this lane.',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO behavior change (Drop byte-identical; feature-off no-op). NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator projecting 1:1 to HeapKind). ADR-005 §1 single-discriminator preserved. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. v2-header offset/DATA_OFFSET tests unchanged. NO collector, NO real barrier bodies, NO Drop-on-collect (Phase 2+; and per §0 the GC is memory-only — never runs Drop on cycle members).',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('RetargetHeader')
const A_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'v2_bits', 'v1_reverted', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    v2_bits: { type: 'string', description: 'GC color/buffered now at which v2-header bits; gc_meta reads/writes v2 flags' },
    v1_reverted: { type: 'boolean', description: 'true iff the v1 header is back to byte-identical-with-main (no GC additions left there)' },
    evidence: { type: 'string', description: 'v2-header offset tests byte-identical; feature-off no-op; check-clean EXIT; captured' },
  },
}
const a = await agent(CTX + '\n\nCORRECTION A ONLY. Move GC metadata to the v2 header (bits 4-5 color, bit 6 buffered); retarget gc_meta; revert the v1-header additions to byte-identical-with-main. Confirm v2-header offset/DATA_OFFSET tests unchanged + feature-off no-op. ' + DX + ' just check-clean EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C prep-fix A: GC metadata on v2 header bits 4-6 (correct runtime carrier)").',
  { label: 'retarget-header', phase: 'RetargetHeader', effort: 'high', schema: A_SCHEMA })

phase('CompleteParity')
const B_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'carriers_unified', 'container_deferral_documented', 'byte_identical', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    carriers_unified: { type: 'string', description: 'which header carriers now share ONE enumeration primitive between Drop + visitor (target: TypedObject + TypedArray + Closure)' },
    container_deferral_documented: { type: 'boolean', description: 'true iff the header-less container/interior-capture deferral to Phase 3.5 is documented (comment + note), not silent' },
    byte_identical: { type: 'boolean', description: 'Drop semantics byte-identical (order/filters/provenance); all Drop/refcount tests green feature-off' },
    evidence: { type: 'string', description: 'parity tests for all 3 carriers green; Drop tests green; check-no-dynamic EXIT; captured' },
  },
}
const b = await agent(CTX + '\n\nCORRECTION A: ' + JSON.stringify(a) + '\n\nCORRECTION B ONLY. Reroute the TypedArray + Closure destructive release paths through the shared yield-edges + per-kind-release-helper pattern (same as TypedObject) so all header carriers are single-source-of-truth; add parity tests for TypedArray + Closure matching the TypedObject one; document the container/interior-capture deferral to Phase 3.5. Preserve byte-identical Drop. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C prep-fix B: TypedArray + Closure destructive release share the GC edge primitive (structural parity)").',
  { label: 'complete-parity', phase: 'CompleteParity', effort: 'high', schema: B_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'correct_header', 'all_carriers_single_source', 'no_behavior_change', 'no_forbidden', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    correct_header: { type: 'boolean', description: 'GC metadata is on the v2 runtime header (bits 4-6), v1 header byte-identical-with-main, no FLAG_CLOSURE_CAPTURES_DROPPED collision' },
    all_carriers_single_source: { type: 'boolean', description: 'TypedObject + TypedArray + Closure destructive release ALL call the shared primitive (not mirror walks); container deferral documented' },
    no_behavior_change: { type: 'boolean', description: 'Drop byte-identical; feature-off no-op; v2 offset tests + all Drop/refcount tests green (re-run yourself)' },
    no_forbidden: { type: 'boolean', description: 'no is_heap/tag-decode/ValueWord/Bool-default/1:1-HeapKind sum type (grep the diff)' },
    evidence: { type: 'string', description: 'your own from-scratch checks: header target, per-carrier single-source, offset+Drop tests, grep; concise' },
  },
}
const verify = await agent(CTX + '\n\nCORRECTION A: ' + JSON.stringify(a) + '\nCORRECTION B: ' + JSON.stringify(b) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) GC color/buffered live on the v2 header (crate::v2::heap_header) at bits 4-6, NOT the v1 header, and the v1 header is byte-identical with main; no overlap with FLAG_CLOSURE_CAPTURES_DROPPED. (2) For EACH of TypedObject, TypedArray, Closure, the DESTRUCTIVE release path and the read-only GC visitor call the SAME enumeration primitive (open the code — a mirror walk that merely has a passing test is REFUTED); the container/interior-capture deferral is documented. (3) ZERO behavior change: v2 offset/DATA_OFFSET tests byte-identical, feature-off no-op, all Drop/refcount tests green. (4) grep the diff for forbidden patterns. Any wrong-header, any remaining mirror walk on a header carrier, any behavior change, or any forbidden pattern = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'parity_tests', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-value tests (feature-off + feature-on), brief' },
    parity_tests: { type: 'string', description: 'the per-carrier parity tests (TypedObject/TypedArray/Closure) names + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nCORRECTION B: ' + JSON.stringify(b) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure per-carrier parity tests exist for all three header carriers and NO new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-value (feature-off) and ' + DX + ' cargo test -p shape-value --features gc. Commit (git commit --no-verify -m "WF-3C prep-fix finalize: correct v2-header GC metadata + full header-carrier edge parity").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { a, b, verify, finish }
