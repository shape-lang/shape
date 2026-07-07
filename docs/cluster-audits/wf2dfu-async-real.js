export const meta = {
  name: 'wf2dfu-async-real-concurrency',
  description: 'WF-2D-fu (async re-route, user-ruled). WF-2D claimed real async concurrency (added async_runtime.rs) but the meta-audit REFUTED it: user-defined async fns still run SERIALLY (two 1s async-lets = ~2005ms, should be ~1003ms). Diagnose-first (reproduce the serial behavior from scratch, do NOT trust the prior claim; pin WHY user async fns serialize), implement REAL concurrency (async let defers RHS as a thunk before spawn; scheduler runs tasks concurrently), independent Opus WALL-CLOCK verify, gates. Also re-confirm the two unconditional bugs: module-qualified calls inside async fns; top-level await time::sleep panic.',
  phases: [
    { title: 'Diagnose', detail: 'reproduce serial 2x1s from scratch; pin why user async fns serialize; recheck the 2 bugs' },
    { title: 'Fix', detail: 'real concurrency: async-let thunk deferral + concurrent scheduler; fix any live bug' },
    { title: 'Verify', detail: 'independent Opus wall-clock: two 1s tasks < ~1.3s; module-call-in-async compiles; no sleep panic' },
    { title: 'Finish', detail: 'gates + timing regression tests + book async examples green' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w5-async'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave5/async-real, off main). Build/test via: ' + DX + ' <cmd>. Release binary at ' + WT + '/target/release/shape after you build.',
  '',
  'DEFECT (meta-audit REFUTED WF-2D\'s green claim): user-DEFINED async functions still run SERIALLY. WF-2D added crates/shape-vm/src/executor/async_runtime.rs (futures spawn + mpsc completion) and claimed real concurrency for async MODULE functions, but two independent 1-second async tasks launched via async-let still total ~2005ms wall-clock instead of overlapping to ~1003ms. So the concurrency is NOT actually happening for the user async-fn path. Do NOT trust the prior claim — reproduce from scratch and pin the true root.',
  '',
  'SUSPECTED ROOTS (verify, do not assume): (1) async let compiles its RHS EAGERLY before SpawnTask (advanced.rs:745 area) — the task body is already evaluated on the spawning thread, so spawning is a no-op; the fix is closure-THUNK deferral (compile the RHS as a deferred thunk that the spawned task runs). (2) the scheduler/async_runtime may await each task to completion at the await point in source order rather than spawning all then joining. (3) block_in_place / a blocking sleep on the spawner thread.',
  '',
  'UNCONDITIONAL BUGS to re-confirm + fix if still live: (a) module-qualified calls INSIDE an async fn fail "Unknown qualified call" (function_calls.rs:3185 — a source-order registration bug); repro: an async fn body that calls e.g. time::millis() or math::sqrt(). (b) top-level `await time::sleep(...)` Rust-panics via block_in_place (modules.rs:733 area); repro: a top-level await of a sleep.',
  '',
  'GOAL SEMANTICS (implement real concurrency): two independent async-lets each doing ~1s of work complete in ~1s total (overlap), not 2s. `join race { }` returns at the FIRST completion (and ideally cancels losers). `join any` skips failures. Keep it strict-typed and sound. If full join race/any/settle/cancellation is too broad for one lane, PRIORITIZE the core: async-let real overlap + the 2 unconditional bugs; note race/any/settle/cancellation status honestly (may be partial / v0.4).',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): strict typing (no runtime coercion, no dynamic fallback); no ValueWord/tag-decode/Bool-default; ' + DX + ' just check-no-dynamic EXIT 0. Async must not corrupt the snapshot/resume path (the SIGINT-snapshot fix must stay green). Real concurrency = actual thread/future parallelism via the existing async_runtime.rs, NOT a fake that still serializes.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields. Put timing numbers in the fields as plain text (e.g. "2x1s=1012ms").',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['serial_confirmed', 'serial_ms', 'root', 'bugs_status'],
  properties: {
    serial_confirmed: { type: 'boolean', description: 'true iff two 1s async-lets measurably serialize (~2s) from scratch' },
    serial_ms: { type: 'string', description: 'measured wall-clock of the 2x1s repro, e.g. "2007ms"' },
    root: { type: 'string', description: 'the true root cause of the serialization (eager RHS compile / serial await / block_in_place), with file:line' },
    bugs_status: { type: 'string', description: 'the 2 unconditional bugs: still-broken(root) | already-fixed, brief' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no fix). Build release. Reproduce the serial 2x1s async-let timing from scratch (use a real time source; print elapsed). Pin the true root. Re-check the 2 unconditional bugs. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'overlap_ms', 'bugs_fixed', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the concurrency fix (thunk deferral / scheduler), brief' },
    overlap_ms: { type: 'string', description: 'measured 2x1s wall-clock AFTER the fix, e.g. "1012ms"' },
    bugs_fixed: { type: 'string', description: 'the 2 unconditional bugs: fixed | already-ok, brief' },
    evidence: { type: 'string', description: 'captured timing + repro outputs; check-no-dynamic EXIT' },
  },
}
const fix = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — FIX real concurrency at the pinned root (async-let RHS thunk deferral + concurrent scheduling), plus any still-live unconditional bug. Build release; measure 2x1s overlap (must drop to ~1s). ' + DX + ' just check-no-dynamic EXIT 0. Commit each logical fix (git add -A && git commit --no-verify -m "WF-2D-fu: <fix>").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'real_overlap', 'overlap_ms', 'bugs_ok', 'no_strict_weakening', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    real_overlap: { type: 'boolean', description: 'two 1s async-lets measurably overlap (< ~1.3s) from your OWN scratch repro' },
    overlap_ms: { type: 'string', description: 'your own measured 2x1s wall-clock' },
    bugs_ok: { type: 'boolean', description: 'module-qualified call in async compiles + runs; top-level await sleep does not panic' },
    no_strict_weakening: { type: 'boolean', description: 'no coercion/dynamic-fallback/Bool-default; snapshot path intact' },
    evidence: { type: 'string', description: 'your own from-scratch timing + repros; concise' },
  },
}
const verify = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. Write your OWN 2x1s async-let timing program from scratch and measure wall-clock (must be < ~1.3s for real overlap; ~2s = still serial = REFUTED). Independently: (a) an async fn calling a module-qualified fn compiles+runs; (b) top-level await time::sleep does not panic. Grep the diff for coercion/dynamic-fallback/Bool-default. Serial timing or any strict-weakening = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'book_status', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    tests_added: { type: 'string', description: 'timing regression test(s) name + location' },
    book_status: { type: 'string', description: 'book async examples green as written, or what remains' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED/PARTIAL-with-real-overlap; else merge_ready:false + what remains). Add a timing regression test (two 1s async tasks total < ~1.3s). Check the book async pages execute green as written (or note precisely what stays v0.4). Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast. Commit (git commit --no-verify -m "WF-2D-fu finalize: async real concurrency + timing tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
