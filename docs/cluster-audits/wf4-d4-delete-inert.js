export const meta = {
  name: 'wf4-d4-delete-inert-tiered',
  description: 'D4 (user-ruled DELETE now): remove the inert tiered-compilation machinery — enable_tiered_compilation / set_backend / register_jit_function + the NotImplemented promote-dispatch stub at control_flow/mod.rs:285-300 — plus the dead code the deletion exposes. Diagnose-first (PROVE each symbol is inert / zero real callers before deleting; if any is actually load-bearing, surface + stop), delete at the root, independent Opus adversarial re-proof that nothing real regressed, gates.',
  phases: [
    { title: 'Diagnose', detail: 'prove each target symbol is inert (zero real callers) + pin exact sites + cascade' },
    { title: 'Delete', detail: 'remove the inert machinery + the dead code it exposes' },
    { title: 'Verify', detail: 'independent Opus: nothing real used them; build+tests green; no new dead code' },
    { title: 'Finish', detail: 'gates + confirm the promote path is gone (or a marker test)' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w4-d4'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave4/d4-delete-inert, off main). Build/test via: ' + DX + ' <cmd>.',
  '',
  'USER RULING (2026-07-06): D4 tiered-compilation = DELETE the inert machinery NOW (not wire, not defer). The audit found this machinery present but INERT: no real code path drives it. Your job is to delete it cleanly and cascade-delete the dead code it exposes.',
  '',
  'DELETION TARGETS (confirm each is inert BEFORE deleting):',
  '1. enable_tiered_compilation — audit says zero callers. Grep the whole workspace for real (non-test, non-self) call sites.',
  '2. set_backend — tier/backend selection setter, audit says inert.',
  '3. register_jit_function — audit says inert JIT-registration entry.',
  '4. The NotImplemented promote-dispatch stub at crates/shape-vm/src/executor/control_flow/mod.rs (~lines 285-300) — a promoted-dispatch path that returns NotImplemented (never reached).',
  'Plus: any struct fields, enum variants, config flags, or helper fns that become dead ONLY because of these deletions (the compiler dead_code warnings + the exhaustive-match errors will guide you).',
  '',
  'DISCIPLINE: this is a DELETION, not a rewrite. For EACH target, first PROVE it is inert: grep for call sites across the workspace (crates/**, bin/**, tools/**, extensions/**), and show that the only references are the definition + other soon-deleted machinery + tests that only exist to exercise the dead path. If ANY target turns out to be load-bearing (a real code path, a public API other code depends on, a wired tier-up that actually fires), DO NOT delete it — surface it in the diagnosis with evidence and mark it keep. Delete only what is genuinely inert.',
  '',
  'CONSTRAINTS (CLAUDE.md): after deletion the workspace must still compile (' + DX + ' just check-clean EXIT 0) and ' + DX + ' just check-no-dynamic EXIT 0. Do NOT introduce any forbidden pattern (this is deletion, so unlikely). Do NOT delete the REAL JIT tiering thresholds/execution (tier.rs T1@100/T2@10k + the actual JIT compile path are LIVE and must stay) — only the INERT promote/register/backend-select machinery the audit flagged. If unsure whether a symbol is the live path or the inert stub, err toward keep + surface.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields. Put per-symbol call-site detail in commit messages, not schema fields.',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['each_target_inert', 'load_bearing_found', 'cascade', 'sites'],
  properties: {
    each_target_inert: { type: 'string', description: 'per target (enable_tiered_compilation/set_backend/register_jit_function/promote-stub): inert(zero real callers) | load-bearing(keep). brief' },
    load_bearing_found: { type: 'boolean', description: 'true iff ANY target is actually load-bearing and must NOT be deleted' },
    cascade: { type: 'string', description: 'dead code exposed by the deletions (fields/variants/helpers), brief' },
    sites: { type: 'string', description: 'file:line anchors for each deletion, brief' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no deletion). For each target prove inert-vs-load-bearing with grep evidence; identify cascade dead code. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Delete')
const DELETE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'deleted', 'kept', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['deleted', 'partial', 'blocked'] },
    deleted: { type: 'string', description: 'symbols/lines removed, brief' },
    kept: { type: 'string', description: 'any target kept because load-bearing (with reason) or "none"' },
    evidence: { type: 'string', description: 'check-clean EXIT + check-no-dynamic EXIT after deletion; captured' },
  },
}
const del = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — DELETE the genuinely-inert targets + cascade dead code (KEEP anything the diagnosis flagged load-bearing). After deleting, ' + DX + ' just check-clean must be EXIT 0 and ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-4 D4: delete inert tiered-compilation machinery").',
  { label: 'delete', phase: 'Delete', effort: 'high', schema: DELETE_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'nothing_real_regressed', 'build_green', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    nothing_real_regressed: { type: 'boolean', description: 'true iff the deleted symbols had no real callers (re-checked from scratch) and no live path lost' },
    build_green: { type: 'boolean', description: 'check-clean + check-no-dynamic both EXIT 0 (re-run yourself)' },
    evidence: { type: 'string', description: 'your own from-scratch grep for lingering references + build result; concise' },
  },
}
const verify = await agent(CTX + '\n\nDELETE: ' + JSON.stringify(del) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT do the deletion). Assume INSUFFICIENT until proven. From scratch: (1) grep the workspace for any lingering reference to the deleted symbols (would be a broken build — confirm none). (2) Confirm the deletion did NOT remove a LIVE path: the real JIT tiering (tier.rs thresholds + actual compile+execute) must still work — run a small hot-loop program and confirm it still JIT-compiles + runs. (3) ' + DX + ' just check-clean and ' + DX + ' just check-no-dynamic both EXIT 0. Any lingering ref, lost live path, or non-zero gate = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_note', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    test_note: { type: 'string', description: 'a marker test or note that the promote/register path is gone (or why none needed)' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nDELETE: ' + JSON.stringify(del) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast (report new failures beyond the known-pinned set). Commit (git commit --no-verify -m "WF-4 D4 finalize: inert tiered-compilation machinery deleted + gates").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, del, verify, finish }
