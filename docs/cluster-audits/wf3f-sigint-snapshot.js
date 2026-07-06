export const meta = {
  name: 'wf3f-sigint-snapshot',
  description: 'RELEASE-BLOCKING (user ruling 2026-07-06): SIGINT during an in-flight stdlib builtin saves a SILENTLY-CORRUPT snapshot (save reports OK, resume dies "callee must be ... got Bool"). Root: the call-frame/closure NativeKind track is not persisted through snapshot -> resume Bool-defaults the callee (ADR-006 §2.7.11 closure_heap_kind; the W17 Bool-default gap; same Bool-sentinel family as the D1 distributed bug). Fix: persist the frame/stack kind track through snapshot with NO Bool-default; if a mid-native-frame point is genuinely unrepresentable, refuse cleanly (barrier Err) — NEVER a silently-corrupt snapshot. Fable independently re-proves.',
  phases: [
    { title: 'Diagnose', detail: 'confirm the exact kind-loss site in snapshot serialize/restore of the call frame + stack' },
    { title: 'Fix', detail: 'persist frame/stack NativeKind track; no Bool-default; clean refuse if truly unrepresentable' },
    { title: 'Fable-verify', detail: 'independent: SIGINT-mid-builtin resumes correctly OR refuses cleanly, never silent corruption' },
    { title: 'Finish', detail: 'gates + deterministic regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3f-sigint'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/snapshot-sigint-corruption, off main). Build/test via: ${DX} <cmd>.

THE DEFECT (release-blocking): a running program interrupted by SIGINT (Ctrl+C) while an in-flight stdlib builtin is executing saves a snapshot that reports OK ("Snapshot saved: <hash>") but is SILENTLY CORRUPT — 'shape --resume <hash>' dies with "call_value_immediate_*: callee must be ... got Bool" (a callee-kind error). Silent corruption is the bug; the design (W17 / snapshot-resume §4.5 barrier rule) forbids it — a non-resumable point must REFUSE cleanly, and a resumable point must resume correctly.

FLOW (already traced): ctrlc handler bin/shape-cli/src/commands/script_cmd.rs:300 sets an interrupt AtomicU8; the VM checks it at a safepoint (crates/shape-vm/src/executor/mod.rs, interrupt flag :433 region) and raises ShapeError::Interrupted{snapshot_hash} after saving a snapshot at that point; script_cmd.rs:462-483 prints the hash and exits 130.

LIKELY ROOT (confirm, do not assume): the snapshot serialize/restore of the call-frame + VM stack does NOT persist the parallel NativeKind track (ADR-006 §2.7.7 stack kinds / §2.7.11 CallFrame.closure_heap_kind / §2.7.8 cell kinds). On resume the callee slot's kind Bool-defaults, so a subsequent CallValue sees a Bool callee. This is a live ADR-006 Bool-default Forbidden-Pattern in the snapshot path (see project_w17_snapshot_completion: "a Bool-default violation"). It is the SAME Bool-sentinel family as the distributed D1 bug.

ACCEPTANCE BAR (either is acceptable; silent corruption is NOT):
  (A) SIGINT-mid-builtin resumes CORRECTLY — the persisted frame/stack kind track round-trips, resume continues and yields the right value; OR
  (B) if a mid-native-frame interrupt point is genuinely unrepresentable, the save REFUSES CLEANLY with a surfaced barrier Err ("cannot snapshot inside a native frame" or similar) and exit 130 prints "Interrupted - no snapshot saved" (script_cmd.rs:481) — never a hash for a corrupt snapshot.
Prefer (A) if the gap is purely serialization (persist the kind track). Fall back to (B) only where the frame is genuinely native-in-flight.

HARD CONSTRAINTS (CLAUDE.md §Forbidden Patterns, ADR-006): NO Bool-default at any kind-source gap. NO ValueWord/tag-decode/raw-u64/synthesize-from-bits. Snapshot uses parallel Vec<u64> data + Vec<NativeKind> kinds (§2.7.7/Q9). If a slot's kind is genuinely unknown at restore, surface-and-stop (NotImplemented(SURFACE)) — never fabricate Bool/Null. ${DX} just check-no-dynamic must stay EXIT 0.

BLACK-BOX REPRO (genuine): a shape program that calls a slow stdlib builtin (e.g. time::sleep(3000) or a long-running loop that stays inside one builtin), run it, send SIGINT via 'kill -INT <pid>' (or Ctrl+C) DURING the builtin, capture the printed hash, then '${WT}/target/release/shape --resume <hash>' and observe corruption vs correct-resume/clean-refuse.
`

phase('Diagnose')

const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['root_cause', 'kind_loss_site', 'is_bool_default', 'chosen_fix', 'repro'],
  properties: {
    root_cause: { type: 'string', description: 'the exact reason resume sees a Bool callee' },
    kind_loss_site: { type: 'string', description: 'file:line where the frame/stack/cell NativeKind fails to persist or restore' },
    is_bool_default: { type: 'boolean', description: 'true if a Bool-default fabrication is the mechanism' },
    chosen_fix: { type: 'string', enum: ['A-persist-kind-track', 'B-clean-refuse', 'A-then-B-hybrid'], description: 'which acceptance path this defect needs' },
    repro: { type: 'string', description: 'the exact black-box repro commands that reproduce the corruption on current HEAD' },
  },
}

const diag = await agent(`${CTX}\n\nDIAGNOSE ONLY (no fix yet). Reproduce the corruption from scratch (black-box). Pinpoint the exact serialize/restore site where the call-frame/stack/cell NativeKind is lost or Bool-defaulted. Decide whether the correct fix is (A) persist the kind track, (B) clean-refuse, or a hybrid. Do NOT commit.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')

const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'approach', 'no_bool_default', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string' },
    approach: { type: 'string', description: 'A/B/hybrid actually implemented + how the kind track round-trips' },
    no_bool_default: { type: 'boolean', description: 'true iff no Bool/Null fabrication was introduced and check-no-dynamic is EXIT 0' },
    evidence: { type: 'string', description: 'black-box: SIGINT-mid-builtin -> resume correct value OR clean refuse (no hash for corrupt snapshot); the exact captured output' },
  },
}

