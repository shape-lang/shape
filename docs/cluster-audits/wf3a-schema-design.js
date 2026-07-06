export const meta = {
  name: 'wf3a-schema-identity-design',
  description: 'DESIGN-FIRST (user 2026-07-06: surface before committing). The recurring schema-id collision root: TypeSchema identity is counter-allocated (process-global NEXT_SCHEMA_ID / per-registry next_id.fetch_add), so any new registration shifts ids and collides (4+ point-patches so far; latest = object-spread "field_idx out of bounds"; also drives the remote::execute JsonValue projection residual). Produce a rigorous diagnosis + STRUCTURAL-identity design + full blast-radius (every SchemaId consumer: compile assign / runtime lookup / snapshot / wire / content-addressed hash / type-equality / JIT) + ADR-needed assessment + migration plan. NO code changes beyond writing the design doc — this surfaces for user ratification.',
  phases: [
    { title: 'Diagnose', detail: 'exact collision mechanism + the prior point-patches + reproduce object-spread + remote::execute-projection link' },
    { title: 'Blast-radius', detail: 'enumerate every SchemaId consumer by category; which are registration-order-sensitive' },
    { title: 'Design', detail: 'structural (content-based) identity design + migration + ADR assessment; write the design doc' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3a-schema'
const DX = 'direnv exec /home/dev/dev/shape-lang'
const DOC = WT + '/docs/design/schema-identity-structural.md'

const CTX = `
Work IN ${WT} (branch wave3/schema-identity-design, off main). Read/build via: ${DX} <cmd>. This is a DESIGN pass — do NOT change compiler/runtime code; the only file you WRITE is the design doc at ${DOC}.

THE RECURRING ROOT: schema identity is COUNTER-allocated. TypeSchema::new (crates/shape-runtime/src/type_schema/schema.rs:86) uses allocate_current_id() = the process-global NEXT_SCHEMA_ID static (schema.rs:20); TypeSchemaRegistry.allocate_id (crates/shape-runtime/src/type_schema/registry.rs:111-113) uses a per-registry next_id.fetch_add(1). Because the id is a monotonic COUNTER, any change in WHICH schemas get registered or in WHAT ORDER shifts every subsequent id. Two structurally-different schemas can then land on the same SchemaId across a registry boundary (compile vs runtime vs a freshly-registered inline schema), and consumers keyed on SchemaId (e.g. the by_id map registry.rs:148, field-offset tables) resolve the WRONG schema -> the object-spread "field_idx N out of bounds (slot count 1)" class, and the remote::execute JsonValue polymorphic projection residual (WireValue rendered via the wrong schema). ensure_next_id_above (registry.rs:121) is a PATCH that bumps the counter past loaded ids — a symptom-suppressor, not a fix.

REPRO (object-spread, currently #[ignore]'d): crates/shape-jit/src/mir_compiler/integration_tests.rs (~line 2096) + tools/shape-test/tests/jit/tiering.rs (~line 276) — a 'type Base { x, y }' spread into an extended type; then 'extended.z' errors "field_idx 2 out of bounds (slot count 1)" because a colliding SchemaId resolves Base's 1-field layout for the extended object. Reproduce it and trace the exact collision.

WHAT TO PRODUCE (write to ${DOC}, return a SLIM summary):
1. DIAGNOSIS: the exact collision mechanism (counter allocation across registry boundaries), a reproduced object-spread trace, the remote::execute-projection connection, and an inventory of the PRIOR point-patches (grep ensure_next_id_above + any id-collision guards + the 4 recurrences the memory references).
2. BLAST-RADIUS: enumerate EVERY SchemaId consumer, grouped: (a) compile-time id ASSIGNMENT (allocate_current_id / allocate_id / with_id call sites), (b) runtime LOOKUP (by_id / by_name / field-offset), (c) SNAPSHOT serialize/restore (does the snapshot persist SchemaId? does resume re-register?), (d) WIRE serialize (shape-wire — 2 files), (e) CONTENT-ADDRESSED hash (does a blob's content hash include SchemaId? if so, structural identity CHANGES hashes — critical), (f) TYPE-EQUALITY / comparison, (g) JIT (18 shape-jit files). For EACH group say whether it is registration-ORDER-SENSITIVE. Count the sites.
3. DESIGN: propose STRUCTURAL (content-based) identity — derive SchemaId from a stable hash of the schema's structure {name, ordered (field_name, field_type), enum variants, ...} so identical schemas share an id and distinct schemas never collide, INDEPENDENT of registration order. Cover: collision-resistance (hash width / fallback on hash collision), determinism across processes/nodes (needed for wire + content-addressed + snapshot cross-node), interaction with content-addressed blob hashes (do they stay stable? is this actually BETTER for cross-node dedup?), migration path (can with_id callers keep working? is a compat shim needed and is that a forbidden pattern?), and whether an ADR amendment is required (ADR-005/006 single-discriminator + §2.7.29 wire). Flag any genuine open design questions for the user.

CONSTRAINTS: this must NOT become a parallel-discriminator or a ValueWord-style shim (CLAUDE.md §Forbidden). Structural identity should be a CLEANER single source of truth, not an added carrier. If the honest recommendation is that a full structural-identity migration is too large for one lane, say so and propose the minimal correct root-fix + what stays as follow-up.

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields. Put ALL detail in ${DOC}.
`

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['mechanism', 'point_patches', 'repro_confirmed', 'projection_link'],
  properties: {
    mechanism: { type: 'string', description: 'the exact counter-collision mechanism, concise' },
    point_patches: { type: 'string', description: 'the prior patches found (ensure_next_id_above + others), named' },
    repro_confirmed: { type: 'boolean', description: 'true iff you reproduced the object-spread collision and traced it' },
    projection_link: { type: 'string', description: 'how/whether remote::execute JsonValue projection shares this root' },
  },
}
const diag = await agent(`${CTX}\n\nPHASE 1 — DIAGNOSE ONLY. Reproduce the object-spread collision, trace the exact SchemaId clash, inventory the prior point-patches, and establish the remote::execute-projection link. Write findings into ${DOC} (create it; section "Diagnosis"). Return the slim summary.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Blast-radius')
const BLAST_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['groups', 'order_sensitive', 'content_hash_impact', 'total_sites'],
  properties: {
    groups: { type: 'string', description: 'the consumer groups (a-g) with a site count each, concise' },
    order_sensitive: { type: 'string', description: 'which groups are registration-order-sensitive' },
    content_hash_impact: { type: 'string', description: 'does structural identity change content-addressed blob hashes? better or worse for cross-node dedup?' },
    total_sites: { type: 'integer', description: 'approx total SchemaId consumer sites' },
  },
}
const blast = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nPHASE 2 — BLAST-RADIUS. Enumerate EVERY SchemaId consumer grouped (a)-(g), site counts, order-sensitivity, and the content-addressed-hash impact. Append a "Blast radius" section to ${DOC} with the full enumeration. Return the slim summary.`,
  { label: 'blast-radius', phase: 'Blast-radius', effort: 'high', schema: BLAST_SCHEMA })

phase('Design')
const DESIGN_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['recommendation', 'adr_needed', 'migration', 'open_questions', 'doc_written'],
  properties: {
    recommendation: { type: 'string', description: 'the proposed structural-identity design in 2-4 sentences (or "minimal root-fix + follow-up" if full migration too big)' },
    adr_needed: { type: 'boolean', description: 'does this need an ADR amendment?' },
    migration: { type: 'string', description: 'migration path; whether a compat shim is needed and if it risks a forbidden pattern' },
    open_questions: { type: 'string', description: 'genuine design questions for the user, or "none"' },
    doc_written: { type: 'boolean', description: 'true iff the full design is written to the doc' },
  },
}
const design = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\nBLAST-RADIUS: ${JSON.stringify(blast)}\n\nPHASE 3 — DESIGN. Write the full structural-identity design + migration + ADR assessment + open questions into ${DOC} (sections "Design", "Migration", "ADR assessment", "Open questions for ratification"). Be honest if a full migration is too large — propose the minimal correct root-fix + follow-up scope. Return the slim summary. Commit the doc (git add -A && git commit --no-verify -m 'WF-3A schema-identity design (design-first surface)').`,
  { label: 'design', phase: 'Design', effort: 'high', schema: DESIGN_SCHEMA })

return { diag, blast, design }
