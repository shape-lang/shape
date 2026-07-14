export const meta = {
  name: 'wf-resumability-opaque-arms',
  description: 'Tail #1 (user-greenlit 2026-07-07): finish the 6 snapshot opaque-marker arms so snapshot/resume is HONEST for these types (no silent payload-drop, no crash). Per the prior user disposition (2026-05-29): Iterator / Deque / FilterExpr / Channel → CLEAN-REFUSE (a clear, distinguishable error at snapshot or resume — never a silent drop or a corrupt resume); Mutex / Lazy → DEFINED-RESET (resume to a defined empty/uninitialized state). The arms live in crates/shape-runtime/src/snapshot.rs (IteratorOpaque:739, DequeOpaque:768, ChannelOpaque:777, FilterExprOpaque:842, MutexOpaque:855, LazyOpaque:876; decode arms ~1558-1624). Today they round-trip only a structural flag and drop the payload; some already surface, some may silently degrade. VERIFY-CURRENT first (what does each actually do on a real snapshot→resume of a program holding that type?), then implement the disposition per type, then prove the behavior. Independent Opus verify. No forbidden patterns.',
  phases: [
    { title: 'Verify-current', detail: 'snapshot→resume a program holding each of the 6 types; record actual behavior (refuse/reset/silent-drop/crash)' },
    { title: 'Implement', detail: 'clean-refuse Iterator/Deque/FilterExpr/Channel; defined-reset Mutex/Lazy; distinguishable messages' },
    { title: 'Verify+Finish', detail: 'independent Opus: each type behaves per disposition, no silent-drop/crash; gates + regression tests' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-resume'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/resumability-opaque-arms, off main HEAD). Build/run via: ' + DX + ' <cmd>. Context: the CLI --resume path (VirtualMachine::from_snapshot / STAGE-R5 native deep-restore) already carries live state (stack+kinds, frames, heap, SharedCell/Reference identity, closures). The 6 opaque-marker heap types are the remaining gap.',
  '',
  'THE 6 ARMS (crates/shape-runtime/src/snapshot.rs): IteratorOpaque(:739), DequeOpaque(:768 — closed-flag/len round-trips, contents dropped), ChannelOpaque(:777 — closed+len only), FilterExprOpaque(:842), MutexOpaque(:855 — has_value only), LazyOpaque(:876 — has_value only). Encode arms ~1558-1624 (LazyOpaque:1558, MutexOpaque:1572, ChannelOpaque:1579, DequeOpaque:1585, FilterExprOpaque:1622, IteratorOpaque:1624).',
  '',
  'DISPOSITIONS (user-ruled 2026-05-29, binding): Iterator / Deque / FilterExpr / Channel → CLEAN-REFUSE: snapshotting (or resuming) a VM holding one of these must produce a CLEAR, DISTINGUISHABLE error naming the type (e.g. "snapshot cannot capture a live <Iterator>: <reason>") — NEVER a silent payload-drop that resumes to a wrong/empty value, NEVER a crash. Prefer refusing at snapshot() time (the earliest honest point) if the type is live in the captured state; if that is not feasible, refuse cleanly at resume. Mutex / Lazy → DEFINED-RESET: resume to a defined state (Mutex → unlocked/empty; Lazy → unforced/uninitialized so the next force re-computes) — documented, deterministic, no stale payload.',
  '',
  'PHASE 1 VERIFY-CURRENT (no fix): for EACH of the 6 types, write a Shape program that constructs one, snapshot()s, and --resume-s (or the test-level from_snapshot round-trip), and record the ACTUAL behavior: clean-refuse / defined-reset / silent-drop-to-wrong-value / crash. This establishes what is already correct vs what needs work (some may already surface).',
  '',
  'PHASE 2 IMPLEMENT: make each arm honor its disposition. Clean-refuse types: replace the payload-dropping round-trip with a clean surfaced error (NotImplemented/SURFACE with a distinguishable per-type message) at the earliest honest point. Defined-reset types (Mutex/Lazy): resume to the defined reset state. NO forbidden patterns (no Bool-default/ValueWord/tag-decode). NO silent drop.',
  '',
  'CONSTRAINTS: ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Do not weaken the deep-restore path for the already-working types.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a test per type — clean-refuse types produce the distinguishable error (not a silent wrong value); reset types resume to the defined state; the already-working heap/scalar round-trip still green. No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Verify-current')
const VC_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['per_type_current', 'needs_work', 'notes'],
  properties: {
    per_type_current: { type: 'string', description: 'actual snapshot→resume behavior today for each of the 6 (refuse/reset/silent-drop/crash)' },
    needs_work: { type: 'string', description: 'which of the 6 do not yet honor their disposition' },
    notes: { type: 'string', description: 'earliest-honest-refuse-point feasibility per type' },
  },
}
const vc = await agent(CTX + '\n\nPHASE 1 — VERIFY-CURRENT only (no fix). Round-trip each of the 6; record actual behavior. Do NOT commit.',
  { label: 'verify-current', phase: 'Verify-current', effort: 'high', schema: VC_SCHEMA })

phase('Implement')
const IMPL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'refuse_done', 'reset_done', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    refuse_done: { type: 'string', description: 'Iterator/Deque/FilterExpr/Channel now clean-refuse with distinguishable messages' },
    reset_done: { type: 'string', description: 'Mutex/Lazy now resume to the defined reset state' },
    evidence: { type: 'string', description: 'each type behaves per disposition; check-no-dynamic EXIT 0' },
  },
}
const impl = await agent(CTX + '\n\nVERIFY-CURRENT: ' + JSON.stringify(vc) + '\n\nPHASE 2 — IMPLEMENT the dispositions. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Resumability: honest dispositions for 6 opaque snapshot arms (clean-refuse Iterator/Deque/FilterExpr/Channel; defined-reset Mutex/Lazy)").',
  { label: 'implement', phase: 'Implement', effort: 'high', schema: IMPL_SCHEMA })

phase('Verify+Finish')
const VF_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'per_type_ok', 'no_regression', 'gates', 'merge_ready'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    per_type_ok: { type: 'boolean', description: 'from YOUR OWN runs: each of the 6 behaves per disposition (no silent-drop/crash)' },
    no_regression: { type: 'boolean', description: 'the already-working heap/scalar snapshot round-trip + deep-restore still green' },
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-runtime/shape-vm snapshot tests, brief' },
    merge_ready: { type: 'boolean' },
  },
}
const vf = await agent(CTX + '\n\nVERIFY-CURRENT: ' + JSON.stringify(vc) + '\nIMPL: ' + JSON.stringify(impl) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. From scratch: round-trip each of the 6 — clean-refuse types produce the distinguishable error (NOT a silent wrong value or crash)? reset types resume to the defined state? the pre-existing heap/scalar deep-restore round-trip (e.g. interrupt_mid_loop_resume_yields_full_loop_sum) still green? Add the per-type regression tests if missing; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-runtime and -p shape-vm (snapshot/resume areas). Any silent-drop, crash, or regression = REFUTED. Commit any added tests (git commit --no-verify -m "Resumability opaque-arms finalize: per-type regression tests").',
  { label: 'verify-finish', phase: 'Verify+Finish', effort: 'high', schema: VF_SCHEMA })

return { vc, impl, vf }
