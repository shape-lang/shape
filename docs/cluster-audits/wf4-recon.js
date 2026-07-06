export const meta = {
  name: 'wf4-recon',
  description: 'READ-ONLY coverage recon: for every wave-0..3 feature area, audit book documentation + runnable examples + test-suite coverage; plus full book-truth-gate over the ~738-fence universe. Produces the WF-4 work-list.',
  phases: [
    { title: 'Audit', detail: 'parallel per-area book+test coverage audit + full-gate measurement' },
    { title: 'Synthesize', detail: 'consolidate into the feature->book->test work-list' },
  ],
}

const MAIN = '/home/dev/dev/shape-lang/shape'
const BOOK = '/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs'
const GATE = '/home/dev/dev/shape-lang/shape-web/book/book-site/scripts/run-book-truth-gate.mjs'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const COMMON = `You are a READ-ONLY coverage-recon agent for the Shape language. Your job is to AUDIT, not fix. DO NOT edit any file (no book, no tests, no source). DO NOT commit. You may read files, grep, and RUN the already-built release binary.
CONTEXT: waves 0-3 changed/introduced many features. The user requires that EVERY such feature be (a) documented in the book with a runnable gate-green (vm+jit) example, and (b) covered by a comprehensive test suite. You are building the gap list for one feature area.
ENV:
- Binary already built at ${MAIN}/target/release/shape. Use it read-only: ${MAIN}/target/release/shape run <file> --mode vm  and  --mode jit. If genuinely missing, build ONCE: cd ${MAIN} && ${DX} cargo build --release --bin shape (foreground). Do NOT rebuild otherwise.
- Book content root: ${BOOK} (Astro Starlight .mdx; subdirs advanced/ fundamentals/ stdlib/ tooling/ getting-started/ appendix/ examples/).
- Shape source/tests: ${MAIN}/crates, ${MAIN}/bin, ${MAIN}/tools. Tests are #[cfg(test)] unit tests inside source files + integration tests under tools/shape-test/tests/ and bin/shape-cli/tests/.
- Shape is namespaced (no global builtins); script mode runs top-level, fn main not auto-invoked; use std::core::... for stdlib.
- Two feature areas are being actively changed by in-flight workflows: snapshot-resume (WF-2G) and comptime (WF-3D). If you audit those, AUDIT CURRENT MAIN STATE and clearly flag "re-verify after WF-2G/WF-3D land".
Write your findings as your structured final message (machine-consumed). Be concrete: cite chapter file paths, exact example fences that fail (with the error), and test file:count evidence.`

const AREA_SCHEMA = { type: 'object', required: ['area', 'book_status', 'test_status', 'gaps'], properties: {
  area: { type: 'string' },
  book_status: { type: 'string' },   // which chapters exist; documented features; UNDOCUMENTED shipped features; example pass/fail vm+jit
  test_status: { type: 'string' },   // which sub-features have tests (file:count); which have NONE
  gaps: { type: 'string' },          // ordered concrete work-list: book gaps + test gaps for this area
  severity: { type: 'string' },      // how far from "documented+tested" this area is
}}

phase('Audit')

