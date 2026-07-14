export const meta = {
  name: 'wf-jit-typed-pointer-migration',
  description: 'USER-DIRECTED (2026-07-07): the JIT typed-pointer migration — the hard prerequisite that unblocks GC 3c (JIT-cycle collection) and full gc-on. The JIT stores heap objects as its OWN struct (`Box::into_raw(UnifiedValue<*const u8>)`: u32 manual refcount + inline u64 field cells + JIT-private byte-offset addressing, control block at +4) whereas the VM v2 carrier is `Arc<TypedObjectStorage>` (schema_id + Vec<u64> slots + Vec<NativeKind> field_kinds + heap_mask, Arc control block at -16). The GC barrier/collector (`gc_decrement_precheck`/`cycle_capable_direct_header`/`for_each_heap_child`) operate on the v2 HeapHeader layout, so they read a JIT carrier at the WRONG offsets = unsound (this is exactly why GC 3c was blocked). The W17 audit (docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md, 2026-05-13) SURFACED this as ADR-006 §2.3 amendment territory (structurally divergent payload shapes) with a 17+-consumer inventory. This is a LARGE architectural campaign — so DESIGN-FIRST with a POC-VALIDATED slice before mass migration: scope the full carrier + consumer inventory, design the unification (JIT produces/consumes `Arc<TypedObjectStorage>` — the v2 carrier — NOT a kind-injection bridge, which the audit §1.3 flags as a §2.7.10/Q11-family defection-attractor) + draft the ADR-006 §2.3 amendment, VALIDATE it by migrating ONE representative producer+consumer end-to-end (proving gc-off JIT byte-identical AND the GC barrier now soundly reads the migrated carrier), then execute the validated first phase + report the phased plan for the rest. Work in an ISOLATED worktree. Independent Opus verify. gc feature off by default (mistakes ship as no-ops), but JIT correctness (gc-off) must be byte-identical.',
  phases: [
    { title: 'Scope', detail: 'full carrier + 17+ consumer inventory; exact layout divergence; what the GC barrier needs from the migrated carrier' },
    { title: 'Design', detail: 'unification architecture (JIT uses Arc<TypedObjectStorage>, no bridge) + ADR-006 §2.3 amendment draft + phased plan + risk' },
    { title: 'POC', detail: 'migrate ONE representative producer+consumer end-to-end; prove gc-off byte-identical + GC barrier soundly reads it' },
    { title: 'Execute+Assess', detail: 'execute the validated first phase; verify; report the phased plan + ADR decision + remaining site count for review' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-jitmig'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/jit-typed-pointer-migration, off main HEAD — ISOLATED worktree). Build/test via: ' + DX + ' <cmd> (devenv toolchain not auto-loaded). This is a LARGE, ARCHITECTURALLY-SIGNIFICANT migration — precision + design-first discipline matter more than speed.',
  '',
  'REQUIRED READING (read these fully before proposing anything): docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md (the 2026-05-13 surface-and-stop audit: §1.3 THE GAP layout divergence, §1.4 the load-bearing receiver_type_name surface, §1.5 the ADR amendment area, §2 the 17+ consumer inventory), docs/cluster-audits/wave-10-jit-playbook.md (§5), docs/adr/006-value-and-memory-model.md (§2.3 typed-Arc payloads, §2.7.5 stamp-at-compile-time, §2.7.7 parallel-kind track, §2.7.10/Q11 the deleted kind-blind ABI defection family), docs/runtime-v2-spec.md, and the GC design docs/design/real-gc-cycle-collection.md (why the barrier needs a v2-HeapHeader carrier).',
  '',
  'THE PROBLEM: the JIT-internal TypedObject carrier (produced by box_typed_object at crates/shape-jit/src/ffi/value_ffi.rs:516 as Box::into_raw(UnifiedValue<*const u8>); consumers in crates/shape-jit/src/ffi/typed_object/{mod,merge_ops,allocation,ffi_exports,field_access}.rs) has a STRUCTURALLY DIVERGENT layout from the VM v2 carrier Arc<TypedObjectStorage> (crates/shape-value/src/heap_value.rs:2356): u32 manual refcount + inline u64 cells + JIT-private byte offsets (control block +4) vs Arc-wrapped {schema_id, slots:Vec<u64>, field_kinds:Vec<NativeKind>, heap_mask} (Arc control block -16). The GC barrier + collector read the v2 HeapHeader layout; on a JIT carrier they read garbage → unsound. GOAL: migrate the JIT to PRODUCE + CONSUME Arc<TypedObjectStorage> (the v2 carrier) so the GC barrier operates soundly AND the JIT hot paths stay correct + fast.',
  '',
  'HARD DESIGN CONSTRAINTS (CLAUDE.md + the audit, CRITICAL):',
  '  - The carrier MUST be the real v2 Arc<TypedObjectStorage> (ADR-005 §1 single-discriminator). NO parallel/JIT-private discriminator; NO kind-injection helper across the crate boundary (audit §1.3 flags this as the §2.7.10/Q11 kind-blind-ABI defection-attractor — refuse on sight). NO ValueWord/Bool-default/tag-decode/is_heap. NO Convert*To bridge opcode.',
  '  - Either the JIT allocates Arc<TypedObjectStorage> directly (preferred — one carrier, no conversion) OR, if a genuine hot-path perf reason forbids that, SURFACE the ADR fork (do NOT silently keep two carriers + a bridge). The audit\'s own recommendation is the Arc<TypedObjectStorage> carrier; a "documented dual carrier" is a defection per CLAUDE.md §Parallel-implementation.',
  '  - gc-OFF JIT behavior must be BYTE-IDENTICAL where possible / at minimum semantically identical + no perf regression on the hot path; the migration is about the CARRIER, tested with gc off first.',
  '  - ADR-006 §2.3 AMENDMENT is required (the payload-shape unification) — draft it; it is a real architecture decision → surface it clearly for supervisor/user ratification, do NOT self-approve a contested fork.',
  '',
  'PHASE 1 SCOPE (no code changes): produce the FULL inventory — every JIT producer of a TypedObject carrier + every consumer (the 17+ from audit §2 + a fresh grep of crates/shape-jit for box_typed_object / UnifiedValue<*const> TypedObject / inc_ref/dec_ref JIT helpers / byte-offset field access), the exact layout divergence, and precisely what the GC barrier (cycle_capable_direct_header / for_each_heap_child) needs from the migrated carrier. Classify each site (producer / field-read / field-write / refcount / method-dispatch / alloc).',
  '',
  'PHASE 2 DESIGN (no code changes): the unification architecture — HOW the JIT produces + consumes Arc<TypedObjectStorage> (allocation path, field addressing = Vec index vs byte offset, refcount = Arc vs manual, method dispatch, cross-crate FFI ABI staying §2.7.5-clean). Draft the ADR-006 §2.3 amendment. The phased migration plan (which sites in which order, dependency edges). Risk assessment + the perf story on the hot path. If a genuinely-contested fork exists (e.g. Arc-alloc-in-JIT is measurably too slow) → flag it as needs-ratification.',
  '',
  'PHASE 3 POC (code, in the isolated worktree): migrate ONE representative TypedObject producer + its consumers end-to-end to Arc<TypedObjectStorage>. PROVE: (a) gc-off JIT behavior unchanged (the migrated path runs correct + the relevant shape-jit/shape-vm tests green); (b) with gc on, the GC barrier (cycle_capable_direct_header) now reads the migrated carrier SOUNDLY (a JIT-produced TypedObject cycle can be buffered + collected — the thing 3c could not do). This VALIDATES the architecture. If the POC reveals the design is wrong, iterate the design, do NOT proceed.',
  '',
  'PHASE 4 EXECUTE+ASSESS: execute the validated first migration phase (the POC + its immediate cluster), run gates, then REPORT: the validated architecture + ADR amendment, the POC/first-phase result, the remaining site count + phased plan for the rest, and any ADR fork needing ratification. This is a design-validate-and-begin close — mass migration of all remaining sites is a follow-up (stay-in-the-loop).',
  '',
  'CONSTRAINTS: ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0 at each committed step. JIT-heavy tests SIGILL-race at high parallelism → --test-threads=1. Commit incrementally with clear messages.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Scope')
const SCOPE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['carrier_inventory', 'layout_divergence', 'gc_barrier_needs', 'site_count'],
  properties: {
    carrier_inventory: { type: 'string', description: 'the JIT TypedObject producers + consumers found, classified (producer/read/write/refcount/dispatch/alloc)' },
    layout_divergence: { type: 'string', description: 'the exact JIT-internal vs Arc<TypedObjectStorage> layout differences that break the GC barrier' },
    gc_barrier_needs: { type: 'string', description: 'precisely what cycle_capable_direct_header / for_each_heap_child require from the migrated carrier' },
    site_count: { type: 'string', description: 'total producer + consumer site count + rough grouping' },
  },
}
const scope = await agent(CTX + '\n\nPHASE 1 — SCOPE ONLY (no code). Full carrier + consumer inventory + layout divergence + GC-barrier requirements + site count. Do NOT commit.',
  { label: 'scope', phase: 'Scope', effort: 'xhigh', schema: SCOPE_SCHEMA })

phase('Design')
const DESIGN_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['architecture', 'adr_amendment', 'phased_plan', 'contested_fork', 'risk'],
  properties: {
    architecture: { type: 'string', description: 'HOW the JIT produces+consumes Arc<TypedObjectStorage>: alloc, field addressing, refcount, dispatch, FFI ABI — no bridge, no dual carrier' },
    adr_amendment: { type: 'string', description: 'the ADR-006 §2.3 amendment draft (payload-shape unification)' },
    phased_plan: { type: 'string', description: 'ordered migration phases across the sites + dependency edges' },
    contested_fork: { type: 'string', description: 'any genuinely-contested architecture decision needing ratification (or "none — clean unification path")' },
    risk: { type: 'string', description: 'hot-path perf story + the main correctness risks' },
  },
}
const design = await agent(CTX + '\n\nSCOPE: ' + JSON.stringify(scope) + '\n\nPHASE 2 — DESIGN ONLY (no code). The unification architecture + ADR-006 §2.3 amendment draft + phased plan + contested-fork flag + risk. Prefer JIT-allocates-Arc<TypedObjectStorage>; refuse the kind-injection bridge. Do NOT commit.',
  { label: 'design', phase: 'Design', effort: 'xhigh', schema: DESIGN_SCHEMA })

