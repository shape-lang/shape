export const meta = {
  name: 'wf4-index-type-strict',
  description: 'Strict-typing hole (reliableonly_strict_bypass class): array index accepts number / unbound-var, so arr[1.5], arr[n:number], arr[time::millis()] all COMPILE and use a float as an int index (wrong element / garbage index / OOB at runtime). An index must REQUIRE int at compile time. BLAST-RADIUS-GATED: diagnose + measure how many existing tests break if we require int; if small + all genuine true-positives, fix + verify + finish; if large, STOP and report for a scope decision (do NOT weaken the fix or auto-tighten a wide surface).',
  phases: [
    { title: 'Diagnose', detail: 'confirm the hole, pin the index type-check site, MEASURE blast radius' },
    { title: 'Fix', detail: 'GATED: only if blast radius small+TP — require int index (no number->int coercion)' },
    { title: 'Verify', detail: 'independent Opus: number/float index errors; int index works; no coercion added' },
    { title: 'Finish', detail: 'gates + regression tests (or blocked-for-scope report)' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w4-index'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave4/index-type, off main). Build/test via: ' + DX + ' <cmd>.',
  '',
  'THE HOLE (reliableonly_strict_bypass class — the top strict-typing failure class): array indexing accepts a non-int index. All THREE of these COMPILE today (built release binary at ' + WT + '/target/release/shape after you build):',
  '  let arr = [1,2,3]; let x = arr[1.5]            // compiles; wrong/garbage',
  '  let arr = [1,2,3]; let n: number = 1.0; arr[n] // compiles; wrong element',
  '  use std::core::time; let arr=[1,2,3]; arr[time::millis()] // compiles -> runtime OOB (float used as int index)',
  'A number (f64) is silently used as an int index. In strict Shape an array index MUST be int at compile time; a number index must be a COMPILE ERROR (the user must write an explicit `as int` cast — number->int is lossy per the numeric-conversion rule, so it is NOT implicit).',
  '',
  'CORRECT FIX (strict): the index-expression type checker/compiler must PROVE the index operand is int (or a type that is int, e.g. an int-typed var/literal). A number/float/decimal/unbound-var index = compile error with a clear message ("array index must be `int`; got `number` — add an explicit `as int` cast if truncation is intended"). Do NOT insert an implicit number->int coercion opcode (FORBIDDEN — no IntToNumber/NumberToInt/Convert*To). Do NOT accept an unbound inference var silently (that is the same unify-with-anything hole seen in argument position). int literals and int vars index fine; ranges/slices keep their existing typed behavior.',
  '',
  'BLAST-RADIUS GATE (important — this is a strict tightening; it may break a lot of existing code the way the strict-flip did). In DIAGNOSE you MUST measure: build the fix mentally / or via a scratch probe, and estimate/count how many existing tests + stdlib .shape files + book examples use a NON-int index today (grep for indexing with number-typed operands; run the touched test targets to count real breaks). Classify each break as TRUE-POSITIVE (genuinely should be int, fix the test/source) vs FALSE-POSITIVE (a case where the index IS int but inference loses it — those are checker bugs the fix must NOT trigger).',
  '  - If blast radius is SMALL (roughly <~25 breaks) AND essentially all TRUE-POSITIVE: proceed to Fix + Verify + Finish, fixing the genuine TP call sites too.',
  '  - If blast radius is LARGE, or there are systematic FALSE-POSITIVES (int index erased to unknown/number by an inference gap): STOP. Do the ROOT inference fix for the FP class if it is small and clearly a checker bug; otherwise report status blocked-for-scope with the measured numbers + the FP roots, and do NOT land a wide half-broken tightening. Surfacing a large blast radius is the CORRECT outcome, not a failure.',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): strict typing; int and number are SEPARATE; NO implicit numeric coercion (number->int needs explicit `as`); NO dynamic fallback; NO Bool-default; NO unbound-var-unifies-with-int. ' + DX + ' just check-no-dynamic EXIT 0. Do NOT weaken the check to make existing tests pass — fix the genuine TP call sites, and for FP fix the inference root or surface.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['hole_confirmed', 'site', 'blast_small', 'blast_detail'],
  properties: {
    hole_confirmed: { type: 'boolean', description: 'arr[1.5]/arr[number]/arr[millis()] compile today (reproduced)' },
    site: { type: 'string', description: 'file:line of the index-expression type-check that must require int' },
    blast_small: { type: 'boolean', description: 'true iff requiring int index breaks few (<~25) existing tests AND ~all are true-positives (safe to proceed to fix)' },
    blast_detail: { type: 'string', description: 'measured count of breaks + TP-vs-FP split + any FP inference-gap root; brief' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no landed fix). Reproduce the hole (build release, run the three probes). Pin the index type-check site. MEASURE the blast radius of requiring int (grep + run touched targets); split TP vs FP. Set blast_small honestly. Do NOT commit a fix.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'tp_fixed', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'blocked-for-scope', 'partial'] },
    files_changed: { type: 'string', description: 'the index type-check change + any TP call-site fixes, brief' },
    tp_fixed: { type: 'string', description: 'genuine true-positive sites fixed (or "n/a — blocked")' },
    evidence: { type: 'string', description: 'arr[number] now errors; arr[int] works; check-no-dynamic EXIT; or the blocked-for-scope numbers' },
  },
}
const fix = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — GATED. If diag.blast_small is TRUE: implement the strict index-int requirement (no coercion), fix the genuine TP call sites, build release, prove arr[number]/arr[1.5]/arr[millis()] now ERROR while arr[int-literal] and arr[int-var] still work. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-4 index-type: require int array index (strict, no coercion)"). If diag.blast_small is FALSE: do the small FP-root inference fix if clearly a checker bug, else return status:blocked-for-scope with the measured numbers + FP roots (do NOT land a wide half-broken tightening).',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'number_index_errors', 'int_index_works', 'no_coercion', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'blocked-for-scope'] },
    number_index_errors: { type: 'boolean', description: 'arr[1.5], arr[n:number], arr[millis()] all compile-ERROR now' },
    int_index_works: { type: 'boolean', description: 'arr[0], arr[i:int] still compile+run correctly (no false positive)' },
    no_coercion: { type: 'boolean', description: 'no implicit number->int coercion opcode / dynamic fallback / unbound-var-accept introduced (grep the diff)' },
    evidence: { type: 'string', description: 'your own from-scratch repros incl. a correct int-index control; concise' },
  },
}
const verify = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. If fix status is blocked-for-scope, set verdict:blocked-for-scope and sanity-check the numbers. Else from scratch: (1) arr[1.5], arr[n:number], arr[time::millis()] ALL compile-error? (2) arr[0] and arr[i:int] still work (no false positive on legit int indexing)? (3) grep the diff: NO implicit number->int coercion, no dynamic fallback, no unbound-var-unifies-with-int. Any hole remaining or any FP or any coercion = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    tests_added: { type: 'string', description: 'regression tests: number index errors + int index works' },
    merge_ready: { type: 'boolean', description: 'true only if CONFIRMED; false if blocked-for-scope or REFUTED' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; if blocked-for-scope or REFUTED, merge_ready:false + what remains + the scope question for the user). Add regression tests: a number/float index is a compile error; an int index works. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast. Commit (git commit --no-verify -m "WF-4 index-type finalize: strict int-index + regression tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
