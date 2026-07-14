export const meta = {
  name: 'wf-book-truth-campaign',
  description: 'USER-DIRECTED (2026-07-07): book-truth campaign. HEAD measurement (b334e8af): 748 fences; curated gate runnable=true 246/246 green vm+jit; honest book-truth 362/426 = 85% of genuinely-runnable green; 64 real defects (45 DOC-WRONG valid-looking-code-the-compiler-rejects, 18 FEATURE-BROKEN, 1 VM/JIT-divergence) + 116 runnable=false-but-actually-green promotion candidates. This first campaign does the HIGH-CONFIDENCE in-mandate work: (A) fix the 45 DOC-WRONG in the book (../shape-web) — CORRECT the code to what actually works where a working form exists, else HONEST-MARK it (runnable=false with a clear SURFACE/pending note, never broken-code-presented-as-working); (B) promote the safe deterministic subset of the 116 hidden-but-green to runnable=true (grow the honest gate); (C) fix the 1 VM/JIT-divergence (datetime.mdx:183) + the VM dev-mode auto-print `{"Integer":..}` internal-wrapper leak in shape/; (D) TRIAGE the 18 FEATURE-BROKEN into a routed roadmap (tractable-now vs known-deferral, each cited to a tracking lane) — do NOT blind-fix them (several overlap feature-tail items under separate decision). Re-run the measurement to PROVE the honest % rose, the curated gate stays 246/246 (no regression), and promotions are green. Two repos: shape/ code fixes in the worktree (merged normally); ../shape-web book edits committed to shape-web git. Independent Opus verify.',
  phases: [
    { title: 'Triage', detail: 'classify the 64 defects: doc-correct vs honest-mark vs feature-route; confirm 116 promotions deterministic-safe' },
    { title: 'FixBook', detail: 'correct/honest-mark the 45 DOC-WRONG + promote the safe 116 in ../shape-web; fix the divergence + auto-print in shape/' },
    { title: 'Measure', detail: 're-run book-truth: honest % up, curated gate 246/246, promotions green, divergence gone' },
    { title: 'Finish', detail: 'gates on shape/ changes + commit shape-web + the 18-feature-broken routed roadmap' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-booktruth'
const BOOK = '/home/dev/dev/shape-lang/shape-web/book'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const ROADMAP = [
  'MEASUREMENT DIGEST (HEAD b334e8af, authoritative — do not re-discover, re-verify):',
  'Total ```shape fences 748; runnable=true 246 (curated gate 246/246 green vm+jit); full-universe green 362/748=48.4%; HONEST book-truth 362/426=85% (excl 299 fragments + 7 negative-demos + 16 env); 116 runnable=false-but-green (promotion candidates).',
  'FAILURE BUCKETS (all failures are in the runnable=false set; runnable=true is clean): (d) FRAGMENT/illustrative 299 (undefined var 86 / undefined fn 59 / qualified-call-needs-use 116 / partial-syntax 25 — correctly no-run); negative-demo 7 (deliberate failing code — correctly no-run); (a) DOC-WRONG 45 (valid-looking code presented as working, compiler rejects); (b) FEATURE-BROKEN 18; (e) FLAKY/ENV 16 (polyglot ext not installed + network); (c) VM/JIT-DIVERGENCE 1 (datetime.mdx:183 — VM auto-prints {"Integer":1705314600}, JIT prints nothing).',
  'TOP REAL-DEFECT CLUSTERS (the 64 = a+b+c): 1) JIT-compile SURFACE move-semantics/Rvalue::Aggregate route-A (8): ownership-deep-dive:459, content:341. 2) @remote annotation arg carrier (6): modules.mdx:48/57 "no statically proven typed-array element carrier". 3) optional chaining ?. (5): operators.mdx:461 cfg?.server?.port ?? 8080. 4) missing methods (5): traits.mdx:172 Array.get/Vec.get "Method not found". 5) intrinsics not migrated (5): stdlib/core/math,rolling,distributions "not migrated to kinded carrier" (math.max/median, rolling.mean/linear_recurrence, dist.uniform). 6) as-cast From/Into/TryFrom auto-derive (4): error-handling:207, traits:249/265 (100.0 as Celsius, 5 as PositiveInt?). 7) match Ok/Err on Result (4): stdlib/core/remote:185 "variant pattern Ok requires an enum-typed value". 8) type-inference gaps (~9). 9) tail (1-2 each): snapshot state.capture no-frame (modules:20/28), DateTime GetProp not kinded (datetime:344/380), var+alias CoW segfault (variables:82, references-borrowing:73), list-comprehension element-type (objects-arrays:135), object-spread dynamic schema (traits:330), associated types (traits:387), polyglot sig must be Result<T> (functions:444).',
].join('\n')

const CTX = [
  'Shape/ code fixes go IN ' + WT + ' (branch wave7/book-truth-campaign, off main HEAD). The BOOK is at ' + BOOK + ' (Astro Starlight; content .mdx under book-site/src/content/docs/; snippet extractor book-site/scripts/extract-shape-snippets.mjs). Build/run shape via: ' + DX + ' <cmd>. The book is a SEPARATE git repo (' + BOOK + '/..) — commit book edits to ITS git, report the hash; do NOT try to merge book edits into the shape/ repo.',
  '',
  ROADMAP,
  '',
  'MANDATE (feedback_book_gate_every_feature): every implemented feature must be in the book + a gate-runnable example that executes GREEN vm+jit. The book must be HONEST — never present broken/pending code as runnable-working. Run examples via the real binary: build ONCE (' + DX + ' cargo build --release --bin shape), run each fence `--mode vm` AND `--mode jit`; set SHAPE_CONFIG_DIR to an empty tempdir to suppress the stale-~/.shape/extensions startup warning (noise only).',
  '',
  'PHASE 1 TRIAGE (read + classify, minimal edits): for EACH of the 64 real defects, read the actual fence body + run it, and classify: (i) DOC-CORRECTABLE — the code is just wrong/retired syntax (e.g. __original__(args), bad as? cast, a renamed API) and a WORKING equivalent exists → will correct the book code; (ii) FEATURE-PENDING-HONEST-MARK — the feature genuinely does not work yet (many carry `SURFACE: ... pending` comments) → keep runnable=false + a clear honest note, do NOT present as working, and cite the tracking lane; (iii) FEATURE-BROKEN-ROUTE — a real bug worth a fix lane → route it (name the cluster + a suggested lane), do NOT fix here. Also confirm which of the 116 runnable=false-but-green are DETERMINISTIC + genuinely standalone (safe to promote) vs nondeterministic/time/random (leave no-run).',
  '',
  'PHASE 2 FIXBOOK + PROMOTE + shape/-FIX: (A) apply the DOC-CORRECTABLE fixes to the book .mdx (correct code to the working form, verified green vm+jit) and the FEATURE-PENDING honest-marks (runnable=false + note). (B) promote the safe deterministic subset of the 116 to runnable=true. (C) In the shape/ worktree, fix the 1 VM/JIT-divergence (datetime.mdx:183) + the VM dev-mode auto-print internal-wrapper leak ({"Integer":..}) — root-fix, no forbidden pattern, no coercion. Keep book edits in ' + BOOK + ' and shape/ edits in ' + WT + '.',
  '',
  'PHASE 3 MEASURE: re-extract + re-run the FULL book-truth (vm+jit) at the campaign state. PROVE: honest book-truth % ROSE (fewer DOC-WRONG); curated gate runnable=true still 100% (no regression — every promoted + corrected fence green vm+jit); the divergence is gone. Report the new numbers vs the 362/426 baseline.',
  '',
  'PHASE 4 FINISH: ' + DX + ' just check-clean + check-no-dynamic EXIT 0 on the shape/ worktree; commit shape/ changes (report for merge); commit the book edits to the ' + BOOK + ' git (report hash); deliver the 18-FEATURE-BROKEN routed roadmap (each: cluster, tractable-now vs deferral, suggested lane).',
  '',
  'CONSTRAINTS: NO forbidden patterns in any shape/ fix. Do NOT edit benchmark files. Do NOT weaken a test/gate to pass. Do NOT delete a fence to raise the %; correct or honest-mark it. no new #[ignore]. STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Triage')
const TRIAGE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['doc_correctable', 'honest_mark', 'feature_route', 'promotable', 'notes'],
  properties: {
    doc_correctable: { type: 'string', description: 'count + which DOC-WRONG fences have a working equivalent to correct' },
    honest_mark: { type: 'string', description: 'count + which are feature-pending → honest-mark runnable=false + note' },
    feature_route: { type: 'string', description: 'the 18 FEATURE-BROKEN triaged: tractable-now vs known-deferral, per cluster' },
    promotable: { type: 'string', description: 'how many of the 116 are deterministic-safe to promote (vs nondeterministic left no-run)' },
    notes: { type: 'string', description: 'any surprises / mis-measured fences' },
  },
}
const triage = await agent(CTX + '\n\nPHASE 1 — TRIAGE (read + classify, run fences; minimal edits). Classify all 64 + the 116. Do NOT commit.',
  { label: 'triage', phase: 'Triage', effort: 'xhigh', schema: TRIAGE_SCHEMA })

phase('FixBook')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'doc_fixed', 'promoted', 'shape_fixes', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    doc_fixed: { type: 'string', description: 'DOC-WRONG corrected + feature-pending honest-marked in the book, brief' },
    promoted: { type: 'string', description: 'how many of the 116 promoted to runnable=true (all green vm+jit)' },
    shape_fixes: { type: 'string', description: 'the divergence + auto-print root-fix in shape/, brief' },
    evidence: { type: 'string', description: 'sampled corrected fences green vm+jit; check-no-dynamic EXIT 0; book edits in shape-web only' },
  },
}
const fix = await agent(CTX + '\n\nTRIAGE: ' + JSON.stringify(triage) + '\n\nPHASE 2 — FIXBOOK + PROMOTE + shape/-FIX. Correct/honest-mark DOC-WRONG, promote the safe 116-subset, fix the divergence + auto-print in shape/. ' + DX + ' just check-no-dynamic EXIT 0. Commit shape/ (git -C ' + WT + ' add -A && commit --no-verify -m "Book-truth campaign shape/ fixes: datetime vm/jit divergence + dev-mode auto-print internal-wrapper leak") and book edits to the shape-web git (git -C ' + BOOK + '/.. add -A && commit -m "Book-truth: correct DOC-WRONG fences + promote hidden-green + honest-mark pending").',
  { label: 'fixbook', phase: 'FixBook', effort: 'xhigh', schema: FIX_SCHEMA })

