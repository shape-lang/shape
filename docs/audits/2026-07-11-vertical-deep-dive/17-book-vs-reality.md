# Vertical Deep-Dive 17: Book vs Reality (Documentation Truth)

Auditor 17 of 19 — ultra-deep-dive audit, 2026-07-11.
Territory: `shape-web/book/` (Astro Starlight book, 106 md/mdx pages), `shape-web/landing/`,
`shape/README.md`, `shape/examples/`, and the committed book truth-gate infrastructure
(`shape-web/book/book-site/scripts/`, `.github/workflows/book-truth.yml`,
`shape/tools/vmjit-diff/`).

Method: full-universe empirical measurement. Every ```shape fence in the book was
extracted with the project's own extractor (`extract-shape-snippets.mjs`) into a
scratch corpus and **executed** with the working-tree debug binary
(`shape/target/debug/shape`, v0.3.2, built 2026-07-11 10:04) in **both** `--mode vm`
and `--mode jit`, 15 s timeout, isolated `SHAPE_CONFIG_DIR`. 707 fences x 2 modes =
1414 executions. Landing-page and README examples were additionally hand-extracted
and run. All transcripts quoted below are from actual runs on this working tree.

---

## 0. Executive Summary

All numbers below are from actual working-tree runs on 2026-07-11 (707 fences × 2 modes
executed; plus the official gate harness, fixtures, README/landing/examples hand-runs).

### Overall health verdict

**The documentation-truth story has flipped from scandal to credible-but-unfinished.** At the
2026-07-05 measurement the book was a 47%-true corpus hiding behind a 240-fence curated gate.
At today's working tree, the same full-universe measurement yields **570/707 fences green
(80.6%)**, the gate's own denominator has grown to **565 runnable fences — all green** (548
plain + 6 expected-fail + 11 real serve/snapshot-resume fixtures via the official harness), and
the fix campaign is visible in git history and 2,613 uncommitted insertions. What remains is
structural, not volumetric: `runnable=false` still overloads five meanings onto one flag (51 of
142 exclusions hide failures silently); output claims are ungated (11/565 pinned; 3 confirmed
wrong `//`-claims sit gate-green in `stdlib/native/json.mdx`); 83% of "JIT" gate legs actually
run the interpreter via `[jit-fallback]`; and the three highest-traffic surfaces — repo README,
landing page, `shape/examples/` — are the worst-failing artifacts measured (README: both
examples compile-error; landing: 3 of 5 samples fiction/stale; examples/: 5 of 6 broken) and
have zero CI coverage. One P0 fell out of probing the exclusion blind spot: the book's first
DateTime fence prints a **wrong result then segfaults under `--mode jit`** (VM fine), invisible
to the gate because the fence is excluded.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|---|---|---|
| 1 | P0 | JIT wrong-result (`false` for `now.year() >= 2024`) then SIGSEGV (exit 139) on `fundamentals/datetime.mdx:19` fence; also L364, L404 — all hidden behind `runnable=false` | §9.1 transcript |
| 2 | P1 | Both `shape/README.md` code examples fail to compile (stale `snapshot()` Result-shape; stale polyglot return-type rule) | §8.6 transcripts |
| 3 | P1 | Landing page: 3 of 5 "actual syntax" hero samples are fiction/stale (`struct`/`u64`/`@db_schema`/`@host`/`emit`/`Snapshot::None` don't exist) | §8.7 table |
| 4 | P1 | 51 of 142 excluded fences fail silently with zero reader-visible disclosure; incl. whole broken pages (`property_testing` 0/4, `finance`, `iot`, `physics`) and 11 `Not implemented` stub surfaces | §8.4 |
| 5 | P1 | book-truth CI can never trigger on shape/ commits (trigger paths are shape-web-only, contradicting its own header comment); no `schedule:` either | §5.2, `book-truth.yml:7-9` vs `:19-40` |
| 6 | P1 | `shape/examples/`: 5 of 6 files fail (4 parse errors, 1 retired API) | §8.8 |
| 7 | P1 | Gate strength: 83% of "JIT" legs interpreter-fallback (§5.3); 32% of green fences print nothing; security-permissions.mdx's 7 green fences include 4 comment-only blocks while all 8 executable examples are excluded-and-failing — the security chapter has zero machine-verified executable examples | §5.3, §8.10 |
| 8 | P2 | Output truth ungated: only 11/565 fences pin stdout; 3 confirmed wrong output-comments in `stdlib/native/json.mdx` are gate-green | §2.4, §8.5 |
| 9 | P2 | `public/llms-full.txt` LLM export 71 pages stale (pre-campaign content, corrected DOC-WRONG fences included) | §5.4 |
| 10 | P2 | CLAUDE.md itself carries stale language claims (`if x != null` narrowing is a parse error; `fn method(self)` trait syntax rejected in impls) | §8.9 transcripts |

### Scores

- **Feature-completeness: 82/100.** The truth-gate machinery is complete and works end-to-end
  (extraction, vm+jit, expected/expected-fail, 3 fixture types incl. remote snapshot-resume,
  5-slice CI); deductions for the unused `deferred cite=` mechanism, the invisible-to-readers
  runnable marker, ungated output claims, and ungated README/landing/examples surfaces.
- **Code-quality: 78/100.** Gate scripts are small, documented, and self-tested (94/94 unit
  tests green); deductions for the duplicated frontmatter/flag parsers, the 200-byte meta
  window, dead legacy gate infra, and the non-hermetic environment dependency.

### Biggest risk

The denominator trap is structurally still present — it has just been shrunk. The gate reports
"565/565 green" while a reader-facing fifth of the book doesn't run, and nothing in the
rendered site, the CI report, or the flag grammar distinguishes "intentional error example"
from "feature broken since W-series". Because the CI also cannot fire on runtime commits, every
future shape/ regression re-widens the gap invisibly: the datetime JIT segfault (finding #1) is
the live demonstration — a default-mode crash on the first fence of a fundamentals page, with
zero signal anywhere. The 47%→80.6% recovery proves the team can close the gap; the risk is
that without typed exclusion classes, a scheduled cross-repo gate run, and reader-visible
verification badges, the same drift re-accumulates and the next "audit vs claimed" cycle
(2026-07-04's finding: "polyglot/FFI + snapshot/resume + security are DEAD stubs despite book")
repeats with new features.

---

## 1. Architecture & Code Structure Map

### 1.1 What "the documentation surface" physically is

The documentation truth surface spans two repos plus the main workspace:

| Component | Path | Size | Role |
|---|---|---|---|
| Book content | `shape-web/book/book-site/src/content/docs/` | **102 mdx pages, ~22,690 LOC** | The canonical user-facing book (Astro Starlight) |
| — getting-started | `.../getting-started/` | 6 pages, 855 LOC | install, REPL, first program |
| — fundamentals | `.../fundamentals/` | 20 pages, 6,200 LOC | core language chapters |
| — stdlib | `.../stdlib/` | 44 pages, 6,139 LOC | per-module API reference (core/native/math/domain) |
| — advanced | `.../advanced/` | 15 pages, 6,661 LOC | comptime, JIT, security, resumability, wire, polyglot-distributed |
| — tooling | `.../tooling/` | 10 pages, 2,288 LOC | LSP, MCP, extensions, projects, polyglot |
| — appendix / examples / index | | 3+3+1 pages, 549 LOC | FAQ, troubleshooting, config, 3 examples |
| Truth-gate extractor | `book-site/scripts/extract-shape-snippets.mjs` | 366 LOC | walks every `.mdx`, emits one `.shape` file per ```` ```shape ```` fence + `manifest.json` |
| Truth-gate harness | `book-site/scripts/run-book-truth-gate.mjs` | 685 LOC | runs each runnable fence `--mode vm` + `--mode jit`, gates on byte-exact stdout equality; supports `expected=`, `expected-fail=`, and 3 fixture types (serve / serve-snapshot-resume / local-snapshot-resume) |
| Gate unit tests | `extract-shape-snippets.test.mjs` (287), `run-book-truth-gate.test.mjs` (177), `serve-fixture.test.mjs` (210) | 674 LOC | harness self-tests |
| Fence-label plugin | `book-site/src/lib/remark-shape-snippet-label.ts` | ~50 LOC | parses `runnable=true|false` from fence info string; **default = runnable** |
| Manifest contract | `book-site/scripts/MANIFEST_SCHEMA.md` | — | schema doc for `manifest.json` |
| CI wiring | `shape-web/.github/workflows/book-truth.yml` | 190 LOC | builds release binary, extracts, runs gate in 5 parallel slice jobs (A–E), aggregates |
| LLM export | `book-site/scripts/generate-llms-full.js` (126 LOC) → `public/llms-full.txt` (13,260 lines) | | flattened book for LLM consumption |
| Landing page | `shape-web/landing/index.html` | 492 LOC | marketing claims + 5 rotating "actual syntax from the book" examples |
| Repo README | `shape/README.md` | 112 LOC | headline positioning + 2 full code examples |
| Repo examples | `shape/examples/*.shape` | 6 files | ad-hoc example programs |
| **Legacy (stale) gate** | `shape-web/book/snippets/` (11 `.shape` + 11 `.expected`) + `book/test-snippets.sh` | | pre-Starlight mdBook-era snippet checker — **broken at HEAD, 0/11 pass** (§3.4) |
| Legacy docs | `book/README.md` (67), `book/HOW_TO_BUILD.md` (81), `book/REWRITE_PLAN.md` (94) | | README is entirely stale mdBook-era (§8.6) |

### 1.2 Data flow of the truth gate

```
.mdx pages --(remark-parse + remark-shape-snippet-label)--> mdast code nodes
   |  fence info-string: runnable=true|false (DEFAULT true), expected="...",
   |  expected-fail="...", fixture=serve|serve-snapshot-resume|local-snapshot-resume,
   |  runnable=deferred cite=<id>, serve-sandbox=none|strict|permissive
   v
extract-shape-snippets.mjs --> .book-truth-gate/snippets/<slice>__<page>__<pos>__L<line>.shape
   |                       \-> manifest.json (707 records at working-tree HEAD)
   v
run-book-truth-gate.mjs: for each runnable snippet:
   shape run --mode vm <f>   AND   shape run --mode jit <f>
   gate = both exit 0 AND vm-stdout == jit-stdout (byte-exact)
        (== expected  if `expected=` set — only 11 fences;
         both-must-fail-with-substring if `expected-fail=` — only 6 fences)
   v
report.json {total, pass, fail, categories per §8.7 taxonomy}
```

Slice attribution (`sliceFor()`, `extract-shape-snippets.mjs:131-167`) partitions pages A–E for CI parallelism: A=fundamentals/appendix (263 fences), B=stdlib+datetime/tables/content (294), C=getting-started/examples/index (26), D=comptime/annotations/polyglot (66), E=other advanced/tooling (58).

### 1.3 Key types / contracts

- **Manifest record** (`MANIFEST_SCHEMA.md`): `{id, slice, page, pageSlug, position, line, runnable, deferred?, cite?, expected?, expectedFail?, fixture?, serveSandbox?, path}`.
- **Failure taxonomy** (`run-book-truth-gate.mjs:18-29`): `vm-only-fail`, `jit-only-fail`, `both-fail`, `output-divergence`, `expected-divergence`, `expected-fail-succeeded`, `expected-fail-missing`, `runtime-timeout`, `pass`.
- **HTML marker**: the remark plugin stamps `data-shape-runnable="true|false"` on each rendered `<pre>` (`remark-shape-snippet-label.ts:43`) — but **no CSS or component consumes it** (§5.3), so the distinction is invisible to readers.

### 1.4 Entry points

- CI: `book-truth.yml` — trigger paths are **shape-web-side only** (mdx content + gate scripts + astro config; lines 21-40). Despite the header comment claiming it triggers "on shape/ commits that touch stdlib / runtime / grammar surfaces the book depends on" (lines 7-9), **no shape/ path can trigger it** — the two repos are separate and the workflow only checks out `shape-lang/shape@main` as a dependency. A runtime regression in shape/ silently breaks the book until the next book edit (§5.2, finding B-9).
- Local: `node scripts/extract-shape-snippets.mjs && node scripts/run-book-truth-gate.mjs --shape-bin <bin>`.
- The measurement in this audit used exactly these entry points against the working-tree debug binary.

---

## 2. Feature Completeness (documentation-truth surface)

Scored per component. "WORKS END-TO-END" means demonstrated by execution in this audit, not
inferred from code reading.

### 2.1 The truth-gate pipeline — IMPLEMENTED, works end-to-end

Every stage was exercised against the working tree on 2026-07-11:

| Stage | Status | Evidence |
|---|---|---|
| Fence extraction (`extract-shape-snippets.mjs`) | **WORKS** | Ran against working-tree book: `Wrote 707 snippets ... runnable=true: 565, runnable=false: 142, deferred: 0, expected=set: 11, expected-fail: 6, fixture=set: 11, by slice: {"D":66,"E":58,"A":263,"C":26,"B":294}` |
| Per-snippet vm+jit harness (`run-book-truth-gate.mjs`) | **WORKS** | Full official gate run launched this session (`SHAPE_BIN=target/debug/shape`); 325/325 green at the last checkpoint observed before audit close (zero failures; serial run still executing); the independent full matrix over the same 707 snippets × 2 modes (identical stdout-equality criterion) completed and confirms 565/565 runnable green (§8.2) |
| `expected=` strict-output gate | **WORKS**, but only 11/565 fences use it | Manifest list §2.4; all 11 are remote/snapshot fixtures |
| `expected-fail=` diagnostic gate | **WORKS** | 6 fences, all `green-expected-fail` in my full run (e.g. `fundamentals/references-borrowing.mdx:30` gates on `[B0005] cannot use this value after it was moved`) |
| `fixture=serve` / `serve-snapshot-resume` / `local-snapshot-resume` | **WORKS END-TO-END** | Official harness run on the 11 fixture fences: `fixture-report.json` → `"total": 11, "pass": 11, "fail": 0`, including a real `shape serve` receiver, remote snapshot hash capture, and `--resume <hash>` re-execution |
| `runnable=deferred cite=<id>` | **CODE EXISTS, UNUSED** | Extractor parses it (`extract-shape-snippets.mjs:212-217`); manifest has `deferred: 0`; the promised cite-id validation is explicitly "deferred to skip if cite-id unknown" (`extract-shape-snippets.mjs:29`) — i.e. the validation arm is itself a stub |
| CI wiring (`book-truth.yml`) | **PARTIAL** | 5-slice parallel jobs exist and run harness unit tests first; but trigger paths cannot fire on shape/ commits despite the header comment claiming they do (§5.2) |
| `data-shape-runnable` HTML marker | **STUBBED at the UI layer** | Plugin stamps it (`remark-shape-snippet-label.ts:43-46`); zero CSS/component consumers in `book-site/src/` (grep over `*.css`, `*.astro`, `*.svelte`, `*.ts` excluding the plugin itself returns nothing) — readers cannot distinguish verified from unverified fences |
| LLM export (`generate-llms-full.js`) | **WORKS but STALE artifact** | `public/llms-full.txt` dated Jul 3 06:36; 71 of 102 content pages are newer than it |

### 2.2 Legacy documentation infrastructure — DEAD / MISLEADING

- `book/test-snippets.sh` + `book/snippets/` (11 `.shape`/`.expected` pairs): **0/11 pass** at
  HEAD. Root cause is the script's `actual=$("$SHAPE_BIN" "$shape_file" 2>&1)` capturing the
  extension-loader stderr banner (`Loaded module: python v0.1.0 ...`) into the comparison. On a
  machine without stale `~/.shape/extensions` it may pass, i.e. the legacy gate is
  **environment-dependent**, which is exactly what the new harness fixed by comparing stdout
  only (`run-book-truth-gate.mjs:112-146` keeps the streams separate).
- `book/README.md` (67 lines): entirely mdBook-era — instructs `mdbook serve -p 9090` against a
  `shape-server` backend on 9091, from a directory (`shape/docs/book`) that does not exist. The
  real build doc is `book/HOW_TO_BUILD.md` (Astro Starlight, correct).
- `book/REWRITE_PLAN.md`: describes the completed rewrite as future work; historical, unmarked.

### 2.3 Book feature coverage of the language — BROAD, minor gaps

Sweep of CLAUDE.md's language-feature list against book content
(`grep -rl` over `src/content/docs/**/*.mdx`):

| Feature | Book pages mentioning | Status |
|---|---|---|
| `join all` / `for await` / `async scope` | 3 / 6 / 5 | covered (fundamentals/async.mdx) |
| pipe `\|>` | 1 (operators.mdx:349) | covered; **verified working** (`21 \|> double` → `42`) |
| `??` / `?.` | 2 | covered; **verified working** (`v ?? 7` → `7`; `cfg?.server?.port ?? 8080` fence green) |
| comptime / comptime for | 5 | covered (advanced/comptime.mdx + cookbook) |
| `extern C` | 8 | covered (advanced/native-c-interop.mdx) |
| traits / enums / match | 14 / 16 / 34 | covered |
| snapshot()/resume | 5 | covered, with **real resume fixtures** (11/11 green) |
| `decimal` / `bigint` | 4 / 2 | thin but present |
| annotations `@annotation` | 2 | covered (advanced/annotations.mdx, 12 fences) |
| Drop / RAII | 3 | covered (fundamentals/resource-management.mdx, 17 fences, 15 runnable-green) |
| `wire-serve` CLI | 1 | mentioned only in advanced/wire-protocol.mdx (whose single fence fails) |

Notable coverage gap: there is no dedicated page for the `out`-param `extern C` stub generation
(CLAUDE.md language feature list) — `native-c-interop.mdx` covers `out` params inline only.

### 2.4 The `expected=` gap — output claims are essentially ungated

Only 11 of 565 runnable fences pin their stdout, and all 11 are the remote/snapshot fixture
fences (`advanced/annotations.mdx:480`, `advanced/polyglot-distributed.mdx:74,213`,
`advanced/resumability.mdx:21,100`, `stdlib/core/remote.mdx:41,77,142,166,187`,
`tooling/execution-server.mdx:130`). For the other **554 runnable fences the gate checks only
"exit 0 + VM stdout == JIT stdout"** — a fence whose inline `// comment` or adjacent prose
claims the wrong output still passes. §8.5 shows three concrete wrong-output claims in
`stdlib/native/json.mdx` that are gate-green today.

### 2.5 Verdict

The documentation-truth *machinery* is genuinely feature-complete and working (extraction,
vm+jit gating, fixtures including remote-snapshot-resume, CI slicing). The *coverage* of that
machinery has two holes: output truth (11/565 pinned) and the 142-fence exclusion set, of which
51 exclusions silently hide failing examples with no reader-visible disclosure (§8.4).

---

## 3. Code Quality (gate infrastructure + doc tooling)

### 3.1 Overall

The truth-gate scripts are the best-engineered part of this territory: small, documented,
self-tested (94 unit tests across 3 files, all passing at working tree — §7.1), with an explicit
schema contract (`MANIFEST_SCHEMA.md`) and a failure-mode taxonomy
(`run-book-truth-gate.mjs:16-29`). Naming is consistent (`runOne`, `runSnippetPair`,
`runWithServeFixture`), and edge cases carry explanatory comments with history (e.g. the
frontmatter line-offset bug postmortem at `extract-shape-snippets.mjs:90-96`, the
timeout-vs-external-SIGKILL disambiguation at `run-book-truth-gate.mjs:137-140`).

### 3.2 Specific defects and smells

1. **200-byte raw-fence window** (`extract-shape-snippets.mjs:280-285`): to recover the original
   fence meta (the remark plugin strips `runnable=`), the extractor re-slices the raw body
   `start.offset .. start.offset+200` and takes the first line. Any fence info-string longer
   than ~200 chars is silently truncated — an `expected="..."` string near/over that length
   would lose its closing quote and the flag would silently not parse. Today's longest
   `expected=` (stdlib/core/remote.mdx:77, 4-line escaped string) is ~130 chars — within one
   edit of the cliff. No test covers the >200 case.
2. **Flag grammar parsed twice with different regexes**: `runnable=` in
   `remark-shape-snippet-label.ts:30` (`RUNNABLE_RE`), everything else in
   `parseExtensionFlags()` (`extract-shape-snippets.mjs:209-239`). The plugin strips its token
   before the extractor sees `node.meta`, which is why the extractor needs the raw-fence hack in
   (1). One parser owning the whole info-string grammar would remove both the duplication and
   the 200-byte window.
3. **`stripFrontmatter` duplicated** (§4.1) with a known-bug fix applied to only one copy.
4. **Error-classification by regex on combined output** in the harness's report bucketing is
   robust enough for CI but treats any nonzero exit as one bucket per mode; there is no
   distinction between compile error and runtime panic in `report.json` buckets
   (`both-fail` covers `error[E0001]` and SIGSEGV alike). A JIT segfault (§9.1) and a typo'd
   fence land in the same bucket.
5. **`sliceFor()` heuristic is hardcoded page-name lists** (`extract-shape-snippets.mjs:131-167`)
   — e.g. `fundamentals/{datetime,tables,content}` → B. Adding a page silently defaults to a
   slice; nothing checks the lists against the actual directory contents. Low risk (slices only
   partition CI), but it will drift.
6. **Extension-loader stderr noise** is tolerated rather than fixed: the harness survives it by
   comparing stdout, but the legacy `test-snippets.sh` and any naive user following
   `book/README.md` sees 3 banner lines on every run. The CLI prints `Loaded module: ...` +
   `Shape engine initialized (2 extension modules loaded)` to stderr even for programs that
   never touch polyglot (verified: `print`-only program, transcript in §2.2). This is a
   UX/doc-truth tax on every transcript shown in the book — none of the book's shown outputs
   include these lines.

### 3.3 Longest files / complexity

- `run-book-truth-gate.mjs` (685 LOC) is the largest script; its complexity is concentrated in
  the three fixture flows (`runWithServeFixture:296`, `runWithServeSnapshotResumeFixture:392`,
  `runWithLocalSnapshotResumeFixture:447`), each of which manages child-process lifecycle,
  readiness polling, and hash capture. These are the right places for the complexity and each
  has a dedicated unit-test file.
- No `unsafe` (JS territory); no dead exports found in the scripts; `serve-fixture.test.mjs`
  exercises the trickiest path (readiness timeout, stderr tail capture).

### 3.4 Book content quality (prose level)

- Honest-marking discipline exists and is real: e.g. `stdlib/core/testing.mdx:85-88` explicitly
  says "Keep this example disabled until Result method dispatch is available" above a
  `runnable=false` fence; `fundamentals/traits.mdx:98` names the deleted `ValueWord` dependency
  for the `WrapTypeAnnotation` caveat; `advanced/jit-compilation.mdx:422` marks its only
  latency number "illustrative; awaiting v0.4 benchmark anchor".
- But the discipline is inconsistently applied: 51 of 142 excluded fences have **no** disclosure,
  error-framing, or fragment-marking within 15 lines (§8.4), and the book UI renders excluded
  fences identically to verified ones (§2.1 marker finding).

---

## 4. Duplication & DRY Violations

### 4.1 `stripFrontmatter` — two copies, one bugfix

- Copy A: `book-site/scripts/generate-llms-full.js:22`
- Copy B: `book-site/scripts/extract-shape-snippets.mjs:85-100`, whose comment says "mirror
  generate-llms-full.js" and then documents a bug the mirror still has: Copy A `trimStart()`s
  the body, which collapses blank lines after the closing `---`. That shifted reported line
  numbers in the extractor (fixed in Copy B, `extract-shape-snippets.mjs:92-96`); Copy A doesn't
  need line numbers so the divergence is currently benign — but anyone "unifying" them by
  picking Copy A re-introduces the extractor bug. Danger: **medium** (documented trap).

### 4.2 Fence info-string grammar — two parsers (see §3.2.2)

`RUNNABLE_RE` in the remark plugin vs `parseExtensionFlags` + raw-fence recovery in the
extractor. The two must agree on tokenization (both anchor on `(?:^|\s)...(?=\s|$)`), and the
extractor's comment at `extract-shape-snippets.mjs:196-201` admits the coupling. A new flag
added to one side only would silently mis-parse. Danger: **medium**.

### 4.3 Two generations of snippet gate

`book/snippets/*.{shape,expected}` + `test-snippets.sh` (mdBook era, 11 snippets, 0/11 pass at
HEAD under a machine with installed extensions) vs `book-site/scripts/` (Starlight era, 707
snippets). The legacy one is not wired into CI but is still committed, still documented by
`book/README.md` as *the* test procedure, and fails in a way that looks like a product bug.
Danger: **low** (confusion, wasted debugging), fix is deletion.

### 4.4 README / landing / book triple-maintaining the same examples

`shape/README.md` "What Shape Looks Like" and "Polyglot Example", the landing page's 5 rotating
`<code>` samples (`landing/index.html:214-247`), and book pages each hand-maintain overlapping
example code. They have already diverged in opposite directions:

- README `match snapshot() { Snapshot::Hash(id) => ...` — **stale**: `snapshot()` now returns
  `Result<Snapshot, SnapshotError>`; compile error (transcript §8.6).
- README `fn python std_dev(...) -> number` — **stale**: polyglot returns must be
  `Result<number>` per the ratified Q13 design; compile error (§8.6).
- Landing block 4 (`fn python ... -> Result<number>`) — **correct** (verified green, §8.7): the
  landing was updated for Q13 but the README wasn't.
- Landing block 7 (`match snapshot() { Snapshot::Hash ... Snapshot::None ... }`) — **doubly
  stale**: pre-Result shape *and* a `Snapshot::None` variant that doesn't exist
  (`stdlib-src/core/snapshot.shape:5-10` defines only `Hash(string)` and `Resumed`).

Danger: **high** — these are the three most-viewed surfaces in the project, and no gate covers
any of them (the truth gate walks only `book-site/src/content/docs/`).

### 4.5 `shape/examples/` — orphaned fourth copy of "showcase" code

6 ad-hoc files, 5 of which fail (§8.8), overlapping in intent with book examples. Not gated,
not indexed, referenced by nothing. Danger: **low-medium** (a user who finds
`complete-language-example.shape` gets a parse error on line 5).

---

## 5. Split-Brain Analysis

### 5.1 The denominator split-brain: gate-green vs book-truth (THE story of this vertical)

Two different truths about the same corpus can be reported honestly:

- **"Gate is green"**: 565/565 runnable fences pass vm+jit (my full re-run over the extracted
  corpus; the official harness corroborates — 325/325 green at the last checkpoint observed,
  zero failures; fixtures 11/11 via the official harness).
- **"The book runs"**: 570/707 = **80.6%** of all fences execute green (both modes, stdout
  equal). 137 fences (19.4%) fail; they are simply excluded from the gate's denominator via
  `runnable=false`.

This is a *designed* split-brain: `runnable=false` exists precisely so intentional-error examples
and fragments can be documented. The failure mode is that the same flag also absorbs
broken-features (§8.4), and nothing in the rendered book distinguishes the two. Drift evidence:
on 2026-07-05 the same split was 240/240 green vs ~350/738 = **47%** real truth
(`docs/cluster-audits/fix-plan-2026-07-05-workflows.md:215-218`). The working tree shows a
massive honest-marking + fix campaign since (git log: `7ae8b9e` "Book-truth: correct DOC-WRONG
fences + promote hidden-green + honest-mark pending", `2238b75`, `9ad8013`, `c9531e8`,
`d4cb494`, `702399d`; plus 2,613 insertions across 59 content files uncommitted in the working
tree). The gap closed from 53 points to 19.4 points — real progress, same structure.

### 5.2 CI trigger split-brain: comment says two-repo, config is one-repo

`book-truth.yml:7-9` claims "Triggers on shape-web/book changes AND on shape/ commits that touch
stdlib / runtime / grammar surfaces the book depends on". The actual `on:` block
(`book-truth.yml:19-40`) lists only shape-web paths; the shape repo is merely checked out as a
dependency (`repository: shape-lang/shape`, `book-truth.yml:52-54`). Since the two repos are
separate, **no shape/ commit can ever fire this workflow**. A runtime regression lands silently;
the book stays "green" (last successful run's badge) until someone edits an .mdx. Drift risk:
**high and already realized** — the 3 datetime JIT segfaults (§9.1) are exactly the class this
gate would have caught if it ran on shape/ commits (they're excluded fences, so it also needs
§8.4's fix — but the trigger gap guarantees even runnable regressions surface late).

### 5.3 VM-vs-JIT split-brain, as measured through the book corpus

The gate's acceptance criterion is "every labeled-runnable snippet runs cleanly VM == JIT"
(`book-truth.yml:4-5`). Measured reality: in **467 of 565** runnable fences the JIT leg printed
`[jit-fallback] function main failed JIT compile: ... top-level code has no MIR data; running
under interpreter` — i.e. **83% of the "JIT" legs execute the interpreter**, and VM==JIT is
trivially true for them. The book documents the fallback honestly
(`advanced/jit-compilation.mdx:214-260`), but the gate's marketing sentence overstates what is
verified. Where the JIT actually engages, it can diverge for real: `fundamentals/datetime.mdx`
fences at L19/L364/L404 pass VM and **segfault (exit 139) under `--mode jit`**, with a wrong
boolean printed before the crash (§9.1) — all three currently hidden behind `runnable=false`.

### 5.4 Doc-vs-doc split-brains

| Surface A | Surface B | Divergence |
|---|---|---|
| `shape/README.md` snapshot/polyglot examples | working binary | both compile-error (§8.6) |
| `landing/index.html` block 5 (`@db_schema`, `struct`, `u64`, `const uri`) | grammar | `struct`/`u64` don't exist in Shape; fence unparseable (§8.7) — pure fiction on the landing page |
| `landing/index.html` block 6 (`await @host("eu-west") fetch_order(id)`, `emit(...)`) | stdlib | no `@host` annotation exists anywhere in `stdlib-src/` (grep empty); `emit` undefined |
| `book/README.md` (mdBook, ports 9090/9091) | `book/HOW_TO_BUILD.md` (Astro) | legacy copy contradicts current copy in the same directory |
| CLAUDE.md "Flow-sensitive narrowing: `if x != null` narrows `T?`" | parser | `x != null` is a parse error at HEAD (`unexpected }`, transcript §8.9); Shape uses `Option<T>`/`None` and the book correctly documents `??`/`?.` over Options — CLAUDE.md carries the stale null-model |
| CLAUDE.md repo map "docs/ = Marketing materials (pitch deck, one-pager)" | reality | `shape/docs/` is 100+ engineering docs (ADRs, cluster-audits, specs); the marketing docs live in the *sibling* `~/dev/shape-lang/docs/` — the table describes the parent-dir layout, misleading in-repo readers |
| `public/llms-full.txt` (Jul 3) | book content (Jul 5-11 campaign) | 71/102 pages newer than the LLM export; an LLM consuming it reads the pre-campaign book, including fences since corrected as DOC-WRONG |

### 5.5 Environment split-brain: the gate is not hermetic

The harness spawns `shape run` with ambient `process.env` (`run-book-truth-gate.mjs:110`).
Result: fence outcomes depend on `~/.shape/extensions`. Measured: 8 python/typescript fences in
`tooling/{python,typescript}-extension.mdx` **pass** on this machine (extensions installed) but
**fail** under an isolated `SHAPE_CONFIG_DIR` (my run_one2 isolation) — and in CI they'd fail
too, which is presumably why they're all `runnable=false` even though the features work locally.
The exclusion thus encodes "CI has no extensions", not "the docs are wrong" — one more meaning
silently overloaded onto `runnable=false`.

---

## 6. ADR & Spec Conformance

The book is documentation, not runtime code, so ADR conformance here means: (a) does the book
describe the ADR-governed model *truthfully*; (b) does the doc tooling respect the Forbidden
Patterns vocabulary discipline; (c) are ADR-mandated observable behaviors documented.

### 6.1 ADR-006 value & memory model — book description CONFORMS

- **Bindings & storage classes** (`ADR-006 §2.7` lattice): `advanced/ownership-deep-dive.mdx:26-55`
  reproduces the model exactly — `let`/`let mut` unique with `Direct`/`UniqueHeap`, `var`
  smart-default across `Direct / UniqueHeap / SharedCow / SharedAtomic / SharedAtomicMut`,
  refcount-on-escape ("refcounting is reached only when escape ... genuinely requires it",
  line 30). This matches ADR-006 §2.7 and the `BindingStorageClass` lattice
  (`crates/shape-vm/src/type_tracking.rs:286`). CONFORMS.
- **RAII / Drop** (`fundamentals/resource-management.mdx`): documents scope-based drop, reverse
  drop order, `method drop()` implicit-receiver signature. 15 of its 17 fences run green.
  CONFORMS on what it covers.
- **ADR-006 §2.7.30 escaping-Drop deferral — REAL but UNDOCUMENTED in the book.** Verified
  empirically: a returned `impl Drop` value's drop runs at module end, not at function scope
  exit — transcript: `end of make body` → `after call, before module end` → `drop escapee`
  (program in scratch `dropesc2.shape`). `resource-management.mdx` never mentions the deferral
  (no hit for module/escape/deferred in the page). CLAUDE.md documents it; the user-facing book
  does not. **GAP** — this is an observable Drop-ordering semantics that users will trip on.
- **LSDS as primary diagnostic format** (ADR-006): the string "LSDS" appears **nowhere** in
  `book-site/src/` — the appendix (configuration/faq/troubleshooting) doesn't describe the
  diagnostic format at all. **GAP**.

### 6.2 Forbidden Patterns vocabulary (CLAUDE.md §Forbidden) — CONFORMS

- Grep over all 102 pages for `ValueWord|dynamic fallback|nan.?box|tag bits` returns exactly two
  hits, both *deletion-fate* references, which is the mandated way to talk about them:
  `fundamentals/traits.mdx:98` ("`WrapTypeAnnotation` depends on the deleted `ValueWord`
  wrapper; tracked ...") and `advanced/jit-compilation.mdx:244` (`RETURN_TAG_NANBOXED`
  surface-and-stop, named as a surfaced error). No live forbidden mechanism is documented as a
  feature; no rename-family vocabulary ("decode bridge", "boundary shim", etc.) appears in the
  book (grep returned zero for the broader-family regex terms). CONFORMS.
- The failing excluded fences that hit `Not implemented: ... SURFACE` stubs (11 fences, §8.4)
  demonstrate the surface-and-stop discipline holding at runtime rather than a silent fallback —
  from the doc-truth perspective these produce honest errors, though the *pages* often don't
  disclose them (that's finding B-3, a disclosure gap, not a forbidden-pattern violation).

### 6.3 ADR-005 single-discriminator — N/A for scripts, book text consistent

Book pages describing the heap model (`ownership-deep-dive.mdx`, `developer-tools.mdx:147`
citing `Vec<u64>` + parallel kinds per §2.7.7) match the ADR text. One oddity worth flagging:
**reader-facing pages cite internal ADR sections and cluster-audit files by path** (e.g.
`objects-arrays.mdx:362` "ckpt-1..ckpt-4 per ADR-006 §2.7.24 Q25.A SUPERSEDED",
`examples/comptime-codegen.mdx:15` "per W12 audit §3.5"). Truthful, but these citations point at
documents that don't ship with the book — internal engineering jargon leaking into user docs
(§11.4).

### 6.4 W15 book-truth spec (`docs/cluster-audits/v0.3-w15-book-truth-reaudit.md` §8) — the gate implements it

Checked contract-by-contract against the implementation: snippet-ID convention §8.2
(`snippetIdFor`, `extract-shape-snippets.mjs:176-178` — matches), vm+jit byte-exact gate §8.1
(`run-book-truth-gate.mjs` pass condition — matches), failure taxonomy §8.7 (report buckets —
matches), 5-slice CI partition §8.3 (`book-truth.yml` matrix — matches), runtime budget §8.5
(15 min job timeout — plausible). The one *spec-level* nonconformance is the acceptance
sentence "every labeled-runnable snippet runs cleanly VM == JIT" — measured: 83% of JIT legs
interpreter-fallback (§5.3), so the criterion verifies less than its words claim.

---

## 7. Test Coverage In-Territory

### 7.1 Gate-infra unit tests — GOOD

Run at working tree this session:

```
extract-shape-snippets.test.mjs  → 53 passed, 0 failed
run-book-truth-gate.test.mjs     → 20 passed, 0 failed
serve-fixture.test.mjs           → 21 passed, 0 failed
```

Assertion quality is high — e.g. the extractor tests cover frontmatter line-offset decoding
(the historical bug), flag-grammar word-boundary composition, escape decoding in `expected=`;
the harness tests cover the timeout-vs-external-SIGKILL distinction (`timedOutFromClose`) and
manifest persistence; the serve-fixture tests cover readiness timeout and stderr-tail capture.
CI runs the first two before every gate run (`book-truth.yml:86-89`).

### 7.2 Corpus-level coverage — measured this audit

- 707/707 fences executed in both modes (this audit's full matrix; the official harness
  corroborates the runnable subset — 325/325 green at the last checkpoint observed, zero
  failures, serial run still executing at audit close).
- 11/11 fixture fences pass through the *official* harness including snapshot-resume round-trips.
- 6/6 expected-fail fences produce the pinned diagnostics.

### 7.3 Gaps

1. **No test for the 200-byte meta window** (§3.2.1) — a long `expected=` string would silently
   drop its gate.
2. **No hidden-green detector**: nothing flags a `runnable=false` fence that actually passes.
   Measured today: 16 excluded fences pass VM on a machine with extensions; 5 pass strict
   vm+jit+stdout-equal even in an isolated config
   (`fundamentals/functions.mdx:413`, `fundamentals/traits.mdx:387`,
   `stdlib/core/monte_carlo.mdx:82`, `tooling/python-extension.mdx:163`,
   `tooling/typescript-extension.mdx:180`). These should be promoted; only a manual campaign
   (`7ae8b9e` "promote hidden-green") does this today.
3. **No output-truth coverage**: 554/565 runnable fences unpinned (§2.4); the three
   `stdlib/native/json.mdx` wrong-output comments (§8.5) are invisible to every existing test.
4. **README / landing / `shape/examples/` have zero coverage** — and are the surfaces that fail
   worst (§8.6-§8.8).
5. **`generate-llms-full.js` has no staleness check** — nothing fails when `llms-full.txt` is 71
   pages behind the corpus.
6. **The legacy `test-snippets.sh` suite** is dead weight: 0/11 pass locally; not in CI; superseded.

### 7.4 Adjacent harness: `shape/tools/vmjit-diff/` — good design, stale corpus

The WF-0B differential harness re-uses the book's runnable fences as corpus tier 1 and diffs
vm-vs-jit stdout+exit per program, with a `known-red.json` allowlist whose entries require an
audit citation and whose header forbids it becoming "a dumping ground that greens a red gate" —
exemplary discipline (two pinned real divergences, each with a multi-paragraph forensic
rationale, incl. the reclassified HOF-return-kind raw-bits printing and a proven-pre-existing
nondeterministic comptime FrameDescriptor flake). But: its committed corpus was generated
2026-07-05 from the **240-fence era** ("Corpus: 467 programs (240 book runnable fences, 224
acceptance, 3 synthetic)", `baseline-2026-07-05.txt:6`), and the latest report predates the
book campaign (`reports/report.md`, generated 2026-07-06). The 325 fences promoted since are
not in it, and — like the book gate — it only ingests `runnable=true` fences, so the datetime
JIT segfaults (§9.1) are invisible to it too. Regenerating the corpus is one command
(`build-corpus.mjs`) and would more than double its book tier.

---

## 8. Book/Docs vs Reality — the full-universe measurement

### 8.1 Method

1. Extracted **all 707** ```` ```shape ```` fences with the project's own extractor into a
   scratch corpus (`--out` redirected; ids like `B__stdlib__native__json__3__L77.shape` carry
   page + fence-position + line provenance).
2. Executed every fence under `shape run --mode vm` **and** `--mode jit` (working-tree debug
   binary, 15 s timeout), capturing stdout/stderr/exit separately per mode
   (2 × 707 = 1414 runs).
3. Classification: `green` = both modes exit 0 **and** vm-stdout == jit-stdout byte-exact
   (stderr excluded — extension-loader noise and `[jit-fallback]` warnings live there);
   `expected=`/`expected-fail=` respected where present; the 11 fixture fences scored via the
   *official* harness (11/11 pass, `fixture-report.json`).
4. In parallel, the **official** `run-book-truth-gate.mjs` was run end-to-end on the working
   tree as confirmation of the gate's own verdict on the 565 runnable fences.
5. The 142 excluded fences were additionally run under ambient env (extensions available) to
   separate hermeticity effects from genuine failures.

### 8.2 Headline numbers (working tree, 2026-07-11)

| Metric | 2026-07-05 (fix-plan doc) | **Today (measured)** |
|---|---|---|
| Total fences | 738 | **707** |
| `runnable=true` (the gate's denominator) | 240 | **565** |
| `runnable=false` (excluded) | 498 | **142** |
| Gate verdict on runnable | 240/240 green | **565/565 green** (548 plain green + 6 expected-fail + 11 fixtures) |
| Excluded fences that actually FAIL | 388 | **137** (126 fail even with extensions available) |
| **Full-universe truth** | ~350/738 = **47%** | **570/707 = 80.6%** |
| Runnable JIT legs that actually ran JIT code | not measured | **98/565 (17%)** — 467 fell back to interpreter (§5.3) |

The denominator trap has been **substantially but not fully remediated**: the curated subset
more than doubled, the excluded set shrank 3.5×, and the truth rate rose 33 points. What
remains is a 142-fence blind spot in which 51 failures are silent (§8.4) and a gate criterion
that only pins output for 11 fences (§2.4).

### 8.3 Per-chapter truth rates (green = both modes clean, all fences counted)

| Chapter | Green/Total | Rate |
|---|---|---|
| getting-started | 22/22 | **100%** |
| index.mdx | 1/1 | 100% |
| fundamentals | 249/287 | **87%** |
| stdlib | 214/251 | **85%** |
| advanced | 64/110 | **58%** |
| tooling | 19/33 | **58%** |
| examples | 1/3 | 33% |

Worst pages (0% green): `advanced/developer-tools.mdx` (0/5 — TimeTravel/patches APIs that
don't exist), `advanced/module-distribution.mdx` (0/1), `advanced/wire-protocol.mdx` (0/1),
`examples/comptime-codegen.mdx` (0/1), `examples/web-request.mdx` (0/1),
`stdlib/core/property_testing.mdx` (0/4 — the module itself fails `[E0900] post-inference
FieldType::Any in user-facing schema std::core::utils::property_testing`),
`stdlib/domain/finance.mdx` (0/1 — stdlib signature error: "Required parameter cannot follow a
parameter with a default value"), `stdlib/domain/iot.mdx` (0/2 — undefined `now`),
`stdlib/domain/physics.mdx` (0/2 — inference failure `unknown * number`),
`tooling/extensions.mdx` (0/1). Next tier: `stdlib/core/stochastic.mdx` 1/5 (4 fences hit
`Not implemented: ... IntrinsicBrownianMotion/Gbm/OuProcess/RandomWalk body migration to kinded
carriers` — the page documents a stochastic-process API whose intrinsics are stubs),
`stdlib/domain/simulation.mdx` 1/4, `advanced/content-addressed-bytecode.mdx` 7/16,
`advanced/security-permissions.mdx` 7/15, `fundamentals/tables.mdx` 4/8.

### 8.4 Anatomy of the 142-fence exclusion set

Every excluded fence executed + classified against its page context (15 lines of preceding
prose + fence body):

| Class | Count | Meaning |
|---|---|---|
| Intentional-error examples | 27 | page frames the failure (B0005 move errors, out-of-bounds, etc.) — legitimate |
| Disclosed limitations | 26 | prose says "disabled until X lands" (e.g. `stdlib/core/testing.mdx:85-88`, `fundamentals/traits.mdx:94-99`) — honest |
| Marked fragments | 22 | `...` elision / "continuing from above" — legitimate |
| **Hidden green** | 16 | **pass VM with extensions installed**; 5 pass strict vm+jit isolated (§7.3.2) — should be `runnable=true` |
| **Silent failures** | **51** | no disclosure, no error-framing, no fragment marker within 15 lines — the reader has no way to know the code doesn't run |

The 51 silent ones decompose further (by error class, from actual runs):

- **~28 cross-fence fragments** that reference bindings from earlier fences
  (`Undefined variable: 'events'` in `fundamentals/tables.mdx:76,109`;
  `'my_value'` in `advanced/content-addressed-bytecode.mdx:264`; `'id'` in
  `advanced/annotations.mdx:508` etc.) — excluding them is defensible but they are
  *indistinguishable* from broken examples to both readers and tooling; an explicit
  `fragment` marker class is missing from the fence grammar.
- **11 `Not implemented` stub hits** — documented-as-working features whose runtime is a
  surface-and-stop stub: `op_new_array(0)` under annotations (`advanced/annotations.mdx:73,89`),
  `HashMap.keys` (`fundamentals/objects-arrays.mdx:366`), f-string `FormatValueWithSpec`
  table-spec rendering (`fundamentals/strings.mdx:277,302`), the four stochastic intrinsics
  (`stdlib/core/stochastic.mdx:30-80`), `IntrinsicDistSampleN`
  (`stdlib/core/distributions.mdx:49`), trait `WrapTypeAnnotation`
  (`fundamentals/traits.mdx:71`).
- **~12 genuinely broken self-contained examples**: all of `stdlib/core/property_testing.mdx`
  (4), `stdlib/domain/physics.mdx` (2), `stdlib/math/optimize.mdx:78`,
  `stdlib/math/rotation.mdx:32` (the page's own `mat(3x3)` call passes 1 element where 9 are
  required — the *example itself* is wrong), `rotation.mdx:43`
  (`rotation::euler_to_matrix` unknown), `stdlib/domain/finance.mdx:16`,
  `stdlib/domain/iot.mdx:17`, `tooling/extensions.mdx:120`.

### 8.5 Output truth — where the gate can't see

**(a) ```text output blocks.** Only 5 shape-fence→```text pairs exist in the whole book (the
book overwhelmingly does *not* show expected output, which limits lie surface but also teaching
value). Of the 5: 2 match exactly; 1 is a runnable=false failing fence
(`fundamentals/content.mdx:107`); 1 shows a "Runtime value" for a fence that prints nothing
(`fundamentals/strings.mdx:102` — not a stdout claim); 1 was a probe timeout artifact
(archive.mdx — fence is green in the full matrix).

**(b) Inline `// comment` output claims.** 86 green snippets carry `print(x) // <claim>`
comments. Automated subsequence-matching plus manual verification of every flagged case found
**3 confirmed wrong-output claims, all in `stdlib/native/json.mdx`, all gate-green today**:

| Fence | Book claims | Actual (verified run) |
|---|---|---|
| `json.mdx:77` (`B__stdlib__native__json__3__L77`) | `print(t.volume)  // 1000` | `1000.0` |
| `json.mdx:157` | `print(first_name)  // Json::Str("Alice")` and `print(count)  // 2` | `Str("Alice")` and `2.0` |
| `json.mdx:221` | `print(result)  // Err("Json value is not a number")` | `Err({category: "RuntimeError", payload: "cannot convert kind Ptr(TypedObject) to number", cause: None, trace_info: None, message: "cannot convert kind Ptr(TypedObject) to number", code: "CONVERSION_FAILED"})` |

Beyond doc-truth, these expose two product papercuts: `Json` numeric leaves and `.len()` print
as floats (`2.0` for a count), and enum values print without the enum name (`Str(...)` not
`Json::Str(...)`) — inconsistent with how the book writes enum values everywhere else.
All other flagged claims proved to be descriptive comments (e.g. `// 3 — integer division`) or
matcher false-positives; e.g. `fundamentals/operators.mdx:33` and `functions.mdx:85` verified
correct line-by-line.

### 8.6 `shape/README.md` — both front-page examples fail to compile

Example 1 ("What Shape Looks Like", README lines 18-38) — actual transcript:

```
error[SEMANTIC]: Invalid pattern type: variant pattern 'Hash' belongs to enum 'Snapshot',
but the matched position has type 'Result'
```

`snapshot()` returns `Result<Snapshot, SnapshotError>` (`stdlib-src/core/snapshot.shape:5-26`);
the README predates the Result-ification. With `match snapshot()? {` the example runs and
prints `high` / `saved snapshot: 61d7dd1b...` — a one-character fix.

Example 2 ("Polyglot Example", README lines 56-66) — actual transcript:

```
error[SEMANTIC]: Foreign function 'std_dev': return type must be Result<number>
(dynamic language runtimes can fail on every call)
```

This is the ratified Q13 design (foreign returns must be `Result<T>`) — the *landing page*
version was updated (`-> Result<number>`, landing/index.html:223) but the README wasn't. With
`-> Result<number>` + `?` the example runs: `4.317406628984581`.

Also stale in README: `Vec<number>` works (alias) but the book teaches `Array<number>`;
"Learn Shape: `https://book.shape-lang.dev`" is consistent with the book site.

### 8.7 Landing page (`shape-web/landing/index.html`) — 3 of 5 rotating code samples are fiction

The hero rotator shows 5 "actual syntax" samples (blocks duplicated at lines 214-247 and
250-270). Verified each:

| Block | Verdict | Evidence |
|---|---|---|
| `type Sentiment` with `@description/@range/@example` | **WORKS** | ran green, printed `positive` |
| `fn python std_dev(...) -> Result<number>` | **WORKS** | ran green with extensions: `4.827007354458868` |
| `pub @db_schema() fn connect(const uri: string) { @table("users") struct User { id: u64, ... } }` | **FICTION** | parse error (`unexpected }`); `struct` is not a Shape keyword (Shape uses `type`), `u64` is not a Shape type (Shape has `int`), no `@db_schema/@table/@index` exist in stdlib |
| `let res = await @host("eu-west") fetch_order(id) ... emit(OrderReady { ... })` | **FICTION** | no `@host` annotation anywhere in `stdlib-src/` (grep empty); `emit` undefined; not parseable as shown |
| `match snapshot() { Snapshot::Hash(id) => resume(id), Snapshot::None => run_pipeline() }` | **DOUBLY WRONG** | `snapshot()` returns Result (same class as README bug) AND `Snapshot::None` variant does not exist — enum is `{ Hash(string), Resumed }` (`stdlib-src/core/snapshot.shape:5-10`) |

The landing also says `cargo install shape-cli` (line 157) — unverifiable offline in this audit;
prior release notes (memory: v0.3.0 partial crates.io publish lockout) make this a claim worth
re-verifying at release time. Meta description claims are otherwise supportable ("statically-typed,
comptime + annotations, polyglot inline functions, resumable snapshots, tiered JIT" — each is
demonstrated somewhere in the measured-green corpus).

### 8.8 `shape/examples/` — 5 of 6 files fail

| File | Result |
|---|---|
| `benchmark_strategy.shape` | parse error (`unexpected ';'` at :69) |
| `complete-language-example.shape` | parse error (`unexpected string` at :5) |
| `datetime_range_example.shape` | parse error (`unexpected ']'` at :13) |
| `debug_datetime.shape` | **runs** (prints `Start` / `42` / ...) |
| `execution_benchmarks.shape` | parse error (`Syntax error near: null`) |
| `test_series_methods.shape` | `Undefined function: 'series'` (retired API) |

The directory is described by CLAUDE.md's repo table only as part of the monorepo; nothing
references these files, and 5/6 predate multiple syntax migrations. They are the first thing a
code-reading visitor finds under `examples/`.

### 8.9 CLAUDE.md claims spot-checked against the binary

| CLAUDE.md claim | Verdict | Evidence |
|---|---|---|
| "Flow-sensitive narrowing: `if x != null { ... }` narrows `T?` to `T`" | **STALE** | `if x != null` is a parse error (`unexpected }` / `Syntax error near: = null`); the null-model was removed (shape-web commit `4ffb711` "Remove stale Shape null docs surface"); Option/None + `??`/`?.` is the shipped model |
| "Traits: `trait Name { fn method(self) -> ReturnType; }`" | **STALE** | `impl` methods with explicit `self` are rejected: `error[SEMANTIC]: Method 'drop' has an explicit 'self' parameter, but method receivers are implicit. Use 'method drop(...)'` — the book teaches `method` syntax (`resource-management.mdx:140-142`) |
| "Null coalescing: `expr ?? default`" | TRUE | verified `v ?? 7` → 7 with `Option<int>` |
| "Pipe operator: `expr \|> fn`" | TRUE | verified `21 \|> double` → 42 |
| "legacy `c"..."` syntax was retired in W18.3" | TRUE | `c"styled"` → `Undefined variable: 'c'` |
| "RAII ... escaping reference's Drop deferred to module end (§2.7.30)" | TRUE | transcript §6.1 |
| repo table: "docs/ = Marketing materials (pitch deck, one-pager)" | **MISLEADING** | `shape/docs/` is the engineering-doc tree; the marketing `docs/` is the parent-dir sibling outside this repo |
| "~11,800 tests" | not re-verified here (other auditors' territory) | — |

### 8.10 Vacuous green — what "passes" actually proves

Of the 548 plain-green runnable fences, **174 (32%) produce empty stdout** — for them the gate
verifies only "compiles and exits 0 in both modes" (and VM==JIT equality is trivially
`"" == ""`). Many are legitimately declaration-only teaching fences (type/trait definitions),
but the pattern concentrates suspiciously on the weakest pages:
`advanced/security-permissions.mdx`'s seven runnable-green fences include **four that are
100% comment lines** (e.g. `E__..._0__L54` is entirely the `// Conceptual flow inside the VM`
comment block; `_2__L124` is commented-out pseudocode of `check_permission`) — they cannot fail.
Meanwhile all eight of that page's *executable* permission examples are `runnable=false` and
fail (`Undefined variable: 'PermissionGrant'` etc.). Net effect: **the book's security chapter
has zero machine-verified executable examples** while contributing 7 green ticks to the gate.
Same shape (1 comment-only green fence) on `advanced/content-addressed-bytecode.mdx`. A
`min-output` or `no-comment-only` lint on runnable fences would close this hole.

### 8.11 Getting-started + install claims

The entire getting-started chapter is 22/22 green — the best chapter in the book, and the one
new users hit first. `installation.mdx:25-26` (`cargo install shape-cli`, `cargo install
shape-lsp`) depends on crates.io publish state that cannot be verified offline; everything else
in the chapter (first program, REPL, first-query) is demonstrated-true. `shape --version`
reports `shape 0.3.2`, consistent with the book's positioning of current release.

---

## 9. Bugs & Correctness Risks Found

Severity scale: P0 = unsound / wrong results / security-relevant; P1 = broken feature or
materially false documentation; P2 = paper cut.

### 9.1 [P0] JIT wrong-result + segfault on the book's first DateTime example (hidden by `runnable=false`)

`fundamentals/datetime.mdx:19` fence (`DateTime.now()` / `DateTime.utc()`):

```
$ shape run --mode vm  B__fundamentals__datetime__0__L19.shape   → true / true   (exit 0)
$ shape run --mode jit B__fundamentals__datetime__0__L19.shape
false                       ← wrong result for now.year() >= 2024
Segmentation fault          ← exit 139
```

Same signature at `datetime.mdx:364` and `datetime.mdx:404` (both VM-pass, JIT exit 139).
Wrong-answer-then-crash under the **default** execution mode class
(`advanced/jit-compilation.mdx:216` documents `--mode jit` as the CLI default) is P0. From this
vertical's angle: all three fences are excluded from the gate, so the book ships crash-repro
code labeled as documentation with no CI signal. (Root-cause belongs to the JIT vertical; the
repro is deterministic and takes <1 s.)

### 9.2 [P1] README both examples fail to compile (§8.6)

The two code blocks on the repo's front page are compile errors at HEAD. Both have verified
one-line fixes (`match snapshot()? {`, `-> Result<number>` + `?`).

### 9.3 [P1] Landing page ships 3 fictional/stale samples out of 5 (§8.7)

Including syntax that has never existed in the shipped grammar (`struct`, `u64`,
`@db_schema/@table/@index/@host`, `emit`, `Snapshot::None`). This is the project's primary
marketing surface claiming "actual syntax".

### 9.4 [P1] 51 silent-failing fences documented as working code (§8.4)

Notably: the whole of `stdlib/core/property_testing.mdx` — a bare
`use std::core::utils::property_testing` fails module load with
`error[SEMANTIC]: [E0900] post-inference FieldType::Any in user-facing schema
std::core::utils::property_testing::PropertySpec at field 'gen'` (the shipped stdlib module
violates the strict-typing gate the compiler enforces on users; direct transcript this
session). Likewise `stdlib/domain/finance.mdx:16` — a bare `use std::finance` dies with
`error[SEMANTIC]: Required parameter cannot follow a parameter with a default value` (stdlib
signature itself invalid; transcript this session). Plus `stdlib/math/rotation.mdx:32`
(example calls `mat(3x3)` with 1 element instead of 9 — error in the example itself),
`stdlib/domain/iot.mdx:17` (undefined `now`), and 11 `Not implemented` stub surfaces
(stochastic intrinsics, `HashMap.keys`, f-string format-spec table rendering, annotation-array
`op_new_array`).

### 9.5 [P1] Book-truth CI cannot trigger on shape/ commits (§5.2)

`book-truth.yml:19-40` trigger paths are shape-web-only while the header comment (lines 7-9)
claims shape/-side triggering. Every runtime regression reaches the book silently. The datetime
JIT segfault class (9.1) is the concrete demonstration.

### 9.6 [P1] `shape/examples/` 5/6 broken (§8.8)

### 9.7 [P2] Gate's "VM == JIT" verifies interpreter==interpreter for 83% of fences (§5.3)

467/565 runnable fences take `[jit-fallback]` on the JIT leg. Not a bug in the harness (exit
codes and stdout are still compared) but the acceptance criterion's words overstate coverage;
a JIT-side regression in top-level-heavy code is invisible to the gate by construction.

### 9.8 [P1] Security chapter effectively unverified while contributing green ticks (§8.10)

`advanced/security-permissions.mdx`: 4 of 7 runnable-green fences are comment-only blocks
(cannot fail); the page's 8 executable examples are all excluded and all fail. Given the
2026-07-04 audit history on security enforcement, a chapter that *looks* gate-verified while
verifying nothing is the most dangerous single page in the book.

### 9.9 [P2] Wrong-output comments in `stdlib/native/json.mdx` (3 confirmed, §8.5b)

Also surfaces two product papercuts: JSON numeric leaves/len print as `2.0`-style floats where
the book (and likely users) expect ints, and enum Display omits the enum path (`Str("Alice")`
vs documented `Json::Str("Alice")`).

### 9.10 [P2] `public/llms-full.txt` 8 days / 71 pages stale (§5.4)

An LLM consuming the published export reads pre-campaign content, including fences that were
since corrected as DOC-WRONG by `7ae8b9e`.

### 9.11 [P2] Legacy gate (`book/test-snippets.sh`) fails 0/11 due to stderr capture (§2.2, §4.3)

Environment-dependent: `2>&1` mixes the extension-loader banner into the comparison. Misleads
anyone following `book/README.md` (which itself documents a dead mdBook workflow).

### 9.12 [P2] Extractor's 200-byte fence-meta window (§3.2.1)

Latent: silently drops flags on long info-strings; nearest real fence is ~130 chars.

### 9.13 [P2] CLAUDE.md stale claims (§8.9)

`if x != null` narrowing and `fn method(self)` trait syntax are both parse/semantic errors at
HEAD. Agents (and this audit series) treat CLAUDE.md as ground truth; both claims send them
down non-existent syntax paths.

---

## 10. What Is Done Well

1. **The truth-gate architecture is the right shape.** One extractor, one manifest contract
   (`MANIFEST_SCHEMA.md`), one harness, a taxonomy of failure modes, per-slice CI parallelism —
   and it *dogfoods the product* (fixtures run real `shape serve` receivers and real
   `--resume <hash>` round-trips; 11/11 green through the official harness). Very few language
   projects execute their book in CI under two execution engines.
2. **Default-runnable convention.** `runnable=true` unless opted out
   (`remark-shape-snippet-label.ts:38`) puts the burden of proof on exclusion, not inclusion —
   which is why the exclusion set is countable and auditable at all. The entire §8 measurement
   is possible because of this design decision.
3. **The 2026-07-05 → 2026-07-11 remediation campaign is real and disciplined.** 240→565
   runnable (+135%), 498→142 excluded, full-universe truth 47%→80.6%. Commits show the right
   verbs: "correct DOC-WRONG fences", "promote hidden-green", "honest-mark pending"
   (`7ae8b9e`), feature-fix-then-flip (`702399d` "?. optional chaining fence runnable (fixed in
   wave7...)"). The book got truer both by fixing docs *and* by fixing the product.
4. **Honest-marking prose exists where it matters most**: `testing.mdx` Result-dispatch caveat,
   `traits.mdx` naming the deleted `ValueWord` dependency (deletion-fate vocabulary, exactly per
   CLAUDE.md discipline), `jit-compilation.mdx` refusing to print a latency number without a
   benchmark anchor and documenting the fallback semantics + the `tail | echo EXIT=$?` smoke
   harness defection it replaced.
5. **Gate unit tests encode past bugs** (frontmatter offset regression, timeout-vs-SIGKILL,
   hidden-files artifact upload comment at `book-truth.yml:106-109`) — institutional memory in
   test form.
6. **expected-fail gating** turns intentional-error examples into regression tests of
   *diagnostic quality* (`[B0005]`, `[B0003]` codes pinned in 6 fences) — error messages are
   part of the documented surface and are tested as such.
7. **getting-started at 100% green** — the funnel's first mile is fully verified.

## 11. What Is Done Poorly / Tech Debt

1. **`runnable=false` is semantically overloaded** — one flag means five different things
   (intentional error, fragment, undisclosed breakage, CI-environment-lacking, stale). §8.4
   quantifies: only 75/142 exclusions are self-evidently legitimate from page context. The
   fence grammar has no `fragment`, no `needs-extension`, no `broken cite=<issue>` classes —
   `runnable=deferred cite=` exists precisely for the last case and is used **zero** times.
2. **Readers can't see verification status.** `data-shape-runnable` is stamped into the HTML and
   then ignored by the theme — the one bit of the whole gate that could reach users is dropped
   at the last hop.
3. **Output truth is unowned**: 11/565 pinned outputs, no convention for output blocks (5 ad-hoc
   ```text pairs in 102 pages), inline `//` claims unchecked (3 confirmed wrong). The gate
   proves "runs", not "does what the page says".
4. **Internal engineering citations in user docs**: reader-facing pages cite ADR sections,
   W-series audits, and cluster-audit file paths (`objects-arrays.mdx:362`,
   `comptime-codegen.mdx:15`, `developer-tools.mdx:147`). These are meaningless (and
   unreachable) for book readers and will rot as internal docs move.
5. **Marketing surfaces have no gate**: README, landing page, `shape/examples/` — the three
   highest-traffic code surfaces are the three worst-failing ones measured (§8.6-8.8), and none
   is covered by any CI.
6. **Dead legacy infra left in place**: mdBook README, `test-snippets.sh` + 11 snippet pairs,
   `REWRITE_PLAN.md` describing a finished rewrite as future — all still committed, all
   actively misleading.
7. **Two-repo coupling is aspirational**: the gate's whole premise is "book truth tracks the
   runtime", but the trigger topology guarantees it only tracks *book edits*. There is no
   scheduled run either (`schedule:` absent from `book-truth.yml`) — a cron nightly would have
   been two lines.
8. **The stdlib documents its own unshipped surface**: property_testing, stochastic intrinsics,
   finance signature, `rotation::euler_to_matrix` — pages were written for API surfaces that
   don't pass their own module load. Doc-first development without the "mark it pending" step.

## 12. Prioritized Recommendations

**P0 (this week)**
1. Fix or quarantine the 3 datetime JIT-segfault fences (§9.1) — file against the JIT vertical
   with the 1-second repro; until fixed, add prose disclosure to `datetime.mdx` (the fence at
   L19 is the page's *first* example).
2. Fix `shape/README.md`'s two examples (verified one-line fixes, §8.6). Effort: minutes.
3. Replace landing blocks 5/6/7 with real syntax (block 7's fix is the same `snapshot()?`
   pattern; blocks 5/6 need honest replacements — annotation/db and distributed-await samples
   exist in the green corpus to copy from: `advanced/annotations.mdx:480`,
   `stdlib/core/remote.mdx:187`). Effort: <1 day.

**P1 (this month)**
4. Split `runnable=false` into typed exclusion classes (`fragment`, `error-example` — can be
   auto-inferred from `expected-fail`, `needs-extension`, `pending cite=<id>`), require `cite=`
   for `pending`, and make the truth-gate report count each class. The 51 silent failures then
   become a burn-down list. While in there, add a `no-comment-only` lint for runnable fences
   (kills the 4 fake-green security fences, §8.10) and report vacuous-green (empty-stdout)
   counts per page. Effort: 1-2 days extractor+harness, plus the marking campaign
   (mechanical; this audit's per-fence classification is in the scratch corpus).
5. Add a nightly `schedule:` + `workflow_dispatch` gate run against shape@main, and/or a
   mirror workflow in the shape repo triggering on `crates/**` + `stdlib-src/**`. Effort: hours.
6. Surface `data-shape-runnable` in the theme (badge or border on unverified fences). Effort:
   hours — the attribute is already in the DOM.
7. Promote the 5 strict hidden-green fences; re-run exclusion probe quarterly (script exists in
   this audit's scratchpad). Effort: hours.
8. Delete legacy: `book/README.md` (replace with pointer to HOW_TO_BUILD), `test-snippets.sh`,
   `book/snippets/`, `REWRITE_PLAN.md` (or mark historical); fix or delete `shape/examples/`
   (5 files). Effort: hours.
9. Regenerate `llms-full.txt` in the book build (`npm run build` hook or CI artifact) so it can
   never lag the corpus. Effort: hours.

**P2 (quarter)**
10. Grow `expected=` coverage beyond fixtures — start with every fence whose page shows output
    (```text pairs) and the `stdlib/native/json.mdx` page (fix its 3 wrong comments; decide
    whether `2.0`-for-len and `Str(...)` displays are the intended product behavior first).
11. Adopt an "output block" convention (```text following a fence, harness-checked) so output
    claims become machine-checkable instead of comment folklore.
12. Fix the extractor meta-window (parse the full fence first line, not `offset+200`); add a
    long-meta regression test. Fix CLAUDE.md's stale `x != null` narrowing + `fn method(self)`
    trait syntax claims and the `docs/` repo-table row.
13. Sweep internal ADR/W-series citations out of reader-facing prose (keep them in HTML
    comments if needed for maintainers).

---

## Appendix A. Per-page truth table (all 91 pages with fences, full working-tree run 2026-07-11)

Columns: green/total counts **all** fences on the page (both modes exit 0, stdout byte-equal;
fixtures via official harness); runnable-green/runnable is the gate's own view. Divergence
between the two columns is the exclusion blind spot.

| Page | Green/Total | Runnable green | Page truth |
|---|---|---|---|
| advanced/annotations.mdx | 9/12 | 9/9 | 75% |
| advanced/comptime-annotations-cookbook.mdx | 9/13 | 9/9 | 69% |
| advanced/comptime-llm-patterns.mdx | 4/5 | 4/4 | 80% |
| advanced/comptime.mdx | 9/10 | 9/9 | 90% |
| advanced/content-addressed-bytecode.mdx | 7/16 | 7/7 | 44% |
| advanced/developer-tools.mdx | 0/5 | 0/0 | 0% |
| advanced/jit-compilation.mdx | — no fences — | | |
| advanced/module-distribution.mdx | 0/1 | 0/0 | 0% |
| advanced/native-c-interop.mdx | 4/7 | 4/4 | 57% |
| advanced/ownership-deep-dive.mdx | 10/19 | 10/10 | 53% |
| advanced/polyglot-distributed.mdx | 3/4 | 3/3 | 75% |
| advanced/resumability.mdx | 2/2 | 2/2 | 100% |
| advanced/security-permissions.mdx | 7/15 | 7/7 | 47% |
| advanced/wire-protocol.mdx | 0/1 | 0/0 | 0% |
| examples/comptime-codegen.mdx | 0/1 | 0/0 | 0% |
| examples/hello-world.mdx | 1/1 | 1/1 | 100% |
| examples/web-request.mdx | 0/1 | 0/0 | 0% |
| fundamentals/async.mdx | 9/10 | 9/9 | 90% |
| fundamentals/builtin-types.mdx | 2/2 | 2/2 | 100% |
| fundamentals/content.mdx | 11/15 | 11/11 | 73% |
| fundamentals/control-flow.mdx | 9/9 | 9/9 | 100% |
| fundamentals/datetime.mdx | 17/20 | 17/17 | 85% |
| fundamentals/enums.mdx | 12/12 | 12/12 | 100% |
| fundamentals/error-handling.mdx | 8/14 | 8/8 | 57% |
| fundamentals/functions.mdx | 28/28 | 27/27 | 100% |
| fundamentals/integer-types.mdx | 4/4 | 4/4 | 100% |
| fundamentals/modules.mdx | 11/13 | 11/11 | 85% |
| fundamentals/names-and-scope.mdx | 4/4 | 4/4 | 100% |
| fundamentals/objects-arrays.mdx | 22/23 | 22/22 | 96% |
| fundamentals/operators.mdx | 29/31 | 29/29 | 94% |
| fundamentals/pattern-matching.mdx | 5/5 | 5/5 | 100% |
| fundamentals/references-borrowing.mdx | 12/15 | 12/12 | 80% |
| fundamentals/resource-management.mdx | 15/17 | 15/15 | 88% |
| fundamentals/strings.mdx | 16/19 | 16/16 | 84% |
| fundamentals/tables.mdx | 4/8 | 4/4 | 50% |
| fundamentals/traits.mdx | 16/21 | 15/15 | 76% |
| fundamentals/variables.mdx | 15/17 | 15/15 | 88% |
| getting-started/basic-concepts.mdx | 10/10 | 10/10 | 100% |
| getting-started/first-query.mdx | 8/8 | 8/8 | 100% |
| getting-started/installation.mdx | 2/2 | 2/2 | 100% |
| getting-started/repl.mdx | 2/2 | 2/2 | 100% |
| index.mdx | 1/1 | 1/1 | 100% |
| stdlib/core/collections.mdx | 7/7 | 7/7 | 100% |
| stdlib/core/distributions.mdx | 4/5 | 4/4 | 80% |
| stdlib/core/log.mdx | 3/3 | 3/3 | 100% |
| stdlib/core/math.mdx | 14/14 | 14/14 | 100% |
| stdlib/core/monte_carlo.mdx | 3/3 | 2/2 | 100% |
| stdlib/core/ode.mdx | 4/4 | 4/4 | 100% |
| stdlib/core/property_testing.mdx | 0/4 | 0/0 | **0%** |
| stdlib/core/random.mdx | 6/6 | 6/6 | 100% |
| stdlib/core/remote.mdx | 6/8 | 6/6 | 75% |
| stdlib/core/rolling.mdx | 3/3 | 3/3 | 100% |
| stdlib/core/set.mdx | 5/5 | 5/5 | 100% |
| stdlib/core/snapshot.mdx | 3/3 | 3/3 | 100% |
| stdlib/core/state.mdx | 22/29 | 22/22 | 76% |
| stdlib/core/stochastic.mdx | 1/5 | 1/1 | **20%** |
| stdlib/core/testing.mdx | 4/8 | 4/4 | 50% |
| stdlib/core/transport.mdx | 3/5 | 3/3 | 60% |
| stdlib/domain/finance.mdx | 0/1 | 0/0 | **0%** |
| stdlib/domain/iot.mdx | 0/2 | 0/0 | **0%** |
| stdlib/domain/physics.mdx | 0/2 | 0/0 | **0%** |
| stdlib/domain/simulation.mdx | 1/4 | 1/1 | 25% |
| stdlib/math/interpolation.mdx | 1/2 | 1/1 | 50% |
| stdlib/math/linalg.mdx | 9/9 | 9/9 | 100% |
| stdlib/math/optimize.mdx | 3/5 | 3/3 | 60% |
| stdlib/math/rotation.mdx | 2/4 | 2/2 | 50% |
| stdlib/native/archive.mdx | 3/3 | 3/3 | 100% |
| stdlib/native/compress.mdx | 7/7 | 7/7 | 100% |
| stdlib/native/crypto.mdx | 14/14 | 14/14 | 100% |
| stdlib/native/csv.mdx | 7/7 | 7/7 | 100% |
| stdlib/native/env.mdx | 1/1 | 1/1 | 100% |
| stdlib/native/file.mdx | 6/6 | 6/6 | 100% |
| stdlib/native/http.mdx | 1/1 | 1/1 | 100% |
| stdlib/native/io.mdx | 15/15 | 15/15 | 100% |
| stdlib/native/json.mdx | 14/14 | 14/14 | 100% (but 3 wrong `//` output claims, §8.5) |
| stdlib/native/math.mdx | 4/4 | 4/4 | 100% |
| stdlib/native/msgpack.mdx | 5/5 | 5/5 | 100% |
| stdlib/native/parallel.mdx | 1/1 | 1/1 | 100% |
| stdlib/native/regex.mdx | 7/7 | 7/7 | 100% |
| stdlib/native/time.mdx | 7/7 | 7/7 | 100% |
| stdlib/native/toml.mdx | 4/4 | 4/4 | 100% |
| stdlib/native/unicode.mdx | 5/5 | 5/5 | 100% |
| stdlib/native/xml.mdx | 4/4 | 4/4 | 100% |
| stdlib/native/yaml.mdx | 5/5 | 5/5 | 100% |
| tooling/docstrings.mdx | 6/6 | 6/6 | 100% |
| tooling/execution-server.mdx | 1/1 | 1/1 | 100% |
| tooling/extensions.mdx | 0/1 | 0/0 | 0% |
| tooling/frontmatter.mdx | 1/1 | 1/1 | 100% |
| tooling/packages.mdx | 1/1 | 1/1 | 100% |
| tooling/polyglot.mdx | 4/8 | 4/4 | 50% |
| tooling/python-extension.mdx | 3/8 | 2/2 | 38% (5 more pass VM with extensions installed — CI-hermeticity exclusions, §5.5) |
| tooling/typescript-extension.mdx | 3/7 | 2/2 | 43% (ditto) |

Reading of the table: the **stdlib/native** chapter (19 pages, all 100%) and **getting-started**
are fully verified; the rot concentrates in `stdlib/domain/*`, `stdlib/core/{property_testing,
stochastic,testing}`, `stdlib/math/{rotation,optimize,interpolation}`, and the `advanced/`
distributed/security/devtools cluster — exactly the pages documenting the newest or
least-finished runtime surfaces. The "Runnable green" column being all-green everywhere is the
denominator trap in miniature: **the gate cannot distinguish a 100% page from a 0% page** when
the 0% page simply excludes everything.


