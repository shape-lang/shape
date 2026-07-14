export const meta = {
  name: 'wf-finance-stdlib-sweep',
  description: 'Tail #3 (user-greenlit 2026-07-07): finance stdlib-SOURCE sweep (the #15 survivors after the compiler occurs-check root-fix `37aa861d`). These are bugs in the std::finance .shape SOURCE (crates/shape-runtime/stdlib-src/finance/*.shape), not the compiler: (a) let-as-mutable (a `let` binding that the source mutates — needs `let mut`/`var`, or a genuine logic fix), (b) missing signal / is_signal (referenced functions that do not exist / are not exported), (c) Float64 object-field subtraction (subtracting two number fields of an object fails). Diagnose each by compiling + running the finance package, fix at the SOURCE (make std::finance usable), verify a real finance program (e.g. an indicator/backtest) compiles + runs green vm+jit. Independent Opus verify. No forbidden patterns; strict-typing intact (int/number separate, explicit casts).',
  phases: [
    { title: 'Diagnose', detail: 'compile+run std::finance; pin each survivor (let-as-mutable / missing signal,is_signal / Float64 field subtraction) to a file:line + root' },
    { title: 'Fix', detail: 'fix at the .shape source; make std::finance usable' },
    { title: 'Verify+Finish', detail: 'independent Opus: a real finance program runs green vm+jit; gates + regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-finance'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/finance-stdlib-sweep, off main HEAD). Build/run via: ' + DX + ' <cmd>. The std::finance SOURCE is at crates/shape-runtime/stdlib-src/finance/*.shape (interfaces, patterns, signals, types, risk, indicators/{trend,moving_averages,volatility,oscillators,atr,...}). The compiler-side root (from-import stack overflow via missing occurs-check) is ALREADY fixed (37aa861d) — these are SOURCE defects that survive.',
  '',
  'THE 3 SURVIVORS (diagnose precisely, then fix at source): (a) LET-AS-MUTABLE — a `let` binding in the finance source is reassigned/mutated (Shape: `let` is immutable; needs `let mut` or `var`, OR the mutation is a genuine logic bug to restructure). (b) MISSING signal / is_signal — the source references `signal(...)` / `is_signal(...)` (likely in signals.shape or a consumer) that are undefined or not exported → define/export them correctly, or fix the call. (c) FLOAT64 OBJECT-FIELD SUBTRACTION — subtracting two `number` (f64) fields of an object (e.g. `bar.high - bar.low`) fails to compile/run → determine why (type inference on object-field access? a number/int mismatch needing explicit handling?) and fix at source (or, if it is a genuine compiler gap in object-field number inference, SURFACE it precisely rather than papering over).',
  '',
  'APPROACH: import + exercise the finance package under vm AND jit — e.g. `from std::finance::indicators::... use {...}` and call an indicator on sample bars; run a small backtest if one exists. Reproduce each survivor, fix at the .shape source so std::finance is genuinely usable, and confirm strict-typing holds (int/number separate; explicit `as` casts where lossy). Do NOT weaken the type system to make finance compile.',
  '',
  'CONSTRAINTS: NO forbidden patterns. Strict-typing intact. If a survivor turns out to be a real COMPILER gap (not a source bug) → SURFACE it precisely (root, repro) rather than band-aid the source. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a test (shape-test or a gate-runnable finance example) that a real std::finance indicator/backtest compiles + runs green vm+jit. No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Diagnose')
const D_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['let_as_mutable', 'missing_signal', 'float64_field_sub', 'any_compiler_gap'],
  properties: {
    let_as_mutable: { type: 'string', description: 'file:line + root of the let-as-mutable survivor' },
    missing_signal: { type: 'string', description: 'file:line + root of the missing signal/is_signal' },
    float64_field_sub: { type: 'string', description: 'file:line + root of the Float64 object-field subtraction failure' },
    any_compiler_gap: { type: 'string', description: 'any survivor that is a real compiler gap (not a source bug) → surface (or "none — all source-fixable")' },
  },
}
const d = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE (compile+run std::finance; pin each survivor). Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: D_SCHEMA })

phase('Fix')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'fixes', 'usable', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    fixes: { type: 'string', description: 'the source fixes applied per survivor' },
    usable: { type: 'boolean', description: 'std::finance now compiles + a real indicator/backtest runs vm+jit' },
    evidence: { type: 'string', description: 'a finance program runs green; check-no-dynamic EXIT 0; strict-typing intact' },
  },
}
const f = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(d) + '\n\nPHASE 2 — FIX at the .shape source (or surface a real compiler gap). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Finance stdlib-source sweep: fix let-as-mutable + missing signal/is_signal + Float64 field subtraction (std::finance usable)").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: F_SCHEMA })

phase('Verify+Finish')
const VF_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'usable', 'strict_intact', 'gates', 'merge_ready'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    usable: { type: 'boolean', description: 'from YOUR OWN run: a real std::finance indicator/backtest runs green vm+jit' },
    strict_intact: { type: 'boolean', description: 'no type-system weakening; int/number separate; no forbidden pattern' },
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + the finance regression test, brief' },
    merge_ready: { type: 'boolean' },
  },
}
const vf = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(d) + '\nFIX: ' + JSON.stringify(f) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. From scratch: does a real std::finance indicator/backtest compile + run green vm AND jit now? Was the type system weakened to get there (int/number unified, a coercion added, a forbidden pattern)? Add the gate-runnable finance regression test if missing; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, and the finance test. Any remaining finance breakage, type-system weakening, or forbidden pattern = REFUTED. Commit any added test (git commit --no-verify -m "Finance sweep finalize: gate-runnable finance regression test").',
  { label: 'verify-finish', phase: 'Verify+Finish', effort: 'high', schema: VF_SCHEMA })

return { d, f, vf }
