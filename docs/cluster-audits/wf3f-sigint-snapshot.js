export const meta = {
  name: 'wf3f-sigint-snapshot-v2',
  description: 'RELEASE-BLOCKING (user 2026-07-06): SIGINT mid-builtin -> silently-corrupt snapshot. Layer 1 (resume-marker off-by-one) FIXED (commit 1f78e9c8, kept). Fable REFUTED completeness: interrupt-resume now prints acc-as-of-interrupt and SKIPS the loop tail (still silent corruption, exit 0), and a user int module binding reads as `false` after resume (the module_binding_kinds Bool-default is ACTIVELY corrupting user bindings, not unused pads). This is W17 whole-VM interrupt-resume completion. Fix the remaining layer(s) so an interrupt snapshot resumes to the CORRECT full-program result (or refuses cleanly) — NEVER save-OK-but-resume-wrong. Fable re-proves.',
  phases: [
    { title: 'Diagnose', detail: 'why interrupt-resume skips the loop tail + where module bindings get Bool-corrupted; one root or two' },
    { title: 'Fix', detail: 'complete interrupt-resume: real binding kinds + correct continuation from the interrupt ip' },
    { title: 'Fable-verify', detail: 'independent: interrupt at many landings -> correct full result, never silent corruption' },
    { title: 'Repair', detail: 'if refuted, repair the surviving corruption and re-run' },
    { title: 'Fable-verify-2', detail: 'independent re-proof after repair' },
    { title: 'Finish', detail: 'gates + deterministic regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3f-sigint'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/snapshot-sigint-corruption @1f78e9c8, which already has the LAYER-1 marker fix — KEEP it). Build/test via: ${DX} <cmd>.

THE RELEASE-BLOCKING DEFECT: SIGINT (Ctrl+C) during an in-flight builtin saves a snapshot that reports OK but 'shape --resume <hash>' produces a WRONG result with exit 0 (silent corruption). The design (W17 / snapshot-resume §4.5) forbids silent corruption: an interrupt snapshot must resume to the CORRECT full-program result, or refuse cleanly.

WHAT IS ALREADY FIXED (layer 1, commit 1f78e9c8, do NOT regress): resume_from_snapshot_impl (execution.rs:341-362) now pushes the Ok(Snapshot::Resumed) operand-stack marker ONLY for a snapshot()-call origin (VmSnapshot.interrupt_saved==false), not for an interrupt snapshot. This removed the wrong-kind-callee CRASH.

WHAT FABLE PROVED STILL BROKEN (from-scratch repro): a hot loop 'acc = add(acc, i)' over 0..60000000, kill -INT mid-loop, then --resume prints acc-as-of-the-interrupt (e.g. 303288894981672 vs the correct 3599999940000000) and exits 0 — THE REST OF THE LOOP IS SILENTLY SKIPPED. A pure between-opcodes loop and a mid-time::sleep interrupt corrupt identically (Fable saw 1485 vs 499999500000 = sum(0..1000000)). Also: a diagnostic run showed an int module binding printing as FALSE after resume — so the module_binding_kinds Bool-default is ACTIVELY corrupting user bindings.

CANDIDATE ROOTS (confirm; they may be ONE root):
- module_binding_kinds Bool-corruption. module_binding_pad_to_kinded (executor/mod.rs:837) pads unwritten slots with a Bool SENTINEL; module_binding_write_kinded (mod.rs:861) stamps the real kind on first write. Bool-write/clear sites: mod.rs:718, 796, 840, 1040. module_binding_kinds IS persisted+restored (executor/snapshot.rs:224 save / :317 restore). HYPOTHESIS: a written user int binding (the loop's acc / counter) ends up Bool at snapshot time, OR the restore mis-reads it, so on resume the loop var reads as 0/false and the loop condition immediately exits -> "skips the tail". If so, the "skips loop" symptom and the "int reads false" symptom are the SAME root: module-binding kinds not faithfully round-tripping for LIVE (written) bindings. VERIFY whether acc/the loop counter are module bindings and what kind they carry at save vs restore.
- OR a genuinely separate continuation bug: the interrupt ip / frame / loop-control state is restored such that vm.execute() (execution.rs:375) resumes past the loop. Determine if the saved resume_ip + call-frame stack place execution back INSIDE the loop body.

ACCEPTANCE BAR: interrupt at ANY landing (mid-sleep, mid-loop-builtin, between-opcodes) -> --resume yields the CORRECT full-program result (the complete loop sum), exit 0; OR, only where genuinely unrepresentable, a clean surfaced barrier Err (no hash for a corrupt snapshot). NO save-OK-but-resume-wrong. NO save-OK-but-resume-partial.

HARD CONSTRAINTS (CLAUDE.md §Forbidden Patterns / ADR-006): NO Bool-default at any real kind-source gap (the pad SENTINEL for never-written slots is allowed; a WRITTEN binding losing its kind to Bool is the bug). NO ValueWord/tag-decode/raw-u64. ${DX} just check-no-dynamic must stay EXIT 0.

BLACK-BOX REPRO: build release; a program with a long loop accumulating into acc (print acc at the end); run it; 'kill -INT <pid>' mid-loop; capture the printed hash; '${WT}/target/release/shape --resume <hash>'; the printed acc MUST equal the full-loop result (as if never interrupted).

STRUCTURED-OUTPUT: emit ONE clean JSON object per schema. Keep each string field to 1-4 plain sentences. NO XML/tool-call tags inside values. NO code blocks in fields. Put long detail in a commit message or a scratch file, not in a schema field.
`

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['root', 'one_or_two', 'evidence', 'fix_plan'],
  properties: {
    root: { type: 'string', description: 'the precise reason interrupt-resume skips the loop tail + why an int binding reads false — file:line' },
    one_or_two: { type: 'string', enum: ['one-root-binding-kinds', 'one-root-continuation', 'two-roots'], description: 'whether the skip + false-binding are the same root' },
    evidence: { type: 'string', description: 'save-vs-restore kind of the loop var/acc; whether acc is a module binding; concise' },
    fix_plan: { type: 'string', description: 'concrete change(s), file:line, brief' },
  },
}
const diag = await agent(`${CTX}\n\nDIAGNOSE ONLY (no fix). Reproduce from scratch. Determine whether "skips loop tail" and "int binding reads false" are ONE root (module-binding kinds not round-tripping for live bindings) or TWO. Pin the exact save-side and/or restore-side site. Do NOT commit.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'brief' },
    evidence: { type: 'string', description: 'black-box: kill -INT mid-loop then resume prints the CORRECT full sum; captured values at 3+ timings; check-no-dynamic EXIT' },
  },
}
const fix = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nIMPLEMENT the fix (keep the layer-1 marker fix). Make interrupt-resume yield the CORRECT full-program result. Build release. Prove the black-box repro at 3+ interrupt timings prints the full loop sum. ${DX} just check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-3F interrupt-resume completion wip').`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Fable-verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'silent_corruption_gone', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    silent_corruption_gone: { type: 'boolean', description: 'true iff NO interrupt landing yields save-OK-but-resume-wrong/partial' },
    evidence: { type: 'string', description: 'your own from-scratch repros at several timings; captured resume values vs expected full result; concise' },
  },
}
const verify = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\n\nYou are Fable, INDEPENDENT adversarial verifier. Assume INSUFFICIENT until your own runs prove otherwise. Build your OWN repro. Interrupt at SEVERAL timings (mid-sleep, mid-loop-builtin, between-opcodes) and confirm each --resume prints the CORRECT FULL result (not a partial). Any single save-OK-but-resume-wrong/partial = REFUTED. Grep the diff for reintroduced Bool-default on WRITTEN bindings / ValueWord / tag-decode.`,
  { label: 'fable-verify', phase: 'Fable-verify', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })

phase('Repair')
let repair = null, verify2 = null
if (verify && verify.verdict === 'REFUTED') {
  repair = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\nFIRST FIX: ${JSON.stringify(fix)}\nFABLE REFUTED: ${JSON.stringify(verify)}\n\nREPAIR every surviving corruption Fable found. Re-run the affected repro yourself until each interrupt landing resumes to the CORRECT full result. Keep all prior correct fixes. NO forbidden machinery. Commit WIP (git commit --no-verify -m 'WF-3F interrupt-resume repair wip').`,
    { label: 'repair', phase: 'Repair', effort: 'high', schema: FIX_SCHEMA })

  phase('Fable-verify-2')
  verify2 = await agent(`${CTX}\n\nREPAIR: ${JSON.stringify(repair)}\n\nYou are Fable, INDEPENDENT verifier, ROUND 2. From scratch again, interrupt at several timings; each --resume MUST print the correct full result. Any save-OK-but-resume-wrong/partial = REFUTED.`,
    { label: 'fable-verify-2', phase: 'Fable-verify-2', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })
}

phase('Finish')
const finalVerify = verify2 || verify
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    test_added: { type: 'string', description: 'deterministic regression test name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFINAL FABLE VERDICT: ${JSON.stringify(finalVerify)}\n\nFINISH (only if the latest Fable verdict is CONFIRMED; else merge_ready:false + what remains). Add a DETERMINISTIC regression test: set the interrupt flag so it fires mid-loop, snapshot, resume, assert the CORRECT FULL result (not timing-dependent). Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test --no-fail-fast. Commit (git commit --no-verify -m 'WF-3F finalize: interrupt-resume completion + regression test').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, repair, verify2, finish }
