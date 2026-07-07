export const meta = {
  name: 'wf2dfu-async-repair-heap-return-regression',
  description: 'Repair a REGRESSION the WF-2D-fu async lane introduced (caught by supervisor differential, NOT the lane verify). The fix defers ALL zero-arg user async fns to a scalar-only isolated VM, so a zero-arg async fn returning a HEAP value (string/array/object) used in `async let` — which returned its value serially on main — now surfaces NotImplemented. Gate the deferral on the callee\'s DECLARED return type: defer only leaf-scalar returns (int/number/bool/unit); heap/non-scalar returns keep the EAGER path (correct + serial, no NotImplemented). Cover the join-branch deferral too. Independent Opus DIFFERENTIAL verify (branch vs main: no previously-working async program now errors).',
  phases: [
    { title: 'Repair', detail: 'gate deferral on scalar declared-return-type; heap-return keeps eager path (async-let + join)' },
    { title: 'Verify', detail: 'independent Opus differential: heap-return async-let works again; scalar overlap preserved' },
    { title: 'Finish', detail: 'gates + heap-return regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w5-async'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave5/async-real, HEAD 8bddb7f7 — the WF-2D-fu async lane). Build/test via: ' + DX + ' <cmd>. Release binary at ' + WT + '/target/release/shape after you build.',
  '',
  'THE REGRESSION (supervisor-confirmed by differential vs main): the WF-2D-fu fix defers a zero-arg user async fn onto an isolated VM that marshals ONLY leaf scalars back (int/number/bool/unit), surfacing NotImplemented on any heap return. But the deferral guard `deferrable_async_call_target` (crates/shape-vm/src/compiler/expressions/advanced.rs) defers EVERY zero-arg user async fn regardless of return type. So a zero-arg async fn returning a HEAP value now breaks:',
  '  async fn fetch() -> string { await time::sleep(50.0); return "hello" }',
  '  async fn run() { async let a = fetch(); print(await a) }  // await run()',
  'On MAIN this prints "hello" (the RHS ran eagerly + serially and returned the string). On this BRANCH it errors: "Not implemented: WF-2D-fu isolated async task returned a non-scalar result kind String ...". Same for Array<int> (Ptr(TypedArray)) and any object/heap return. THIS IS A REGRESSION — code that worked now errors.',
  '',
  'THE FIX: gate the deferral on the callee\'s DECLARED return type (known at compile time). Defer to the isolated VM ONLY when the declared return is a LEAF SCALAR the isolation boundary can marshal (int / number / bool / unit — the exact set run_isolated_async_fn supports). For a zero-arg async fn whose declared return type is a heap/non-scalar type (string, Array<T>, HashMap, Option/Result, TypedObject, enum, etc.), KEEP THE EAGER PATH (the pre-WF-2D-fu behavior — runs the body inline, returns the value correctly, serially). No NotImplemented for any program that worked before. Apply the SAME scalar-return gate to the join-branch deferral in crates/shape-vm/src/compiler/expressions/misc.rs (a heap-returning async fn inside join race/any/all/settled must also keep the eager path, not NotImplemented).',
  '',
  'NET EFFECT after the fix: zero-arg SCALAR-return async fns still overlap (the WF-2D-fu win preserved — two 1s scalar async-lets ~1s); zero-arg HEAP-return async fns work again (serial, correct, no error); arg-bearing async fns unchanged (eager). No previously-working async program errors.',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): strict typing; the scalar-vs-heap decision uses the PROVEN declared return type (no fabrication, no Bool-default). No coercion, no dynamic fallback. Keep the isolation marshaling\'s hard NotImplemented as a SAFETY net, but the compile-time gate must ensure it is never reached for a previously-working program. ' + DX + ' just check-no-dynamic EXIT 0. Do NOT regress the SIGINT-snapshot path (snapshots_resume must stay green). Do NOT weaken the overlap win.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Repair')
const REPAIR_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'heap_return_works', 'scalar_overlap_preserved', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the scalar-return gate in advanced.rs + misc.rs, brief' },
    heap_return_works: { type: 'boolean', description: 'zero-arg heap-return async fn in async let now prints its value (no NotImplemented)' },
    scalar_overlap_preserved: { type: 'boolean', description: 'two 1s scalar async-lets still overlap (~1s)' },
    evidence: { type: 'string', description: 'captured: fetch()->string prints hello; nums()->array prints [1,2,3]; 2x1s scalar overlap ms; check-no-dynamic EXIT' },
  },
}
const repair = await agent(CTX + '\n\nREPAIR. Gate the async-fn deferral (async-let in advanced.rs + join branches in misc.rs) on the callee declared return type: defer only leaf-scalar returns; heap/non-scalar returns keep the eager path. Build release; prove fetch()->string and nums()->Array both print their values again AND two 1s scalar async-lets still overlap to ~1s. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-2D-fu repair: only defer scalar-return async fns; heap-return keeps eager path (fix regression)").',
  { label: 'repair', phase: 'Repair', effort: 'high', schema: REPAIR_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'no_regression', 'overlap_preserved', 'overlap_ms', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    no_regression: { type: 'boolean', description: 'true iff a DIFFERENTIAL vs main finds NO previously-working async program that now errors (heap-return async-let, join with heap branch, etc.)' },
    overlap_preserved: { type: 'boolean', description: 'two 1s scalar async-lets still overlap (< ~1.3s)' },
    overlap_ms: { type: 'string', description: 'your own measured 2x1s scalar overlap' },
    evidence: { type: 'string', description: 'your own from-scratch DIFFERENTIAL matrix (scalar/string/array/object return x async-let/join) branch-vs-main; concise' },
  },
}
const verify = await agent(CTX + '\n\nREPAIR: ' + JSON.stringify(repair) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. Build a DIFFERENTIAL matrix of async programs (return type in {int, number, bool, string, Array<int>, an object/TypedObject, Option/Result} x form in {async let, join race, join all}) and run EACH on BOTH this branch\'s binary AND a main binary; ANY program that succeeds on main but errors (NotImplemented or otherwise) on the branch = REFUTED (regression not fully fixed). Also confirm two 1s scalar async-lets still overlap to < ~1.3s (the win is preserved) and snapshots_resume stays green. Report the matrix.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    test_added: { type: 'string', description: 'regression test: heap-return async fn in async let returns its value (no NotImplemented) + scalar overlap' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nREPAIR: ' + JSON.stringify(repair) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add a regression test asserting a zero-arg heap-return async fn (e.g. -> string) used in async let returns its value (NOT NotImplemented), alongside the existing scalar-overlap test. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast. Commit (git commit --no-verify -m "WF-2D-fu repair finalize: heap-return async regression test + gates").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { repair, verify, finish }