const AREAS = [
  { key: 'polyglot-ffi', features: 'extern C fn (+ out params), fn python, fn typescript, the modular extension system (ext install, LanguageRuntimeVTable), Ffi permission', book: 'advanced/ (polyglot / ffi / extensions), stdlib', tests: 'test-ffi, extensions/, functions_foreign, extern_c, marshal' },
  { key: 'snapshot-resume', features: 'snapshot() -> Result<Snapshot,SnapshotError>, --resume (top-level shape --resume <hash>), interrupt-save (SIGINT->130), short-hash resolve, ModuleFn + heap-element-array projection (WF-2G in flight)', book: 'advanced/resumability, stdlib/core/snapshot', tests: 'snapshot, resume, executor/snapshot' },
  { key: 'remote-distributed', features: '@remote per-function transfer, remote::call(addr,fn,args), remote.execute/ping, closure-over-wire, TLS-on-TCP, permission-over-wire (receiver-owned), MissingModuleFunction resupply, shape serve / wire-serve', book: 'advanced/remote, transport-layer, wire-protocol, distributed', tests: 'remote, serve, wire, remote_builtins' },
  { key: 'polyglot-distributed', features: 'foreign-fn blobs transfer+execute remotely, snapshot/resume across foreign frames, Ffi permission union, the {C,py,ts} x {transfer,snapshot,combined} composition', book: 'advanced/polyglot-distributed (new from WF-2F; may be freshly added)', tests: 'the WF-2F matrix; foreign+remote+snapshot compose' },
  { key: 'async', features: 'async fn, async let, await, async scope, for await x in stream, join all|race|any|settle, real concurrency (WF-2D)', book: 'advanced/async or fundamentals/async', tests: 'async, await, join, concurrency' },
  { key: 'comptime', features: 'comptime blocks/fns, comptime for, annotations (@ann before/after/comptime), directives (remove/set/replace/extend), introspection (target, type_info, implements, build_config, warning, error), diagnostics (LSDS, --diagnostics json), showcases std::serde + std::llm (WF-3D in flight)', book: 'advanced/comptime, comptime-annotations-cookbook, comptime-llm-patterns, content-addressed-bytecode', tests: 'comptime, comptime_builtins, annotations, functions_annotations' },
  { key: 'security-permissions', features: 'the 17 permissions incl Ffi, compile-time capability derivation, runtime check_permission gating, ScopeConstraints (path/host globs), ResourceLimits/sandbox presets, Ed25519 package signing', book: 'advanced/security-permissions, security, sandboxing', tests: 'permission, capability, scope, sandbox, signing, resource_limits' },
  { key: 'drop-raii', features: 'Drop trait, automatic scope-based drop, escape semantics (returned/module-bound referent Drop deferral, ADR-006 2.7.30), & and &mut references, var storage-class inference', book: 'advanced/ownership-deep-dive, memory, references', tests: 'drop, raii, escape, ownership, borrow' },
  { key: 'serialization-stdlib', features: 'json (parse/stringify/navigation), msgpack, toml, yaml, xml, http, time/DateTime — all native round-trip', book: 'stdlib/json, stdlib/serialization, stdlib/http, stdlib/time', tests: 'json, msgpack, toml, yaml, xml, http, datetime, serialization' },
  { key: 'strict-typing', features: 'let-generalization (HM), no-truthiness (bool conditions only), numeric conversion (explicit as for lossy int<->number/narrowing), generic-types-require-args (bare Option/Array invalid), strict TypeDiagnosticMode (ReliableOnly->strict flip)', book: 'fundamentals/types, language reference, appendix', tests: 'strict, type_inference, let_generalization, truthiness, numeric_conversion, generics' },
  { key: 'core-language', features: 'enums (unit/tuple/struct payloads), traits (+extends, impl), generics, pattern matching (guards/destructuring), Result/Ok/Err/?/!!, control flow (break-with-value), strings + f-string format specs + Content builder, collections/ranges, modules (import/export/mod/use), pipe |>, null-coalescing ??', book: 'fundamentals/*, getting-started', tests: 'enum, trait, generic, match, pattern, result, string, collection, module' },
]

const areaReports = await parallel(AREAS.map(a => () =>
  agent(COMMON + `
FEATURE AREA: ${a.key}
Features in scope: ${a.features}
Likely book chapters: ${a.book}
Likely test locations (grep keywords): ${a.tests}
TASKS (read-only):
1. BOOK: locate the chapter(s) for this area under ${BOOK}. List which features from the scope are documented vs UNDOCUMENTED (shipped but absent from the book). For each documented feature, is there a runnable example? Extract the runnable example fences and RUN them (${MAIN}/target/release/shape run <tmpfile> --mode vm AND --mode jit) — record which PASS both, which FAIL (with the error), which are runnable=false/omitted. Prioritize examples for features this area introduced/changed in waves 0-3.
2. TESTS: grep the source + tools/shape-test for this area's features. For each sub-feature, is there a test (give file:approx-count evidence)? List sub-features with NO or THIN coverage.
3. GAPS: produce an ordered, concrete work-list for this area = (book: features to document + examples to add/fix, named) + (tests: suites to add, named per sub-feature). This feeds the WF-4 close workflow, so be specific and actionable.
Return area + book_status + test_status + gaps + severity.`,
    { label: 'audit:' + a.key, phase: 'Audit', effort: 'high', schema: AREA_SCHEMA })
    .then(r => r || { area: a.key, book_status: 'agent died', test_status: '', gaps: 'RE-RUN', severity: 'unknown' })
))

