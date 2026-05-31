// Book-Driven User-Acceptance Validation — v0.3.3 pre-tag gate.
// RATIFIED supervisor + user 2026-05-28. GATED: fire ONLY after the full
// v0.3.3 fix-set is green/allowlisted (FN-REG-CORRECTNESS=0 AND SCOPE-RECLAIM
// closed). Spec: docs/cluster-audits/v0.3.3-book-acceptance/PLAN.md
//
// Fire:  Workflow({ scriptPath: "docs/cluster-audits/v0.3.3-book-acceptance/run-validation.workflow.js" })

export const meta = {
  name: 'book-acceptance-validation',
  description: 'Book-driven user-acceptance gate: per-slice author writes small + 1000-LOC machine-proofable programs following the book, runs VM+JIT, adversarial verify, compile go/no-go',
  phases: [
    { title: 'Author', detail: 'one agent per slice: write small + large program from the book, run VM+JIT, classify' },
    { title: 'Verify', detail: 'second agent per slice: reproduce, confirm expected-values are book-correct, check breadth, re-adjudicate' },
    { title: 'Compile', detail: 'synthesize master truth-set + go/no-go' },
  ],
}

const DOCS = 'shape-web/book/book-site/src/content/docs'
const OUT = 'docs/cluster-audits/v0.3.3-book-acceptance/programs'