const fix = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nIMPLEMENT the fix per chosen_fix. Persist the frame/stack/cell NativeKind track through snapshot serialize+restore (parallel Vec<NativeKind> per §2.7.7/§2.7.8/§2.7.11) with NO Bool-default; clean-refuse only where a frame is genuinely native-in-flight. Build release + prove the black-box repro now resumes correctly OR refuses cleanly (never a corrupt-snapshot hash). ${DX} just check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-3F snapshot frame-kind persistence wip').`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Fable-verify')

const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'behavior', 'silent_corruption_gone', 'forbidden_patterns', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    behavior: { type: 'string', enum: ['resumes-correctly', 'refuses-cleanly', 'still-corrupt', 'mixed'] },
    silent_corruption_gone: { type: 'boolean', description: 'true iff no interrupt point produces a save-OK-but-resume-broken snapshot' },
    forbidden_patterns: { type: 'string', description: 'result of grepping the diff for Bool-default/ValueWord/tag-decode/raw-u64 in the snapshot path — "none" or the hunk' },
    evidence: { type: 'string', description: 'your own from-scratch repro across several interrupt timings (mid-sleep, mid-loop-builtin, between-opcodes) + captured resume outcomes' },
  },
}

const verify = await agent(`${CTX}\n\nFIX CLAIM: ${JSON.stringify(fix)}\n\nYou are Fable, an INDEPENDENT adversarial verifier. Assume the fix is INSUFFICIENT until your own hands-on runs prove otherwise. Construct your OWN repro from scratch. Interrupt at SEVERAL timings (mid-time::sleep, mid-loop-heavy-builtin, and a between-opcodes point) and for EACH: does resume yield the correct value, or a clean refuse, or STILL a silently-corrupt snapshot (save-OK/resume-broken)? Any single silent-corruption case = REFUTED. Grep the diff for reintroduced Bool-default/ValueWord/tag-decode/raw-u64 in the snapshot serialize/restore. Report exactly what you saw.`,
  { label: 'fable-verify', phase: 'Fable-verify', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')

const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['check_clean', 'check_no_dynamic', 'test_added', 'tests', 'merge_ready'],
  properties: {
    check_clean: { type: 'string' },
    check_no_dynamic: { type: 'string' },
    test_added: { type: 'string', description: 'the deterministic regression test added (sets interrupt flag mid-builtin, snapshots, resumes, asserts correct/clean-refuse) — its name + location' },
    tests: { type: 'string', description: 'just test result: new failures beyond pinned pre-existing, or "only pinned"' },
    merge_ready: { type: 'boolean' },
  },
}

const finish = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\nFABLE VERDICT: ${JSON.stringify(verify)}\n\nFINISH (only if Fable CONFIRMED; if REFUTED, report merge_ready:false and what remains). Add a DETERMINISTIC regression test (unit or integration): programmatically set the interrupt flag so it fires mid-builtin, take the interrupt snapshot, resume, assert the correct value (or a clean barrier Err) — NOT timing-dependent. Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test. Commit (git commit --no-verify -m 'WF-3F finalize: snapshot frame-kind persistence + regression test'). Report merge readiness.`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