phase('POC')
const POC_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'site_migrated', 'gc_off_identical', 'gc_barrier_sound', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['validated', 'design-wrong', 'blocked'] },
    site_migrated: { type: 'string', description: 'the representative producer+consumer migrated to Arc<TypedObjectStorage>' },
    gc_off_identical: { type: 'boolean', description: 'gc-off JIT behavior unchanged on the migrated path (tests green)' },
    gc_barrier_sound: { type: 'boolean', description: 'with gc on, cycle_capable_direct_header now reads the migrated carrier soundly — a JIT-produced TypedObject can be buffered/collected' },
    evidence: { type: 'string', description: 'captured: migrated-path tests green gc-off; the GC barrier reads the real v2 header; check-no-dynamic EXIT 0' },
  },
}
const poc = await agent(CTX + '\n\nSCOPE: ' + JSON.stringify(scope) + '\nDESIGN: ' + JSON.stringify(design) + '\n\nPHASE 3 — POC (code, isolated worktree). Migrate ONE representative TypedObject producer+consumer to Arc<TypedObjectStorage>; prove gc-off byte-identical AND the GC barrier soundly reads it. If design-wrong, say so + why (do NOT force). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF JIT-typed-pointer-migration POC: migrate one TypedObject carrier to Arc<TypedObjectStorage> (validate GC-barrier soundness + gc-off parity)").',
  { label: 'poc', phase: 'POC', effort: 'xhigh', schema: POC_SCHEMA })