// Canonical slice list. books = chapter paths under DOCS (book-primary source).
// caps = sandbox capabilities the large program likely needs (informs the
// non-interactive/deterministic strategy). det = determinism note.
const SLICES = [
  { id: 'variables',        books: ['fundamentals/variables.mdx', 'fundamentals/names-and-scope.mdx'], det: 'pure' },
  { id: 'types-primitive',  books: ['fundamentals/builtin-types.mdx', 'fundamentals/integer-types.mdx'], det: 'pure' },
  { id: 'operators',        books: ['fundamentals/operators.mdx'], det: 'pure' },
  { id: 'control-flow',     books: ['fundamentals/control-flow.mdx'], det: 'pure' },
  { id: 'functions',        books: ['fundamentals/functions.mdx'], det: 'pure (closures + HOFs)' },
  { id: 'strings',          books: ['fundamentals/strings.mdx'], det: 'pure (interpolation, format specs)' },
  { id: 'objects-arrays',   books: ['fundamentals/objects-arrays.mdx'], det: 'pure' },
  { id: 'enums',            books: ['fundamentals/enums.mdx'], det: 'pure' },
  { id: 'traits',           books: ['fundamentals/traits.mdx'], det: 'pure (impl, supertraits, dyn)' },
  { id: 'generics',         books: ['fundamentals/functions.mdx', 'fundamentals/traits.mdx'], det: 'pure (generic fns + bounds; book has no dedicated chapter)' },
  { id: 'pattern-matching', books: ['fundamentals/pattern-matching.mdx'], det: 'pure (destructure, guards, enum/struct/array patterns)' },
  { id: 'error-handling',   books: ['fundamentals/error-handling.mdx'], det: 'pure (Result/Option, ?, !! — HIGH PRIORITY: this was a Wave-1 fix cluster)' },
  { id: 'references',       books: ['fundamentals/references-borrowing.mdx'], det: 'pure (&, &mut, borrow rules)' },
  { id: 'resource-mgmt',    books: ['fundamentals/resource-management.mdx'], det: 'pure (Drop/RAII scope-exit order — assert via side-effect log)' },
  { id: 'modules',          books: ['fundamentals/modules.mdx'], det: 'pure (import/export/mod/use; multi-file in slice dir)' },
  { id: 'async',            books: ['fundamentals/async.mdx'], det: 'deterministic via seeded/ordered tasks; assert final aggregate, not timing' },
  { id: 'datetime',         books: ['fundamentals/datetime.mdx'], det: 'fix epoch inputs; assert formatted output of FIXED datetimes (no now())' },
  { id: 'content',          books: ['fundamentals/content.mdx'], det: 'assert rendered string/structure of Content builders' },
  { id: 'tables',           books: ['fundamentals/tables.mdx', 'getting-started/first-query.mdx'], det: 'fixed in-memory dataset; assert query results' },
  { id: 'comptime',         books: ['advanced/comptime.mdx'], det: 'pure (assert comptime-evaluated constants baked at compile time)' },
  { id: 'annotations',      books: ['advanced/annotations.mdx', 'advanced/comptime-annotations-cookbook.mdx'], det: 'assert annotation before/after/comptime effects via observable result' },
  { id: 'jit-compilation',  books: ['advanced/jit-compilation.mdx'], det: 'compute-heavy deterministic kernel; PRIMARY signal = VM==JIT byte-identical + tier-up does not change result' },
  { id: 'ownership',        books: ['advanced/ownership-deep-dive.mdx'], det: 'pure (storage classes; var smart-default; escape→RC)' },
  { id: 'native-c',         books: ['advanced/native-c-interop.mdx', 'tooling/extensions.mdx'], det: 'extern C fn against a small fixed C stub; assert returned values; out-params' },
  { id: 'security-perms',   books: ['advanced/security-permissions.mdx'], det: 'assert permission grant/deny outcomes under explicit capability sets (deterministic)' },
  { id: 'resumability',     books: ['advanced/resumability.mdx', 'stdlib/core/snapshot.mdx'], det: 'snapshot→resume a deterministic computation; assert resumed result == uninterrupted result' },
  { id: 'transport',        books: ['advanced/transport-layer.mdx', 'stdlib/core/transport.mdx', 'stdlib/core/remote.mdx'], det: 'single-process loopback only; assert round-trip serialization equality; declare network parts run-only-no-assert' },
  { id: 'collections',      books: ['stdlib/core/collections.mdx', 'fundamentals/objects-arrays.mdx'], det: 'pure (HashMap/Array/Set operations; assert)' },
  { id: 'set',              books: ['stdlib/core/set.mdx'], det: 'pure (union/intersection/membership; assert)' },
  { id: 'state',            books: ['stdlib/core/state.mdx'], det: 'pure (state machinery; assert transitions)' },
  { id: 'stdlib-log',       books: ['stdlib/core/log.mdx', 'stdlib/native/log.mdx'], det: 'capture log output deterministically; assert formatted lines' },
  { id: 'stats',            books: ['stdlib/core/distributions.mdx', 'stdlib/core/stochastic.mdx', 'stdlib/core/random.mdx'], det: 'SEED all randomness; assert against known seeded outputs or statistical invariants with fixed seed' },
  { id: 'numeric-sim',      books: ['stdlib/core/monte_carlo.mdx', 'stdlib/core/ode.mdx'], det: 'seed MC; assert ODE solution against analytic value within tolerance' },
  { id: 'rolling',          books: ['stdlib/core/rolling.mdx'], det: 'pure (rolling-window aggregates over fixed series; assert)' },
  { id: 'testing',          books: ['stdlib/core/testing.mdx', 'stdlib/core/property_testing.mdx'], det: 'use the testing facility itself; assert pass/fail counts of an embedded suite' },
  { id: 'math-core',        books: ['stdlib/core/math.mdx', 'stdlib/native/math.mdx'], det: 'pure (assert math fn results)' },
  { id: 'linalg',           books: ['stdlib/math/linalg.mdx'], det: 'pure (matrix ops; assert against hand-computed results)' },
  { id: 'optimize',         books: ['stdlib/math/optimize.mdx'], det: 'fixed objective; assert optimum within tolerance' },
  { id: 'interp-rotation',  books: ['stdlib/math/interpolation.mdx', 'stdlib/math/rotation.mdx'], det: 'pure (assert interpolated/rotated values)' },
  { id: 'domain-finance',   books: ['stdlib/domain/finance.mdx'], det: 'pure (assert financial calcs against known values)' },
  { id: 'domain-iot',       books: ['stdlib/domain/iot.mdx'], det: 'fixed sensor fixtures; assert processed outputs' },
  { id: 'domain-physics',   books: ['stdlib/domain/physics.mdx'], det: 'assert physics calcs against analytic values' },
  { id: 'domain-simulation',books: ['stdlib/domain/simulation.mdx'], det: 'seeded sim; assert final deterministic state' },
  { id: 'serialization',    books: ['stdlib/native/json.mdx', 'stdlib/native/yaml.mdx', 'stdlib/native/toml.mdx', 'stdlib/native/xml.mdx', 'stdlib/native/csv.mdx', 'stdlib/native/msgpack.mdx'], det: 'round-trip: encode→decode→assert equality on fixed structures' },
  { id: 'filesystem',       books: ['stdlib/native/file.mdx', 'stdlib/native/io.mdx', 'stdlib/native/env.mdx', 'stdlib/native/archive.mdx', 'stdlib/native/compress.mdx'], det: 'write→read temp files in slice dir; assert content; deterministic; clean up' },
  { id: 'http',             books: ['stdlib/native/http.mdx', 'examples/web-request.mdx'], det: 'loopback or declare run-only-no-assert; assert request CONSTRUCTION (deterministic) even where response is live' },
  { id: 'regex-unicode',    books: ['stdlib/native/regex.mdx', 'stdlib/native/unicode.mdx'], det: 'pure (assert match/replace/normalize results)' },
  { id: 'crypto',           books: ['stdlib/native/crypto.mdx'], det: 'fixed inputs; assert hashes/signatures against known vectors' },
  { id: 'time-parallel',    books: ['stdlib/native/time.mdx', 'stdlib/native/parallel.mdx'], det: 'fixed durations; parallel map over fixed data; assert order-independent aggregate' },
  { id: 'polyglot-python',  books: ['tooling/python-extension.mdx', 'tooling/polyglot.mdx'], det: 'fn python with fixed inputs; assert returned values (requires python extension built)' },
  { id: 'polyglot-ts',      books: ['tooling/typescript-extension.mdx', 'tooling/polyglot.mdx'], det: 'fn typescript with fixed inputs; assert returned values (requires ts extension built)' },
]

