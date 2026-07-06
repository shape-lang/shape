export const meta = {
  name: 'wf3a-schema-identity-m1',
  description: 'M1 (user-ratified 2026-07-06): replace counter-allocated schema identity with content-derived structural identity, in-process. SchemaContentId = hash over {name-for-NAMED-types, ordered (field_name,field_type), enum variants}; SchemaId:u32 becomes a per-Runtime intern(content_id) handle (the blessed StringId relationship, NOT a parallel discriminator). Delete BOTH counters + all 4 point-patches; un-ignore object-spread. Identity model: NAMED/branded types are NOMINAL (name in hash -> type A{x,y} != type B{x,y}); ANONYMOUS types are STRUCTURAL (no name -> {x,y} dedups by structure); fields are declaration-ordered (fixed-offset layout). Draft the ADR amendment. Fable independently re-proves. M2 (cross-node wire/snapshot determinism) is a separate follow-up.',
  phases: [
    { title: 'Implement', detail: 'content-id + intern handle at the ~50 mint sites; delete the 4 patches + 2 dedup caches; ADR draft' },
    { title: 'Fable-verify', detail: 'independent: object-spread + json/xml id-41 + cross-registry equality fixed; NO collision-suppressor reborn' },
    { title: 'Repair', detail: 'if refuted, repair and re-run' },
    { title: 'Fable-verify-2', detail: 'independent re-proof after repair' },
    { title: 'Finish', detail: 'gates + un-ignore object-spread tests + regression tests' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3a-schema'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/schema-identity-design, which has the ratified design doc at docs/design/schema-identity-structural.md — READ it first). Build/test via: ${DX} <cmd>.

RATIFIED DESIGN (M1, in-process only):
- Introduce SchemaContentId([u8;32]) = a stable SHA-256 over the schema's structure. Identity model (user-ratified 2026-07-06):
  * NAMED / "branded" types (declared 'type Foo {...}', enums): the NAME is part of the hash -> NOMINAL identity. type A {x,y} and type B {x,y} are DISTINCT.
  * ANONYMOUS types (object-literal / inline / merged with no user name): NO name in the hash -> STRUCTURAL identity. Two anonymous {x:int, y:int} share one id anywhere.
  * Fields are DECLARATION-ORDERED in the hash (Shape has C-compatible fixed-offset TypedStruct; order = layout). Include (field_name, field_type/layout) in order + enum variants.
- Keep SchemaId: u32 but mint it ONLY via a per-Runtime intern(content_id) -> u32 table (the blessed StringId interning relationship — a derived index of the ONE canonical id, NOT a second source of truth / parallel discriminator). Identical content_id -> same handle; distinct -> distinct handle; deterministic within a Runtime, independent of registration order.

DELETE (root cutover — these are the collision + its 4 symptom-suppressors; keeping ANY beside the new path is the forbidden 'ensure_next_id_above reborn' pattern):
- Counter A: the compile-time TypeTracker registry next_id path (register_type_scoped) — route through intern.
- Counter B: NEXT_SCHEMA_ID static (crates/shape-runtime/src/type_schema/schema.rs:20 allocate_current_id, used by TypeSchema::new schema.rs:86) — route through intern.
- ensure_next_id_above (registry.rs:121) + ensure_next_schema_id_above (mod.rs:111) + all callers (load_program program.rs:79, extension load statements.rs:2211, seed_persistent_schemas compiler_impl_initialization.rs:914, merge registry.rs:521/567).
- merge() id-collision reallocation loop + id_remap table (registry.rs:536-545).
- reserved flag (schema.rs:61) + reserved-skip in field-order inference (mod.rs:146) — the WF-1B guard.
- WF-2E resolve_typed_object_schema arity heuristic (crates/shape-runtime/src/.../json_value.rs:435) — the runtime arity disambiguation across 3 registries.
- Name-based structural-dedup caches now subsumed by content-id: __inline_obj_N by (name,type) (type_tracking.rs:1135) and __merged_L_R by name (collections.rs:1101).

SCOPE NOTE: only the ~50 MINT sites (group a) change; the ~117 by_id lookups + snapshot + content-hash + type-equality + ~83 JIT sites (groups b-g) CONSUME the u32 handle unchanged — do NOT churn them. Bytecode/JIT operand format is unchanged (still u32). Blob content hashes will change (they embed schema_id operands) — that is EXPECTED and BETTER (deterministic across registration order); bump any on-disk cache/version tag as needed. M2 (wire content-id carrier + snapshot content-id + cross-node blob determinism) is OUT OF SCOPE — leave the wire Some(1) placeholder + raw-u32 snapshot as-is for M1, but do NOT add a shim that would block M2.

ADR: draft an ADR amendment (append to docs/adr/006-value-and-memory-model.md or a new ADR) recording: content-derived schema identity, the intern-handle relationship (StringId-family, single-discriminator-preserving), the named-nominal / anonymous-structural rule, and the deletion of the counter path + 4 patches. This is REQUIRED (user-ratified adr_needed).

ACCEPTANCE: (1) the object-spread repros (crates/shape-jit/src/mir_compiler/integration_tests.rs ~2096 + tools/shape-test/tests/jit/tiering.rs ~276) UN-IGNORED and PASSING (extended.z resolves the 3-field layout). (2) The json/xml id-41 case resolves the correct schema WITHOUT the arity heuristic (delete it, prove typed_object_to_json_value still renders XmlNode vs Json correctly). (3) A named vs anonymous identity test: type A {x,y} != type B {x,y} (distinct handles); two anonymous {x:int,y:int} == (same handle). (4) NO collision anywhere; NO surviving counter/ensure_next_id_above/arity-heuristic fallback (grep the diff). (5) ${DX} just check-clean + check-no-dynamic EXIT 0; just test only pinned pre-existing failures.

CONSTRAINTS (CLAUDE.md §Forbidden): the u32 intern handle must be a DERIVED index of the canonical content id, not a parallel discriminator (ADR-005 §1). NO ValueWord/tag-decode/Bool-default. NO keeping an old-id resolution or arity-heuristic fallback beside the new path (surface-and-stop if a genuine hash collision occurs — but a 256-bit hash collision is astronomically unlikely; do NOT add a counter fallback). check-no-dynamic stays EXIT 0.

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.
`

phase('Implement')
const IMPL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'patches_deleted', 'object_spread', 'adr_drafted'],
  properties: {
    status: { type: 'string', enum: ['implemented', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'brief: the mint-point change + which files' },
    patches_deleted: { type: 'string', description: 'which of the 4 point-patches + 2 dedup caches + 2 counters were deleted' },
    object_spread: { type: 'string', description: 'object-spread repros un-ignored + passing? evidence' },
    adr_drafted: { type: 'boolean', description: 'ADR amendment written' },
  },
}
const impl = await agent(`${CTX}\n\nIMPLEMENT M1. Read the design doc first. Add SchemaContentId + per-Runtime intern; route both mint paths through it; delete the 2 counters + 4 point-patches + 2 dedup caches; un-ignore the object-spread repros; draft the ADR. Build release. Prove object-spread + json/xml id-41 + named-vs-anonymous identity. ${DX} just check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-3A M1 content-derived schema identity wip').`,
  { label: 'implement', phase: 'Implement', effort: 'high', schema: IMPL_SCHEMA })

phase('Fable-verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'object_spread_fixed', 'projection_fixed', 'no_fallback_reborn', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    object_spread_fixed: { type: 'boolean' },
    projection_fixed: { type: 'boolean', description: 'json/xml id-41 renders correctly WITHOUT the deleted arity heuristic' },
    no_fallback_reborn: { type: 'boolean', description: 'true iff NO counter/ensure_next_id_above/arity-heuristic fallback survives in the diff' },
    evidence: { type: 'string', description: 'your own from-scratch tests: object-spread, named-vs-anonymous identity, json/xml render, a registration-order-shuffle collision probe; concise' },
  },
}
const verify = await agent(`${CTX}\n\nIMPL: ${JSON.stringify(impl)}\n\nYou are Fable, INDEPENDENT adversarial verifier. Assume INSUFFICIENT until your own runs prove otherwise. Build your OWN tests: (1) object-spread extended.z; (2) named-vs-anonymous identity (type A{x,y} != type B{x,y}; two anon {x:int,y:int} equal); (3) json/xml id-41 render correct WITHOUT the arity heuristic; (4) a REGISTRATION-ORDER-SHUFFLE probe — register schemas in different orders and confirm NO wrong-arity lookup (the old counter would collide). Grep the diff: any surviving counter / ensure_next_id_above / resolve_typed_object_schema arity fallback = REFUTED. check-no-dynamic EXIT 0.`,
  { label: 'fable-verify', phase: 'Fable-verify', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })

phase('Repair')
let repair = null, verify2 = null
if (verify && verify.verdict === 'REFUTED') {
  repair = await agent(`${CTX}\n\nIMPL: ${JSON.stringify(impl)}\nFABLE REFUTED: ${JSON.stringify(verify)}\n\nREPAIR every surviving issue Fable found. Keep the clean cutover (no fallback reborn). Re-run the affected probe yourself. Commit WIP (git commit --no-verify -m 'WF-3A M1 repair wip').`,
    { label: 'repair', phase: 'Repair', effort: 'high', schema: IMPL_SCHEMA })
  phase('Fable-verify-2')
  verify2 = await agent(`${CTX}\n\nREPAIR: ${JSON.stringify(repair)}\n\nYou are Fable, INDEPENDENT verifier ROUND 2. Re-prove object-spread + named/anonymous identity + json/xml render + order-shuffle from scratch. Any collision or surviving fallback = REFUTED.`,
    { label: 'fable-verify-2', phase: 'Fable-verify-2', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })
}

phase('Finish')
const finalVerify = verify2 || verify
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests', 'adr_committed', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + verify-merge + just test (new failures beyond pinned, or "only pinned"), brief' },
    tests: { type: 'string', description: 'tests un-ignored/added: object-spread, named-vs-anonymous, order-shuffle' },
    adr_committed: { type: 'boolean' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFINAL FABLE: ${JSON.stringify(finalVerify)}\n\nFINISH (only if latest Fable CONFIRMED; else merge_ready:false + what remains). Ensure the object-spread repros are un-ignored + passing; add a named-vs-anonymous identity test + a registration-order-shuffle regression test. Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} bash scripts/verify-merge.sh, ${DX} just test --no-fail-fast. Ensure the ADR amendment is committed. Commit (git commit --no-verify -m 'WF-3A M1 finalize: content-derived schema identity + ADR + tests').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { impl, verify, repair, verify2, finish }
