export const meta = {
  name: 'wf-time-benchmark-schema',
  description: 'Finding #18 residual: time::benchmark(cb, n) HARD-CRASHES (panic exit 101) with "Missing field \'__variant\' while materializing typed object" (type_schema/mod.rs:336), in pure VM mode too. A BenchmarkResult STRUCT return resolves to an ENUM schema (one expecting __variant) at the module-fn typed-return projection (invoke_module_fn_id_stub -> project_typed_return -> typed_object_from_concrete_pairs, executor/vm_impl/modules.rs). NOT subsumed by WF-3A M1 content-derived schema identity (already merged 1f9b05be — and this still crashes): stdlib_time.rs:138 says the return schema resolves "by its field-name set", a pre-WF-3A heuristic that collides with an enum schema. Diagnose-first, then ROOT-fix (resolve the return schema by the declared Named-type content-id, NOT by field-name-set matching); no benchmark-specific band-aid, no forbidden patterns. Independent Opus verify + regression test.',
  phases: [
    { title: 'Diagnose', detail: 'trace why BenchmarkResult return resolves to an enum (__variant) schema at project_typed_return' },
    { title: 'Fix', detail: 'root-fix: resolve module-fn named-struct return by content-id, not colliding field-name-set' },
    { title: 'Verify', detail: 'independent Opus: benchmark works vm+jit; no schema regressions; forbidden-clean' },
    { title: 'Finish', detail: 'gates + benchmark regression test + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-benchsch'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/time-benchmark-schema, off main HEAD). Build/test via: ' + DX + ' <cmd> (devenv toolchain not auto-loaded — cargo fails without it).',
  '',
  'THE CRASH (verified real at HEAD, Opus-indep, VM AND JIT): `time::benchmark(cb, n)` panics exit 101: "Missing field \'__variant\' while materializing typed object" at crates/shape-runtime/src/type_schema/mod.rs:336 (build_typed_object_with_schema — it iterates schema.fields and finds a field named `__variant` that the value map lacks). `__variant` is the ENUM discriminant field name. So a BenchmarkResult STRUCT return is being materialized against an ENUM schema. Minimal repro:',
  '  use std::core::time',
  '  fn work() {}',
  '  let r = time::benchmark(work, 5)',
  '  print(r.iterations)',
  'Run: ' + DX + ' cargo run --bin shape -- run repro.shape  (and --mode=vm to confirm not JIT-specific). The audit-original symptom (rejects int iterations / never runs callback) is ALREADY fixed — n=5 is accepted and the callback runs; only the RESULT materialization crashes.',
  '',
  'STACK (from the verifier): invoke_module_fn_id_stub -> project_typed_return (crates/shape-vm/src/executor/vm_impl/modules.rs:562) -> typed_object_from_concrete_pairs (modules.rs:535) -> build_typed_object_with_schema (crates/shape-runtime/src/type_schema/mod.rs:335).',
  '',
  'KEY LEAD: crates/shape-runtime/src/stdlib_time.rs:138 says the benchmark return "resolves to the `BenchmarkResult` schema BY ITS FIELD-NAME SET" and stdlib_time.rs:143 declares the return as ConcreteType::Named("BenchmarkResult"). WF-3A M1 (merged 1f9b05be) replaced counter-allocated schema ids with a content-derived SchemaContentId + per-Runtime intern_content handle — BUT this module-fn typed-return projection path evidently still resolves the return schema by matching a FIELD-NAME SET (a pre-WF-3A heuristic), which collides with some ENUM schema (an enum whose materialized shape carries __variant). So the crash is a WF-3A-M1 RESIDUAL at the module-fn return-projection site, not a benchmark quirk.',
  '',
  'DIAGNOSE FIRST (no fix): reproduce; instrument/trace project_typed_return + typed_object_from_concrete_pairs to establish EXACTLY how the return schema is chosen for a ConcreteType::Named("BenchmarkResult") return, WHY it lands on an enum (__variant) schema (field-name-set collision? a by-name lookup hitting the wrong registration? an enum registered with the same field-name set?), and what the CORRECT resolution is (resolve by the declared Named type -> its content-derived schema id / interned content handle, the WF-3A M1 mechanism). Confirm whether other module-fn NAMED-struct returns share the same by-field-name-set path (blast radius).',
  '',
  'ROOT-FIX (no band-aid): make the module-fn typed-return projection resolve a Named-struct return by the declared type\'s content-derived schema identity (WF-3A M1 mechanism), NOT by a field-name-set match that can collide with an enum. Do NOT special-case BenchmarkResult. Do NOT paper over by adding a `__variant` default or skipping the missing field. If the by-field-name-set resolver is shared by legitimate anonymous-struct returns, narrow it so NAMED returns take the named-content-id path while anonymous returns keep structural resolution (mirroring WF-3A M1 named-nominal vs anon-structural).',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/generic-opcode/parallel-discriminator/Convert*To). NO runtime coercion. ADR-005 §1 single-discriminator + ADR-006 preserved. Strict typing intact. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule — a real crash slipping past means a test was missing): add a test that time::benchmark(cb, n) returns a well-formed BenchmarkResult (iterations == n, fields readable) WITHOUT crashing, vm AND jit; plus a guard that a module-fn returning a Named struct whose field-name set overlaps an enum resolves to the STRUCT schema.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['root_cause', 'wrong_schema_reason', 'correct_resolution', 'blast_radius'],
  properties: {
    root_cause: { type: 'string', description: 'exact mechanism: how the return schema is chosen + why it lands on an enum __variant schema' },
    wrong_schema_reason: { type: 'string', description: 'which enum / field-name-set collides, and what registration the resolver hits' },
    correct_resolution: { type: 'string', description: 'the WF-3A M1 named-content-id path that should be used instead' },
    blast_radius: { type: 'string', description: 'other module-fn Named-struct returns on the same path; anon-struct returns that must keep structural resolution' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no fix). Reproduce + trace; establish the exact root cause + correct resolution + blast radius. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'benchmark_works', 'no_bandaid', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the return-projection resolution fix, brief' },
    benchmark_works: { type: 'boolean', description: 'time::benchmark(cb,n) returns a well-formed BenchmarkResult, no crash, vm AND jit' },
    no_bandaid: { type: 'boolean', description: 'root-fix (resolve by declared named content-id); NO benchmark special-case / __variant default / missing-field skip' },
    evidence: { type: 'string', description: 'captured: repro exit 0 vm+jit; iterations==n; check-no-dynamic EXIT 0' },
  },
}
const fix = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — ROOT-FIX. Resolve the module-fn Named-struct return by content-derived schema identity (WF-3A M1 mechanism), not colliding field-name-set. No band-aid. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Fix time::benchmark schema crash: resolve module-fn named-struct return by content-id, not colliding field-name-set (#18 residual)").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'benchmark_works', 'no_schema_regression', 'root_not_bandaid', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    benchmark_works: { type: 'boolean', description: 'from YOUR OWN scratch repro: benchmark returns well-formed result, no crash, vm AND jit' },
    no_schema_regression: { type: 'boolean', description: 'other named-struct + anon-struct + enum returns still materialize correctly (you checked a spread)' },
    root_not_bandaid: { type: 'boolean', description: 'the fix resolves by declared named identity, NOT a benchmark special-case / field default / missing-field skip' },
    evidence: { type: 'string', description: 'your own repros incl. a named-struct-vs-enum field-name collision probe + an anon-struct return control; concise' },
  },
}
const verify = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) does `time::benchmark(work, 5)` return a well-formed BenchmarkResult (iterations==5, fields readable) with NO crash, in BOTH default and --mode=vm? (2) REGRESSION: do other module-fn returns still work — a DIFFERENT named struct, an anonymous-struct return, and an actual ENUM return (which legitimately needs __variant)? Construct a named-struct whose field-name set overlaps an enum and confirm it resolves to the STRUCT. (3) Is the fix a ROOT fix (resolve by declared named content-id) and NOT a band-aid (no benchmark special-case, no __variant default, no missing-field skip)? (4) forbidden-patterns clean (check-no-dynamic EXIT 0), no coercion. Any crash, schema regression, or band-aid = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-runtime/shape-vm tests + the benchmark repro, brief' },
    test_added: { type: 'string', description: 'the benchmark-works + named-vs-enum-collision regression tests' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add the regression tests (benchmark returns well-formed result vm+jit; named-struct-vs-enum field-name collision resolves to struct); no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-runtime and -p shape-vm (schema/module areas). Commit (git commit --no-verify -m "time::benchmark schema fix finalize: regression tests (#18 residual)").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