const COMMON = `
Repo: /home/dev/dev/shape-lang/shape. Book root: ${DOCS}.
Run programs with the canonical (ii) F' release-binary harness (NOT pipe-to-tail):
  out=$(timeout 30 ./target/release/shape run --mode $MODE $FILE 2>&1); ec=$?; last=$(echo "$out" | tail -1)
$MODE in {interp, jit} (or the binary's VM/JIT mode flags — discover from 'shape run --help'). Build the release binary first if absent: cd /home/dev/dev/shape-lang/shape && direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape (timeout 600000).
HARD RULES (standing bindings): NO 'git stash' in any form. No source changes to the compiler/stdlib. Read/Write ONLY your own slice dir under ${OUT}/<slice>/. No commits (team-lead commits results at close).
`

const AUTHOR_SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    slice: { type: 'string' },
    small: {
      type: 'object', additionalProperties: false,
      properties: {
        loc: { type: 'number' }, vm_ec: { type: ['number','null'] }, jit_ec: { type: ['number','null'] },
        vm_jit_byte_identical: { type: 'boolean' }, self_check_passed: { type: 'boolean' },
        classification: { type: 'string' }, notes: { type: 'string' },
      },
      required: ['loc','vm_jit_byte_identical','self_check_passed','classification','notes'],
    },
    large: {
      type: 'object', additionalProperties: false,
      properties: {
        loc: { type: 'number' }, app_description: { type: 'string' },
        vm_ec: { type: ['number','null'] }, jit_ec: { type: ['number','null'] },
        vm_jit_byte_identical: { type: 'boolean' }, self_check_passed: { type: 'boolean' },
        machine_proofable: { type: 'boolean' }, num_assertions: { type: ['number','null'] },
        classification: { type: 'string' }, notes: { type: 'string' },
      },
      required: ['loc','app_description','vm_jit_byte_identical','self_check_passed','machine_proofable','classification','notes'],
    },
    book_gaps: { type: 'array', items: { type: 'string' } },
    book_wrong: { type: 'array', items: { type: 'string' } },
    files_written: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
  required: ['slice','small','large','book_gaps','book_wrong','files_written','summary'],
}

