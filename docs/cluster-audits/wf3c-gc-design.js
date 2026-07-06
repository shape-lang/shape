export const meta = {
  name: 'wf3c-real-gc-design',
  description: 'D3 (user-ruled: REAL cycle-collecting GC, not weak-refs). DESIGN-FIRST pass — produces a ratifiable design doc, NO code merge. Survey the current Arc-refcount + escape-analysis + Drop memory model (ADR-006) and the existing gc_integration.rs scaffold + the two leak findings (#31 unbounded ref-cycle leak, #82 module-scope closure-capture Drop finalizer leak §2.7.30.4). Then run an independent design panel of cycle-collection approaches scored against Shape\'s hard constraints (typed slots, snapshot-serializable, JIT/FFI carriers, ADR-006 forbidden patterns). Synthesize a recommendation + integration plan + blast radius + open questions for user ratification.',
  phases: [
    { title: 'Survey', detail: 'parallel readers: RC/escape/Drop model, gc scaffold, cycle-formation sites, snapshot/JIT constraints' },
    { title: 'Panel', detail: 'independent GC-approach proposals scored on Shape constraints' },
    { title: 'Synthesize', detail: 'recommendation + integration plan + blast radius -> design doc' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape'
const DX = 'direnv exec /home/dev/dev/shape-lang'
const DOC = 'docs/design/real-gc-cycle-collection.md'

const CTX = [
  'Work IN the MAIN checkout ' + WT + ' (READ-ONLY except the final design doc ' + DOC + '). This is a DESIGN pass — do NOT change compiler/runtime code, do NOT merge anything. Read via normal tools; build only if you need to confirm a claim.',
  '',
  'USER RULING (2026-07-06): D3 = implement a REAL cycle-collecting garbage collector (the user explicitly rejected the weaker weak-refs / explicit-cycle-breaking option). This lane DESIGNS it; a later lane implements it after user ratification.',
  '',
  'THE PROBLEM: Shape\'s value model is Arc-refcount based (ADR-006: HeapHeader with AtomicU32 refcount, typed Arc<T> payloads, refcount-on-escape, Drop trait for RAII). Plain reference counting CANNOT reclaim cycles, so:',
  '  - Finding #31: reference cycles leak unboundedly (two objects/closures referencing each other are never freed).',
  '  - Finding #82: module-scope closure-capture Drop finalizer leak (ADR-006 §2.7.30.4) — an escaping closure capture\'s Drop is deferred to program lifetime and effectively leaks.',
  '',
  'HARD CONSTRAINTS the GC design MUST satisfy (these are what make Shape-GC non-trivial — a textbook tracing GC is NOT automatically viable):',
  '  1. Typed zero-tag slots (ADR-006 / runtime-v2-spec): stack slots are raw 8-byte values with a parallel NativeKind track; a heap pointer is Ptr(HeapKind). The GC must find roots + trace heap graph WITHOUT reintroducing runtime tags / ValueWord / is_heap() probes (all FORBIDDEN — see CLAUDE.md Forbidden Patterns). It can use the existing parallel-kind tracks (Vec<NativeKind>) to identify heap slots.',
  '  2. Snapshot/resume serializability: snapshot() captures full VM state for distributed/resumable execution. The GC\'s heap graph + any GC metadata must be snapshot-serializable (or reconstructable on resume). A cycle that spans a snapshot boundary must still be collectable after resume.',
  '  3. JIT/FFI compatibility: JIT FFI carriers hold raw heap pointers; the magic-byte scheme distinguishes JitAlloc/UnifiedArray from VM Arc<HeapValue>. A moving/compacting GC would break raw pointers held by JIT code — factor this into the algorithm choice (non-moving is likely required, or a pin/safepoint scheme).',
  '  4. Coexist with the existing Arc RC + escape→RC-promotion (§2.7.8) + the ratified narrow-floor reference model (§2.7.30, PromotedCell/identity-map). The GC should collect CYCLES that RC misses, ideally WITHOUT throwing away the RC fast path (RC handles the acyclic common case cheaply). I.e. a hybrid (RC + cycle collector) is a strong candidate — but evaluate alternatives.',
  '  5. ADR-006 single-discriminator discipline (ADR-005 §1): HeapValue is the canonical discriminator; do not add a parallel sum type that projects 1:1 to HeapKind. GC metadata must live in HeapHeader (it has spare flag bits) or a side table, not a new discriminator.',
  '  6. Determinism option: the Deterministic sandbox permission implies reproducible execution; consider whether GC timing must be deterministic under that flag.',
  '',
  'ANCHORS to read: docs/adr/006-value-and-memory-model.md (esp. §2.3 typed Arc payloads, §2.7.7 parallel-kind track, §2.7.8 cell-storage, §2.7.30 reference/escape/Drop, §2.7.30.4 finalizer leak); crates/shape-value/src/heap_header.rs (HeapHeader layout + spare flags); crates/shape-vm/src/executor/gc_integration.rs (existing scaffold — what is already there?); crates/shape-value/src/reference.rs; the escape→RC promotion in crates/shape-vm/src/mir/storage_planning.rs (~928-959); crates/shape-value/src/shape_graph_current.rs.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Survey')
const SURVEY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['area', 'findings', 'constraints_for_gc'],
  properties: {
    area: { type: 'string', description: 'which survey area you covered' },
    findings: { type: 'string', description: 'what exists today in that area, concrete (files/types/mechanisms)' },
    constraints_for_gc: { type: 'string', description: 'the hard constraints this area imposes on a GC design' },
  },
}
const SURVEY_AREAS = [
  { key: 'rc-drop-escape', prompt: 'Survey the Arc-refcount + Drop + escape-analysis model: HeapHeader refcount, where clone/drop_with_kind happen, escape->RC promotion (storage_planning.rs), BindingStorageClass (SharedCow/SharedAtomic). How are objects freed today, and exactly where does a cycle escape collection?' },
  { key: 'gc-scaffold', prompt: 'Survey the EXISTING gc_integration.rs scaffold + any related GC types/flags. What is already built (roots, safepoints, mark bits, anything)? Is it inert or partially wired? What HeapHeader spare flag bits exist for GC metadata?' },
  { key: 'cycle-sites', prompt: 'Enumerate the concrete cycle-formation sites in Shape: mutually-referencing closures (closure cells capturing each other), TypedObject fields holding refs that form cycles, SharedCow/SharedCell graphs, module-scope bindings (finding #82). Give minimal Shape programs that create a leaking cycle today.' },
  { key: 'snapshot-jit', prompt: 'Survey snapshot/resume serialization + JIT/FFI carrier pointer handling. What would a moving/compacting GC break (JIT raw pointers, magic-byte scheme)? How must GC state be snapshot-serialized so a cross-snapshot cycle is still collectable on resume? Is non-moving mandatory?' },
]
const survey = await parallel(SURVEY_AREAS.map(a => () =>
  agent(CTX + '\n\nSURVEY AREA: ' + a.prompt, { label: 'survey:' + a.key, phase: 'Survey', effort: 'high', schema: SURVEY_SCHEMA })))
const surveyText = survey.filter(Boolean).map(s => '[' + s.area + '] ' + s.findings + ' || GC-constraints: ' + s.constraints_for_gc).join('\n\n')

phase('Panel')
const PROPOSAL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['approach', 'algorithm', 'integration', 'blast_radius', 'soundness', 'weaknesses'],
  properties: {
    approach: { type: 'string', description: 'name of the GC approach you are proposing' },
    algorithm: { type: 'string', description: 'the core algorithm + how it finds roots/traces heap using the typed parallel-kind track (no runtime tags)' },
    integration: { type: 'string', description: 'how it coexists with Arc RC + escape/Drop + snapshot + JIT carriers' },
    blast_radius: { type: 'string', description: 'files/subsystems touched; is it non-moving; new HeapHeader bits/side tables' },
    soundness: { type: 'string', description: 'why it respects ADR-006 forbidden patterns + single-discriminator + snapshot-serializability' },
    weaknesses: { type: 'string', description: 'honest downsides / risks / where it could go wrong' },
  },
}
const APPROACHES = [
  'Bacon-Rajan synchronous trial-deletion CYCLE COLLECTOR layered on the existing Arc RC (RC keeps the acyclic fast path; candidate roots buffered on decrement; periodic mark-gray/scan-black cycle collection). Argue it as the recommended default and stress-test it against snapshot + JIT.',
  'A full tracing mark-sweep GC that AUGMENTS (or replaces) RC for heap objects: precise roots from the stack parallel-kind track + closure cells + module bindings; non-moving sweep. Compare cost/benefit vs keeping RC.',
  'Deferred / coalesced reference counting with a backup cycle collector, optimizing the RC write-barrier cost; evaluate whether the added complexity beats Bacon-Rajan.',
  'The WEAKER baseline the user rejected (Weak<T> refs + explicit/compiler-inserted cycle-breaking): design it honestly ONLY as a comparison baseline, and articulate precisely why it fails the user requirement (what real cycles it cannot break automatically).',
]
const proposals = await parallel(APPROACHES.map((a, i) => () =>
  agent(CTX + '\n\nSURVEY RESULTS:\n' + surveyText + '\n\nDESIGN PROPOSAL #' + (i + 1) + ' — propose THIS approach in depth and adversarially stress it against all 6 hard constraints:\n' + a,
    { label: 'panel:approach-' + (i + 1), phase: 'Panel', effort: 'high', schema: PROPOSAL_SCHEMA })))
const proposalText = proposals.filter(Boolean).map((p, i) => '### Approach ' + (i + 1) + ': ' + p.approach + '\nAlgorithm: ' + p.algorithm + '\nIntegration: ' + p.integration + '\nBlast: ' + p.blast_radius + '\nSoundness: ' + p.soundness + '\nWeaknesses: ' + p.weaknesses).join('\n\n')

phase('Synthesize')
const SYNTH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['recommendation', 'rationale', 'integration_plan', 'blast_radius', 'open_questions', 'doc_written'],
  properties: {
    recommendation: { type: 'string', description: 'the single recommended GC approach' },
    rationale: { type: 'string', description: 'why it wins for Shape vs the alternatives, scored on the 6 constraints' },
    integration_plan: { type: 'string', description: 'phased implementation plan (what lands first, safepoints, where the cycle collector runs)' },
    blast_radius: { type: 'string', description: 'subsystems + estimated scale + whether ADR-006 amendment needed' },
    open_questions: { type: 'string', description: 'decisions requiring USER ratification (determinism, collection trigger policy, snapshot-GC interaction)' },
    doc_written: { type: 'boolean', description: 'true iff you wrote the full design doc to ' + DOC },
  },
}
const synth = await agent(CTX + '\n\nSURVEY:\n' + surveyText + '\n\nPROPOSALS:\n' + proposalText + '\n\nSYNTHESIZE. Judge the approaches against the 6 hard constraints; pick the single best for Shape (the user wants a REAL cycle collector, so the weak-ref baseline cannot win — but cite precisely why). Write a COMPLETE ratifiable design doc to ' + DOC + ' covering: problem statement + the two leak findings; the recommended algorithm; how it finds roots/traces via the typed parallel-kind track with NO runtime tags; coexistence with Arc RC + escape/Drop + the §2.7.30 reference model; snapshot/resume + JIT-carrier handling (moving vs non-moving); HeapHeader-bit / side-table GC metadata (no new discriminator); a phased implementation plan; blast radius; whether an ADR-006 amendment is required; and an explicit OPEN QUESTIONS FOR USER RATIFICATION section. Then return the summary fields.',
  { label: 'synthesize', phase: 'Synthesize', effort: 'high', schema: SYNTH_SCHEMA })

return { survey, proposals, synth }