const gate = await agent(COMMON + `
TASK — FULL BOOK TRUTH-GATE MEASUREMENT (read-only). The book truth-gate at ${GATE} normally measures only runnable=true fences (a curated subset). Per the known denominator trap, real book truth is much lower because ~500 runnable=false fences are skipped and many actually fail.
1. Run the gate: cd ${MAIN}/../shape-web/book/book-site && node scripts/run-book-truth-gate.mjs (record the reported pass/total and HOW it selects fences).
2. Then measure the FULL universe: extract ALL shape code fences from ${BOOK} (both runnable=true and runnable=false), and probe each under ${MAIN}/target/release/shape run <tmp> --mode vm (5-10s timeout). Tally: total fences, gate-measured, actually-pass-vm, actually-fail, by chapter/subdir. Identify the chapters with the worst real-pass ratio.
3. List the specific FAILING fences (path + first error line) grouped by failure class (unknown-symbol / unknown-annotation / parse-error / SURFACE / semantic / crash), capped to the top ~40 with a total count per class.
Return: gate_reported (what the gate prints), full_universe (total / real-pass / real-fail counts + per-chapter worst), failing_fences (top failures by class + counts), and delta (how far the real number is from the gate number).`,
  { label: 'full-book-gate', phase: 'Audit', effort: 'high', schema: { type: 'object', required: ['gate_reported', 'full_universe', 'failing_fences', 'delta'], properties: {
    gate_reported: { type: 'string' }, full_universe: { type: 'string' }, failing_fences: { type: 'string' }, delta: { type: 'string' },
  }}})

phase('Synthesize')
const synth = await agent(COMMON + `
TASK — SYNTHESIS (read-only; return the consolidated work-list as your final message, do NOT write files).
Per-area audit reports: ${JSON.stringify(areaReports)}
Full book-gate measurement: ${JSON.stringify(gate)}
Consolidate into a single prioritized WF-4 work-list:
1. book_worklist: per feature area, the exact chapters to create/update + examples to add/fix so every wave-0..3 feature has a runnable gate-green (vm+jit) example. Order by severity. Flag snapshot-resume + comptime as "re-verify after WF-2G/WF-3D land".
2. test_worklist: per feature area, the exact test suites to add (named per sub-feature) so every feature is comprehensively tested. Note where in-flight workflows (WF-2G/WF-3D) already add tests.
3. gate_target: the current real book-truth number (full universe) vs the gate-reported number, and the concrete criterion WF-4 must hit (e.g. flip the N currently-passing runnable=false fences to runnable=true; fix/annotate the M failing).
4. workflow_shape: recommend how to structure the WF-4 close workflow (how many parallel lanes, which areas pipeline together, dependencies on WF-2G/3D/3A merges).
Be exhaustive and concrete — this is THE plan for the book+test completeness close.
Return: book_worklist, test_worklist, gate_target, workflow_shape.`,
  { label: 'synthesize', phase: 'Synthesize', effort: 'high', schema: { type: 'object', required: ['book_worklist', 'test_worklist', 'gate_target', 'workflow_shape'], properties: {
    book_worklist: { type: 'string' }, test_worklist: { type: 'string' }, gate_target: { type: 'string' }, workflow_shape: { type: 'string' },
  }}})

return {
  areas: areaReports.map(r => ({ area: r.area, severity: r.severity })),
  gate: { reported: gate.gate_reported, full: gate.full_universe, delta: gate.delta },
  synthesis: synth,
}