const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    slice: { type: 'string' },
    reproducible: { type: 'boolean' },
    expected_values_book_sound: { type: 'boolean' },
    expected_values_notes: { type: 'string' },
    breadth_adequate: { type: 'boolean' },
    breadth_notes: { type: 'string' },
    reclassifications: { type: 'array', items: { type: 'string' } },
    release_blocking_findings: { type: 'array', items: { type: 'string' } },
    verdict: { type: 'string', enum: ['PASS','FAIL','PARTIAL'] },
    notes: { type: 'string' },
  },
  required: ['slice','reproducible','expected_values_book_sound','breadth_adequate','reclassifications','release_blocking_findings','verdict','notes'],
}

function authorPrompt(s) {
  return `${COMMON}
SLICE: ${s.id}. Book chapter(s) (book-PRIMARY source): ${s.books.map(b => `${DOCS}/${b}`).join(', ')}.
Determinism strategy for this slice: ${s.det}.

You are simulating a REAL USER who learns Shape from the book. Methodology = book-primary, reference-fallback:
- Read the assigned chapter(s) thoroughly FIRST. Write programs using ONLY what the book teaches.
- If the book is SILENT on something you need, you MAY consult the shape MCP tools (get_shape_syntax / get_shape_api / get_shape_examples / search_shape_docs) or the reference — but EVERY such fallback is a BOOK-GAP finding: record it in book_gaps with what the book failed to cover.
- If the book DOCUMENTS something the language does NOT actually do (you followed the book correctly and it fails) → that is BOOK-WRONG: record in book_wrong + classify the test failure.

DELIVERABLE 1 — small program (~20-60 LOC): idiomatic, exercises the chapter's core. Self-checking: assert results, print "ALL_CHECKS_PASSED" on success.

DELIVERABLE 2 — large program (~1000 LOC): a REAL-WORLD, NON-INTERACTIVE, MACHINE-PROOFABLE application rooted in this slice (e.g. error-handling → a parser-with-error-recovery; strings → a template/markup processor; linalg → a 3D transform pipeline; serialization → a config round-trip tool). It must:
  - be NON-INTERACTIVE: no stdin, no REPL, no blocking on live network; deterministic (seed randomness, fix clocks, use local fixtures);
  - be MACHINE-PROOFABLE: compute many results and ASSERT each against an EXPECTED VALUE. **CRITICAL: derive every expected value from BOOK SEMANTICS and write it BEFORE the first run. NEVER back-fill an expected value from observed output — that would assert the bug.** On all-pass, print "ALL_CHECKS_PASSED"; on any mismatch print "CHECK_FAILED: <which> expected=<e> got=<g>".
  - where a behavior is genuinely non-deterministic (live http/time), use the closest deterministic proxy and mark that section run-only-no-assert with rationale.

PROCEDURE:
1. mkdir -p ${OUT}/${s.id}. Write small.shape + large.shape (+ any module/fixture files) there.
2. Run BOTH under VM and JIT via the harness. Capture ec + full output for each.
3. DISCIPLINE: if a program fails, decide AUTHOR-ERROR (your own typo — fix it and re-run, a real user would) vs language defect (book says it works + it doesn't → record, classify, DO NOT work around). Record FIRST-RUN truth for defects.
4. Write ${OUT}/${s.id}/REPORT.md: per-program result, expected-value rationale (cite book sections), failure classifications, book_gaps, book_wrong.
5. Return the structured result. classification ∈ {PASS, FN-REG-CORRECTNESS, SCOPE-RECLAIM, BOOK-WRONG, BOOK-GAP, V0.4-DEFER, AUTHOR-ERROR}. vm_jit_byte_identical = (VM stdout == JIT stdout byte-for-byte).`
}