phase('Measure')
const MEASURE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'honest_pct_before_after', 'gate_still_green', 'divergence_gone', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    honest_pct_before_after: { type: 'string', description: 'honest book-truth % before (362/426=85%) vs after' },
    gate_still_green: { type: 'boolean', description: 'curated runnable=true gate still 100% green vm+jit (no regression); promotions all green' },
    divergence_gone: { type: 'boolean', description: 'datetime vm/jit divergence fixed; no new divergence introduced' },
    evidence: { type: 'string', description: 'your own re-run numbers from scratch; concise' },
  },
}
const measure = await agent(CTX + '\n\nTRIAGE: ' + JSON.stringify(triage) + '\nFIX: ' + JSON.stringify(fix) + '\n\nPHASE 3 — MEASURE. You are an INDEPENDENT reviewer, FRESH context. Re-extract + re-run the FULL book-truth vm+jit from scratch. Confirm: honest % rose, curated gate still 100% (every corrected + promoted fence green vm+jit — spot-run them yourself), divergence gone. Any gate regression (a promoted/corrected fence not actually green), any new divergence, or an inflated % (a fence deleted rather than fixed) = REFUTED.',
  { label: 'measure-verify', phase: 'Measure', effort: 'xhigh', schema: MEASURE_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'shape_commit', 'book_commit', 'feature_roadmap', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'shape/ check-clean + check-no-dynamic EXIT 0, brief' },
    shape_commit: { type: 'string', description: 'the shape/ worktree commit (for merge)' },
    book_commit: { type: 'string', description: 'the shape-web book commit hash' },
    feature_roadmap: { type: 'string', description: 'the 18 FEATURE-BROKEN routed roadmap (cluster -> tractable-now/deferral -> suggested lane)' },
    merge_ready: { type: 'boolean', description: 'the shape/ changes are merge-ready' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nMEASURE: ' + JSON.stringify(measure) + '\n\nPHASE 4 — FINISH (only if Measure CONFIRMED; else merge_ready:false + what remains). ' + DX + ' just check-clean + check-no-dynamic EXIT 0 on the shape/ worktree. Ensure shape/ + book commits exist. Deliver the 18-FEATURE-BROKEN routed roadmap.',
  { label: 'finish', phase: 'Finish', effort: 'high', schema: FINISH_SCHEMA })

return { triage, fix, measure, finish }