phase('Execute+Assess')
const EXEC_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'phase1_done', 'verify', 'remaining_plan', 'adr_ratification_needed', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['first-phase-done', 'design-validated-only', 'blocked'] },
    phase1_done: { type: 'string', description: 'the validated first migration phase executed (sites migrated)' },
    verify: { type: 'string', description: 'independent-check result: gc-off byte-identical + gc-on barrier-sound + no regression' },
    remaining_plan: { type: 'string', description: 'the remaining site count + phased plan for the rest (follow-up workflows)' },
    adr_ratification_needed: { type: 'string', description: 'the ADR-006 §2.3 amendment + any contested fork needing user/supervisor ratification' },
    evidence: { type: 'string', description: 'gates: check-clean + check-no-dynamic EXIT 0; shape-jit/shape-vm tests --test-threads=1; brief' },
  },
}
const exec = await agent(CTX + '\n\nSCOPE: ' + JSON.stringify(scope) + '\nDESIGN: ' + JSON.stringify(design) + '\nPOC: ' + JSON.stringify(poc) + '\n\nPHASE 4 — EXECUTE the validated first phase (only if POC status==validated; else stop at design-validated-only + report why), run gates + an independent correctness check (gc-off byte-identical, gc-on barrier-sound, no regression), and report the remaining phased plan + the ADR amendment for ratification. Add regression tests for the migrated sites. ' + DX + ' just check-clean + check-no-dynamic EXIT 0; shape-jit/shape-vm tests --test-threads=1. Commit (git commit --no-verify -m "WF JIT-typed-pointer-migration phase-1: migrate first carrier cluster to Arc<TypedObjectStorage> + tests").',
  { label: 'execute-assess', phase: 'Execute+Assess', effort: 'xhigh', schema: EXEC_SCHEMA })

return { scope, design, poc, exec }