function verifyPrompt(s, author) {
  return `${COMMON}
SLICE: ${s.id}. ADVERSARIAL VERIFY of the author agent's work. Author summary: ${author ? author.summary : '(author returned null — investigate the slice dir directly)'}.
Author files: ${OUT}/${s.id}/ (small.shape, large.shape, REPORT.md). Book: ${s.books.map(b => `${DOCS}/${b}`).join(', ')}.

Do NOT trust the author's PASS claims. Verify:
1. REPRODUCIBLE: re-run small.shape + large.shape under BOTH VM and JIT. Confirm the author's reported ec + output + byte-identical claim. Report mismatches.
2. EXPECTED-VALUES-BOOK-SOUND (the anti-tautology check — most important): pick a SAMPLE of the large program's assertions and INDEPENDENTLY recompute the expected value from book semantics + first principles. Confirm the expected values encode book-CORRECT answers, not values back-filled from buggy output. If any assertion asserts against what the (possibly-buggy) VM produced rather than the truth, FAIL with that finding — a green machine-proof against a wrong expected value is worse than a visible failure.
3. BREADTH: does the large program actually exercise the chapter's DOCUMENTED breadth (the methods/forms/edge-cases the book shows), or just a narrow happy path? Note under-coverage.
4. RE-ADJUDICATE: re-check each failure classification (AUTHOR-ERROR vs real defect; SCOPE-RECLAIM vs FN-REG-CORRECTNESS per the dated-pull-in taxonomy in docs/cluster-audits/v0.3-classification/TAXONOMY.md). List reclassifications.
5. release_blocking_findings = the FN-REG-CORRECTNESS / SCOPE-RECLAIM / BOOK-WRONG / VM!=JIT findings that block the v0.3.3 tag.
verdict: PASS (slice clean), FAIL (release-blocking finding), PARTIAL (clean but breadth/book-gap issues).`
}

// ---- Phase 1+2: author -> verify per slice, no barrier (pipeline) ----
const perSlice = await pipeline(
  SLICES,
  (s) => agent(authorPrompt(s), { label: `author:${s.id}`, phase: 'Author', schema: AUTHOR_SCHEMA }),
  (author, s) => agent(verifyPrompt(s, author), { label: `verify:${s.id}`, phase: 'Verify', schema: VERIFY_SCHEMA })
    .then(v => ({ slice: s.id, author, verify: v })),
)

const slices = perSlice.filter(Boolean)

// ---- Phase 3: compile master truth-set ----
const COMPILE_SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    go_no_go: { type: 'string', enum: ['GO','NO-GO'] },
    total_slices: { type: 'number' },
    slices_pass: { type: 'number' }, slices_fail: { type: 'number' }, slices_partial: { type: 'number' },
    release_blocking: { type: 'array', items: { type: 'string' } },
    vm_jit_divergences: { type: 'array', items: { type: 'string' } },
    book_wrong: { type: 'array', items: { type: 'string' } },
    book_gaps: { type: 'array', items: { type: 'string' } },
    headline: { type: 'string' },
  },
  required: ['go_no_go','total_slices','slices_pass','slices_fail','slices_partial','release_blocking','vm_jit_divergences','book_wrong','book_gaps','headline'],
}

const compiled = await agent(
  `${COMMON}
COMPILE the book-acceptance master truth-set from ${slices.length} slice results (author + adversarial verify each).
Per-slice verdicts + findings:
${JSON.stringify(slices.map(x => ({ slice: x.slice, verdict: x.verify && x.verify.verdict, blocking: x.verify && x.verify.release_blocking_findings, vm_jit: x.author && { small: x.author.small.vm_jit_byte_identical, large: x.author.large.vm_jit_byte_identical }, book_wrong: x.author && x.author.book_wrong, book_gaps: x.author && x.author.book_gaps })), null, 1)}

Write docs/cluster-audits/v0.3.3-book-acceptance/RESULTS.md: per-slice table (slice | small | large | VM==JIT | verdict | blocking-findings), then sections for RELEASE-BLOCKING findings (FN-REG-CORRECTNESS / SCOPE-RECLAIM / BOOK-WRONG / VM!=JIT), BOOK-GAP findings (route to book owner), and the go/no-go.
go_no_go = GO only if ZERO release-blocking findings across all slices. Otherwise NO-GO with the blocking list.
Return the structured summary.`,
  { label: 'compile:master', phase: 'Compile', schema: COMPILE_SCHEMA },
)

return { compiled, slices }
