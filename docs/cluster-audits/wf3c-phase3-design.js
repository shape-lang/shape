export const meta = {
  name: 'wf3c-phase3-design-refine',
  description: 'WF-3C GC Phase 3 DESIGN-REFINE (design-only, no merge). Before building the collector + the user-required multi-thread rendezvous, map the ACTUAL runtime concurrency + heap-sharing model and pin exactly what the STW rendezvous must cover. Key uncertainty: the async fix uses ISOLATED per-task VMs (separate heaps, scalar-only marshaling), and JIT COMPILATION runs on background worker threads while JIT-compiled CODE executes on the VM thread — so it is not obvious that any two threads share a MUTABLE heap graph. Determine who shares a mutable heap with whom across threads; define the concrete rendezvous; specify CollectCycles (Bacon-Rajan trial deletion, memory-only per §0 — no Drop on cycle members) + the JIT old-kind-tag threading deferred from Phase 2. Write a refined Phase 3 plan to the design doc.',
  phases: [
    { title: 'Survey', detail: 'map the real thread/heap-sharing model + JIT tag gap + safepoint reachability' },
    { title: 'Refine', detail: 'define the concrete rendezvous scope + CollectCycles integration + write the plan' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape'
const DX = 'direnv exec /home/dev/dev/shape-lang'
const DOC = 'docs/design/real-gc-cycle-collection.md'

const CTX = [
  'Work IN the MAIN checkout ' + WT + ' (READ-ONLY except appending a Phase 3 section to ' + DOC + '). DESIGN pass — do NOT change runtime code, do NOT merge. Build only to confirm a claim.',
  '',
  'CONTEXT: The ratified cycle-collector design (' + DOC + ' §0) requires (user ruling) that the MULTI-THREAD stop-the-world rendezvous ships in v1 (no single-VM-first shortcut) so SharedAtomic cross-thread cycles are collectable. Phases 0/1/2 are MERGED (metadata on v2 HeapHeader bits 4-6; shared for_each_heap_child edge primitive per carrier; RC barriers + candidate buffer, behind default-off `gc` feature; collection still a no-op). Phase 3 is the actual collector + the rendezvous.',
  '',
  'THE DESIGN QUESTION TO RESOLVE: what does the STW rendezvous concretely need to halt, given the ACTUAL runtime? Investigate and answer precisely:',
  '  1. THREADS that touch heap objects: the main VM dispatch loop; the async scheduler (crates/shape-vm/src/executor/async_runtime.rs — WF-2D-fu runs zero-arg scalar async fns on ISOLATED VirtualMachines on a blocking pool; do those isolated VMs SHARE any Arc heap object with the parent, or are their heaps fully separate + only scalars marshal back?); the JIT tiering worker threads (crates/shape-jit worker.rs — do they only COMPILE (touching bytecode, not live heap objects), or do they mutate/read the live heap graph?); any other thread.',
  '  2. SHARED MUTABLE HEAP: is there ANY path today where two OS threads hold Arc<HeapValue>/HeapHeader pointers to the SAME heap object graph and can mutate it concurrently? (SharedAtomic / SharedAtomicMut storage classes exist in the lattice — are they actually EXERCISED by real cross-thread sharing, or latent?) This decides whether the rendezvous must be a true cross-heap STW or just a same-thread safepoint + per-isolated-VM-local collection.',
  '  3. SAFEPOINT reachability: jit_gc_safepoint (Phase 2 gave it a real poll, flag unraised) is emitted at loop back-edges; the VM dispatch loop can check a flag at the top. Can every mutator thread reach a safepoint in bounded time (no unbounded native call holding heap refs)? Where are the gaps?',
  '  4. JIT old-kind-tag gap (Phase 2 deferred): 3 JIT store sites pass old_kind_tag=0 to jit_write_barrier, so a heap-field overwrite by JIT code does not buffer the old occupant as a decrement candidate. What is needed to thread the real per-field old kind at those sites, and is it required for soundness (can a cycle form purely through JIT-compiled writes)?',
  '',
  'THEN DEFINE (write to ' + DOC + ' as a new "## Phase 3 (refined)" section):',
  '  - The CONCRETE rendezvous: given the real model, exactly which threads halt and how (flag + poll at VM loop top + jit_gc_safepoint + isolated-task-VM quiescence). If the real model has NO shared mutable cross-thread heap, say so explicitly and define the correct minimal safety (e.g. collect per-VM at a local safepoint; the "MT rendezvous" reduces to ensuring no isolated task VM / JIT thread is mid-heap-op) — and flag that this satisfies the user requirement\'s INTENT (cross-thread cycles collectable) because there are no cross-thread shared cycles to miss. If therefore a broader rendezvous is premature, RECOMMEND the correct v1 scope and note what would trigger needing the full STW (when real SharedAtomic cross-thread sharing lands).',
  '  - CollectCycles: the Bacon-Rajan 3-pass (MarkRoots/ScanRoots/CollectRoots via MarkGray/Scan/ScanBlack/CollectWhite) over the Phase-2 candidate buffer, using the true HeapHeader.refcount + the for_each_heap_child visitor; MEMORY-ONLY reclaim (per §0: NO Drop on cycle members — CollectWhite frees memory, runs no finalizers); header-less side-table shadow-count for SharedCell/Reference/containers.',
  '  - The JIT old-kind-tag threading fix (or why it can be deferred safely).',
  '  - A concrete sub-phase breakdown for the IMPLEMENTATION (3a CollectCycles single-VM; 3b rendezvous/quiescence; 3c JIT tag) with gates.',
  '',
  'CONSTRAINTS: respect ADR-006 forbidden patterns (no is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator); the collector uses HeapKind-dispatch + the existing parallel-kind track. Memory-only (no Drop-on-collect). Non-moving.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Survey')
const SURVEY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['area', 'finding', 'implication'],
  properties: {
    area: { type: 'string', description: 'which survey area' },
    finding: { type: 'string', description: 'what the code actually does (files/mechanisms), concrete' },
    implication: { type: 'string', description: 'what it implies for the rendezvous scope / collector' },
  },
}
const AREAS = [
  { key: 'async-isolation', prompt: 'Investigate async_runtime.rs + async_ops: do the isolated per-task VMs share ANY Arc heap object with the parent VM, or are their heaps fully separate with only scalars marshaled back? Can a heap object graph span two async task threads?' },
  { key: 'jit-threads', prompt: 'Investigate the JIT tiering worker threads (shape-jit worker.rs + how tier compilation is scheduled): do worker threads touch/mutate the LIVE heap graph, or only compile bytecode->native? Does JIT-compiled code execute on the VM thread or a separate thread?' },
  { key: 'shared-atomic', prompt: 'Investigate SharedAtomic/SharedAtomicMut storage classes + Channel/Mutex heap kinds: is there ANY exercised path where two OS threads concurrently mutate the same Arc<HeapValue> graph today, or are these latent/unused? Grep for actual cross-thread heap sharing.' },
  { key: 'safepoint-jit-tag', prompt: 'Investigate safepoint reachability (VM loop flag check + jit_gc_safepoint poll) + the Phase-2 JIT old_kind_tag=0 gap (the 3 store sites): can every mutator reach a safepoint bounded, and is threading the real old-kind-tag required for soundness (can a cycle form purely via JIT writes)?' },
]
const survey = await parallel(AREAS.map(a => () =>
  agent(CTX + '\n\nSURVEY AREA: ' + a.prompt, { label: 'survey:' + a.key, phase: 'Survey', effort: 'high', schema: SURVEY_SCHEMA })))
const surveyText = survey.filter(Boolean).map(s => '[' + s.area + '] ' + s.finding + ' || implies: ' + s.implication).join('\n\n')

phase('Refine')
const REFINE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['heap_sharing_model', 'rendezvous_scope', 'jit_tag_needed', 'subphases', 'doc_written', 'user_decision'],
  properties: {
    heap_sharing_model: { type: 'string', description: 'the ACTUAL cross-thread mutable-heap-sharing reality (shared vs isolated), concrete' },
    rendezvous_scope: { type: 'string', description: 'the concrete v1 rendezvous the real model needs (full STW vs minimal safepoint/quiescence)' },
    jit_tag_needed: { type: 'string', description: 'is threading the JIT old-kind-tag required for soundness now, or safely deferrable — why' },
    subphases: { type: 'string', description: 'the 3a/3b/3c implementation breakdown + gates, brief' },
    doc_written: { type: 'boolean', description: 'true iff the refined Phase 3 section was appended to the design doc' },
    user_decision: { type: 'string', description: 'if the real model makes the ratified full-STW-in-v1 scope premature/over-built, the precise question to put to the user; else "none — proceed as ratified"' },
  },
}
const refine = await agent(CTX + '\n\nSURVEY:\n' + surveyText + '\n\nREFINE. Given the ACTUAL model, define the concrete rendezvous scope + CollectCycles integration + JIT-tag decision + the 3a/3b/3c impl breakdown. Append a "## Phase 3 (refined)" section to ' + DOC + '. If the real heap-sharing model makes the user-ratified "full MT rendezvous in v1" premature or over-built (e.g. no cross-thread shared mutable heap exists yet), surface the precise decision for the user in user_decision (do NOT silently downscope a user ruling); else user_decision = "none — proceed as ratified".',
  { label: 'refine', phase: 'Refine', effort: 'high', schema: REFINE_SCHEMA })

return { survey, refine }
