# Vertical Deep-Dive Audit 15: Testing & Quality Infrastructure

**Auditor:** 15 of 19 (ultra-deep-dive, commissioned by project owner)
**Date:** 2026-07-11
**Scope:** tiered test system (justfile), tools/shape-test, tools/shape-fuzz, tools/vmjit-diff, tests/ + testdata/ + test-arena/, ci/ + .github/workflows/, scripts/ (verify-merge.sh, check-no-dynamic.sh, and siblings), sentinel test no_dynamic.rs, benchmarks/ + benchmark-integrity rule, deep-tests feature gating.
**Method:** working-tree audit (dirty tree as-is), static reading + live gate executions + GitHub Actions API queries. Cargo budget respected (≤2 narrow invocations).

## 0. Executive summary

### Verdict

The testing and quality infrastructure of Shape is, at the artifact level, among the most
elaborate this auditor has seen in a project of this size: a five-tier `just` test system with a
~48 GiB per-process memory backstop, an 11 KLOC+ battery of gate scripts, a 15-check
exit-code-based merge verifier, a frozen-baseline forbidden-symbol guard, THREE independent
VM-vs-JIT differential harnesses, a 707-fence book truth-gate, a per-test resource-limited
integration framework with ~7,260 tests in 347 files, and a compiled-in sentinel test that
re-implements one gate row in Rust so it survives even when the shell gate is skipped.

At the *system* level, however, the infrastructure has one catastrophic failure and a cluster of
serious ones. The catastrophic one: **the remote CI has been validating a 6-week-old snapshot of
the project**. Local `main` is **1,872 commits ahead of `origin/main`** (last push 2026-05-26,
v0.3.2). Every scheduled "success" (nightly-fuzz, coverage — green daily through 2026-07-11) runs
against that stale snapshot; and the last push-triggered CI runs on record **all failed** (8+
consecutive `CI` failures 2026-05-18 → 2026-05-26, 9 test failures in the final run) while
releases were tagged and shipped anyway (release.yml builds artifacts with no test gate). The
net effect is that ~6 weeks of the most invasive work in the project's history (GC default flip,
W17 snapshot completion, strict-flip, wave-7 merges) has **zero CI history**, and the visible
green checkmarks actively mislead.

Below that headline: two of the 15 verify-merge checks are silent no-ops (a ripgrep flag
incompatibility swallowed by `2>/dev/null || true` — a planted merge-marker file is not
detected); `just test-deep` filters away most of the deep tests it claims to run;
`check-no-dynamic` is wired into neither CI nor pre-commit despite CLAUDE.md claiming both;
the coverage "gate" cannot fail by construction; and the "fuzzer" has never fuzzed (mutation
engine written, tested, and never wired to the CLI). Meanwhile the parts that DO run — the
tiered suite, the curated differential gate (re-ran live for this audit: 13/13 convergent on the
working tree), the book truth-gate (565/565 runnable snippets green on 2026-07-10), the
vmjit-diff corpus (466/467 MATCH, 1 pinned known-red) — are in genuinely good shape.

### Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P0** | All CI signal is 6 weeks stale: local main 1,872 commits ahead of origin/main (frozen at v0.3.2, 2026-05-26); nightly fuzz/coverage "green" validates the old snapshot | `git rev-list --count origin/main..main` = 1872; `gh run list` transcripts in §2.6/§9.1 |
| 2 | **P0** | Last-known CI state on every push was FAILURE (8+ consecutive red runs, 9 failing tests at v0.3.2), yet v0.3.0/1/2 releases shipped — release.yml has no test gate | `gh run list --workflow=ci.yml`; release.yml build-only (§9.2) |
| 3 | **P1** | verify-merge.sh CHECK 4 + CHECK 7 (merge-marker scans) + CHECK 11 (grep -c anti-pattern tripwire) are silent no-ops: rg 15.1 rejects `-E`/`--include`, error swallowed by `2>/dev/null \|\| true`; planted marker file NOT detected | live transcript §9.3; scripts/verify-merge.sh:115-117,361-363,472-473 |
| 4 | **P1** | `just test-deep` is broken: the `-- deep` name filter excludes most deep-gated modules (`hashmap_ops`, `iterator_ops`, `operator_overload`, `extend_blocks`, `differential_trusted`, `trusted_edge_cases` contain no "deep" substring) and it omits the `--test-threads=1` its own sibling comment requires for shape-jit/deep-tests | justfile:93 vs crates/shape-vm/src/executor/tests/mod.rs:74-89 (§9.4) |
| 5 | **P1** | CLAUDE.md "Mechanical enforcement" claims check-no-dynamic runs "on every CI run and pre-commit" — false on both: ci.yml has no such step, .git/hooks/pre-commit is stash/conflict-marker only; coverage.yml repeats the false claim | ci.yml (all 77 lines); .git/hooks/pre-commit grep EXIT=1 (§5.1) |
| 6 | **P1** | shape-fuzz has never fuzzed: CLI is corpus-replay only; mutation.rs (679 LOC) + minimizer.rs (360 LOC) unit-tested but unreachable from the binary; `--seed` accepted and ignored; nightly job replays 59 fixed seeds | tools/shape-fuzz/src/main.rs:5-13,61-65 (§2.4) |
| 7 | **P1** | Coverage gate cannot fail: coverage.sh never wires `--fail-under`; coverage.yml wraps every measurement step in `continue-on-error: true`; the ratified "≥99% per-feature" criterion is unenforced 7+ weeks after landing | scripts/coverage.sh:229-237; .github/workflows/coverage.yml:66-83 (§9.6) |
| 8 | **P1** | nightly-fuzz can never alert: corpus contains 5 known-negative seeds ⇒ harness always exits 1 ⇒ `continue-on-error: true` ⇒ job always green; shape-fuzz has no known-red allowlist (unlike vmjit-diff), so NEW divergences are indistinguishable from expected ones | .github/workflows/nightly-fuzz.yml:44-56 (§9.7) |
| 9 | **P1** | Two quality gates FAIL on the current working tree and two gate scripts are wired nowhere: check-heapkind-wildcards.sh fails on untracked `closure_layout_fallback.rs:174` wildcard; check-ignored-test-classification.py fails on 76→72 drift; neither classification nor typed-opcode-proof script is referenced by justfile/CI/verify-merge | live transcripts §9.8 |
| 10 | **P2** | Ignored-test documentation is stale in both directions: CLAUDE.md documents ~23 jit + 4 simulation ignores; reality is 78 shape-vm + 21 shape-jit + 21 shape-cli + 2 shape-test attribute-level ignores, and the 4 simulation.rs ignores no longer exist (fixed, doc not updated) | §7.3 full inventory |

Additional notable: `just ci-test` would SIGABRT if ever run (`--include-ignored` hits two
extern-C `todo!()` aborting tests — their own ignore messages say so) (§9.9);
`book_doctests.rs` silently greens when the book checkout is absent — which is exactly the
GitHub CI environment (§9.10); justfile `verify-phase-5` carries a stale TODO claiming the
sentinel test isn't wired when it is (§5.4).

### Scores

**Feature completeness: 72/100.** Every category of quality infrastructure exists and most of it
genuinely works when invoked locally (tiers, differential gates, book gate, merge verifier,
baseline guard, sentinel); but the fuzzer's core feature (fuzzing) is unshipped, the coverage
gate enforces nothing, two merge-verifier checks are inert, and the CI feedback loop — the thing
that makes all the rest matter — has been severed for six weeks.

**Code quality: 74/100.** Gate scripts are disciplined (set -euo pipefail, exit-code-based
verdicts, extensively documented rationale, frozen baselines with monotonic rules), the test
framework is a clean fluent builder with per-test resource limits, and test assertion quality is
high (exact-value expectations). Docked for: the CHECK 4/7 rg bug class (an error-swallowing
anti-pattern in the very script that lectures about error-swallowing), gawk-only awk constructs,
a redundant `unsafe impl Send/Sync`, three 1,300-1,600-line single-file hotspots, and
duplicated harness logic across five VM-vs-JIT comparison mechanisms.

### Biggest risk

The biggest risk is **the illusion of verification**. This project's culture writes a gate for
everything — but a gate that isn't wired into an enforcement point (classification,
typed-opcode-proof, check-no-dynamic-in-CI), that cannot fail (coverage, nightly-fuzz), that
silently no-ops (CHECK 4/7, book_doctests without checkout), or that runs against a 6-week-old
commit (all of GitHub Actions) produces green signals that are worse than no signal, because
they suppress the alarm a missing gate would raise. The 2026-07-04 audit already found this
pattern once ("foreign e2e tests gated out of every tier"); the FFI job was then added — to a CI
that no longer runs against current code. Re-establishing one honest, continuously-exercised
enforcement point (push main; make CI red mean stop) matters more than any individual gate fix
below.

## 1. Architecture & code structure map

### 1.1 Component inventory (LOC measured by `wc -l` on the working tree)

| Component | Path | LOC | Responsibility |
|-----------|------|-----|----------------|
| Tier system | `justfile` | 249 | 24 recipes: test tiers 0-3, ffi tier, gates, coverage, diff harnesses |
| Merge gate | `scripts/verify-merge.sh` | 605 | 15 exit-code-based checks (CHECK 1-14 incl. 6b) |
| Defection guard | `scripts/check-no-dynamic.sh` | 60 | per-symbol monotonic count vs frozen baseline |
| Baseline | `docs/check-no-dynamic-baseline.txt` | 43 | 40 pattern rows (limit + PCRE + note) |
| Wildcard guard | `scripts/check-heapkind-wildcards.sh` | 160 | new HeapKind wildcard-arm detector w/ audited baseline |
| Ignored-test classifier | `scripts/check-ignored-test-classification.py` | 383 | source-level ignore-reason bucket counts vs expected |
| Typed-opcode proof guard | `scripts/check-typed-opcode-proof-coverage.py` | 279 | classifies typed-opcode emissions into proof buckets |
| Miri gate | `scripts/check-miri-provenance.sh` | 225 | narrow nightly-Miri provenance filters |
| Coverage wrapper | `scripts/coverage.sh` | 237 | tarpaulin line/branch/dead-code/per-crate modes |
| Curated diff gate | `scripts/differential-gate.sh` | 58 | 13 golden seeds through shape-fuzz subprocess harness |
| Failure-set differ | `scripts/compare_cargo_failures.sh` | 161 | extracts + diffs `failures:` lists from saved cargo logs |
| Worktree tooling | `scripts/create-agent-worktree.sh`, `install-git-stash-wrapper.sh` | 109+87 | parallel-agent dispatch hygiene |
| Bench gate | `ci/bench-gate.sh` | ~110 | criterion regression gate (baseline compare) |
| Bench regression parser | `ci/check_regressions.py` | 140 | criterion change/estimates.json threshold check |
| Workflows | `.github/workflows/*.yml` (6) | 475 total | ci 76, benchmarks 43, coverage 92, nightly-fuzz 66, release 152, vscode-publish 46 |
| Integration framework | `tools/shape-test/src/` | 1,697 | `shape_test.rs` 1,568 (fluent builder), `book_snippets.rs` 127 |
| Integration suite | `tools/shape-test/tests/` | 101,051 in 347 files | ~7,260 `#[test]` fns across 66 feature dirs |
| Differential fuzzer | `tools/shape-fuzz/src/` | 1,821 | lib 308, main 200, divergence 274, minimizer 360, mutation 679 |
| Fuzz corpus | `tools/shape-fuzz/tests/corpus/` | 59 seeds | 7 domains: arithmetic/collections/closures/patterns/async/generics/fallthrough |
| VM/JIT diff harness | `tools/vmjit-diff/` | 637 (mjs) | run-diff.mjs 376, build-corpus.mjs 261; corpus 467 programs; known-red.json |
| Workspace automation | `tools/xtask/src/main.rs` | 1,296 | workspace-smoke, vmvalue/line-budget/bench-specialization/native-docs guards, perf-regression-gate, loc-check, grammar-parity, doctest |
| Smoke fixtures | `tests/smokes/` (7), `tests/smokes-fallback/` (6) | — | s1-s5 canonical matrix + 6 JIT-Err-class fallback fixtures, each with a binding README harness |
| Sentinel test | `crates/shape-vm/src/executor/tests/no_dynamic.rs` | 99 | Rust-layer mirror of one baseline row (Bool-default fabrication) |
| Benchmarks | `benchmarks/` | 21 shape programs | + go/node/python/rust ports, run_all.sh, RESULTS.md, tracking/ TSVs, ci_jit_node_budget.tsv |
| Test data | `testdata/` (1 file), `test-arena/` (1 file) | — | sample.csv; wave1b_custom_iterator.shape |

### 1.2 The tier system (justfile)

- **Tier 0** `test-check` = `check-clean` = `cargo check --workspace --all-targets`
  (justfile:56,164-165). Benches rejoined the gate 2026-07-05 (justfile:152-157).
- **Tier 1** `test-fast` (justfile:59-60): `--lib` only, excludes shape-test + both extension
  crates, wrapped in `ulimit -v 50331648` (48 GiB VM backstop, justfile:5-12).
- **Tier 2** `test` (justfile:63-64): tier 1 + `deep-tests` features on shape-vm, shape-runtime,
  shape-ast, **and shape-jit** (see §9.5 for the SIGILL contradiction).
- **Tier 3** `test-all` (justfile:87-89): workspace minus shape-test with 3 deep-tests features
  (NOT shape-jit's), then shape-test separately at `--test-threads=1` (annotations_comptime
  parallel flake, justfile:83-86).
- `test-deep` (justfile:92-93): same features plus shape-jit/deep-tests, filter `-- deep
  --include-ignored` — broken, see §9.4.
- `test-ffi` (justfile:122-124): builds extensions, runs `ffi_e2e` with `--include-ignored` +
  gated shape-test e2e; deliberately NOT ulimit-wrapped (V8 cage reservation documented,
  justfile:112-121).
- `ci-test` (justfile:133-135): `--include-ignored` across workspace — never referenced by any
  workflow and would SIGABRT (§9.9).
- `differential-gate`, `diff-vmjit`, `coverage`, `miri-provenance`, `check-no-dynamic`,
  `verify-merge` wrap the scripts above.

### 1.3 Data flow of the quality signal

```
developer edit
  └─ pre-commit hook (.git/hooks/pre-commit, 149 LOC)
       ├─ stash-reflog guard (worktree discipline)
       └─ staged-diff conflict-marker grep          ← ONLY live merge-marker guard (see §9.3)
  └─ just test-fast / test                          ← local, actually used per CLAUDE.md workflow
  └─ scripts/verify-merge.sh (15 checks)            ← merge gate for sub-cluster branches
       ├─ CHECK 1/2: cargo check exit codes
       ├─ CHECK 3: check-no-dynamic.sh (43-row frozen baseline)
       ├─ CHECK 4/7: merge-marker scans             ← SILENT NO-OPS (§9.3)
       ├─ CHECK 5/6/6b/8: HeapKind ordinal/lockstep/brace guards (gawk)
       ├─ CHECK 9-13: use-dup, receiver-recovery heuristic, grep-c anti-pattern,
       │              HK_* ordinal guard, colon-return-type doc guard
       └─ CHECK 14: check-heapkind-wildcards.sh     ← currently FAILING on working tree (§9.8)
  └─ push to origin/main                            ← SEVERED SINCE 2026-05-26 (§9.1)
       ├─ ci.yml: check → test → workspace-smoke → differential gate → ffi job
       ├─ benchmarks.yml: cargo bench + perf-regression-gate (advisory)
       ├─ coverage.yml (also nightly): tarpaulin, cannot fail (§9.6)
       └─ nightly-fuzz.yml: 59-seed replay, cannot fail (§9.7)
  └─ tag push → release.yml: cross-compile + publish, NO tests (§9.2)
```

### 1.4 Key types / entry points

- `ShapeTest` builder (tools/shape-test/src/shape_test.rs:100-127): holds source text, LSP
  position/range, stdlib flag, snapshot tempdir, `PermissionSet`, `ResourceLimits`,
  `ExecMode {Vm, Jit}` (shape_test.rs:93-98, added for WF-0B in-process differential).
  `eval_with_output` (shape_test.rs:256-342) drives `ShapeEngine` + either `BytecodeExecutor`
  or `shape_jit::JITExecutor` with a `CaptureAdapter` output sink.
- `shape_fuzz::compare_outputs / classify_divergence / record_finding` (tools/shape-fuzz/src/lib.rs)
  — subprocess differential; 8-class `Divergence` taxonomy + 5-level `Signal`
  (divergence.rs:54-89).
- `run-diff.mjs` — resumable Node runner writing `reports/report.{json,md}` + `progress.jsonl`,
  known-red pinning via `known-red.json` (5 pinned classes).
- xtask `workspace_smoke` (tools/xtask/src/main.rs:1267-1296): vmvalue guard → line budget →
  benchmark-specialization guard → native-docs guard → cargo check → cargo test --all-targets →
  advisory perf gate.

## 2. Feature completeness

Legend: **WORKS** = verified end-to-end in this audit (transcript or fresh artifact);
**EXISTS** = code present, plausible, not independently executed here; **PARTIAL**;
**BROKEN**; **MISSING**.

### 2.1 Tiered test system — PARTIAL (core tiers EXISTS/WORKS, two recipes defective)

- Tier recipes exist with correct target sets and documented rationale (justfile:52-135).
  Not re-run here (18-sibling cargo-lock budget); the tier structure matches CLAUDE.md's
  documentation exactly.
- `test-deep` — **BROKEN** as specified: its `deep` name-filter drops ~6 of 8 deep-gated
  shape-vm modules (§9.4).
- `ci-test` — **BROKEN/dead**: `--include-ignored` would abort on
  `crates/shape-jit/src/ffi/async_ops.rs:296` and `ffi_symbols/simulation/mod.rs:118`
  (both ignore-reasons state the `todo!()` body SIGABRTs the test process); no workflow calls it
  (§9.9).
- The 48 GiB `ulimit -v` backstop (justfile:5-12) plus the in-process per-buffer alloc ceiling
  (commit 1fff04da, `shape-value::v2::alloc_budget`) is the two-layer answer to the June
  bulk-hang; see §2.8.

### 2.2 shape-test framework — WORKS

- Fluent builder with exact-value assertions; per-test default resource limits
  (2e9 instructions / 2 GiB heap-growth / 120 s wall, shape_test.rs:216-223) so runaway
  programs fail in-process (shape_test.rs:308-329 documents the 83 GB OOM class it bounds).
- Dual-mode execution: `with_jit()` drives `shape_jit::JITExecutor` with the same
  limits/permissions wiring as the release binary (shape_test.rs:281-304).
- LSP assertions run the real shape-lsp entry points (hover/completions/definition/etc.,
  shape_test.rs:346+).
- 347 test files / 101,051 LOC / ~7,260 `#[test]`s across 66 feature directories
  (`ls tools/shape-test/tests/`), including dedicated `jit/`, `numeric_conversions_jit/`,
  `snapshots_resume/`, `security_permissions/`, `wire_protocol/`, `regression/` suites.

### 2.3 VM-vs-JIT differential harnesses — WORKS (three of them)

1. **shape-fuzz subprocess harness + curated gate**: re-ran live for this audit with the
   working-tree debug binaries — all 13 curated seeds `Convergent` (transcript §9.11); exit 0.
2. **vmjit-diff** (Node): last report 2026-07-06 (`tools/vmjit-diff/reports/report.md`):
   467 programs, MATCH=466, DIVERGED=1 (pinned known-red `hof-return-kind-raw-bits`),
   0 unexpected non-MATCH. known-red.json carries 5 pinned classes with audit citations and an
   explicit "must never become a dumping ground" rule.
3. **In-process `ExecMode::Jit`** in ShapeTest — used by shape-test suites (e.g.
   `numeric_conversions_jit/`, `regression/jit.rs` 1,252 LOC).

### 2.4 shape-fuzz "fuzzing" — PARTIAL (differential replay works; fuzzing MISSING)

The crate self-describes the gap: "There is no mutation engine, no minimizer engine, no CI
integration — those land in W13.3 and W13.4" (main.rs:5-7). W13.3 landed `mutation.rs` (679 LOC,
14 unit tests) and `minimizer.rs` (360 LOC, 6 unit tests) as **library code only**; the CLI still
exposes a single `run` (replay) subcommand, and `--seed` is "Reserved for future mutation
seeding … ignores the value" (main.rs:61-65). Nothing in scripts/, justfile, or workflows calls
`mutate_seed` / `minimize_failure`. Net: the project has never generated a single fuzz input.
The README (tools/shape-fuzz/README.md "Adding corpus seeds") is honest that mutation-engine CLI
exposure is a "W13.4-follow-up candidate" — that follow-up did not happen, while the workflow
name (`nightly-fuzz-differential`) and README framing continue to say "fuzz".

### 2.5 Book truth-gate — WORKS (in shape-web; silently-skipping mirror in shape-test)

- Canonical harness `shape-web/book/book-site/scripts/run-book-truth-gate.mjs` runs every
  runnable snippet under both modes with an 8-category failure taxonomy + serve /
  serve-snapshot-resume / local-snapshot-resume fixture modes.
- Latest report (`.book-truth-gate/report.json`, generated 2026-07-10T07:13Z against
  `shape/target/release/shape`): **total 565, pass 565, fail 0**; manifest has 707 fences of
  which 565 runnable / 142 non-runnable. The "denominator trap" flagged in the 2026-07-05 era
  (240-green curated subset vs ~47% real truth) has been substantially closed by the book-truth
  campaign (commit f4a83de7: 85.0%→86.5%, curated 246→354; now 565); the residual risk is the
  142 `runnable=false` fences that remain outside the gate.
- shape-test's `book_doctests.rs` mirror silently passes when the snippets dir is missing
  (§9.10).

### 2.6 CI — EXISTS, severed from reality

Workflows are real and well-constructed (ci.yml even encodes the 2026-07-04 audit lesson: the
`ffi` job exists so "the foreign path can never silently die again", ci.yml:39-46). But:

```
$ git log -1 origin/main --format='%ci %h %s'
2026-05-26 19:50:02 +0200 82f049dd v0.3.2: fix print() ...
$ git rev-list --count origin/main..main
1872
$ gh run list --workflow=ci.yml --limit 8       # all 8: completed failure (05-18 .. 05-26)
$ gh run list --limit 30                        # 30 most recent runs: all schedule-triggered, all success
```

The scheduled coverage (1h15m) and nightly-fuzz (~9-15 min) jobs run daily and pass — against
v0.3.2. Push-triggered validation of the 1,872 new commits: none.

### 2.7 Coverage gate — EXISTS as measurement, MISSING as gate (§9.6)

### 2.8 Bulk-hang lane — RESOLVED (diagnosis-first mandate satisfied)

The June "613-test bulk run hangs" priority lane closed with a root cause and an in-process fix:
geometric doubling-realloc growth vs linear instruction counting, fixed by a per-execution
per-buffer byte ceiling in `shape-value::v2::alloc_budget` consulted by `TypedArray::grow` /
`TypedMap::grow_buckets` (commit 1fff04da, 2026-06-17). Corroborated by the framework comment
"the full 613-test operators binary peaks at ~27MB / 8.4s" (shape_test.rs:322-323) — i.e. the
previously-hanging binary now completes — plus the justfile ulimit backstop for anything the
in-VM caps miss (justfile:5-12). The STAGE3 deterministic rerun docs
(docs/cluster-audits/afinal-roots/STAGE3-FINAL-DETERMINISTIC-RERUN.md) show full strict-flip-era
suite sweeps completing with counted failure sets, which is only possible post-fix.

### 2.9 Pre-existing shape-test failure clusters (~48) — status: superseded, doc stale

CLAUDE.md still documents "Pre-existing shape-test failure clusters (~48 tests, present on
jit-v2-phase1@53a06ce baseline)" with 7 named classes. That snapshot is from the May era; since
then the strict-flip classification waves and the wave-7 book-truth campaign reworked exactly
these classes (e.g. `window_functions`, `strings .join`, array slice/sort/some appear throughout
docs/cluster-audits/v0.3-classification/). The working tree's only shape-test-level `#[ignore]`s
are 2 (objects.rs:265 `len()` TypedObject — reason still true; mutable_capture.rs:219 v2-raw-heap
SIGABRT-on-suite-state residual). A current-count rerun of shape-test was out of budget for this
audit (shared cargo lock); what is certain is that the CLAUDE.md paragraph no longer describes
the working tree — it describes a two-month-old baseline, and the "tracked as
shape-test-residuals-audit" tracker has no current artifact in docs/ (rg finds only historical
mentions).

### 2.10 Benchmarks + integrity rule — WORKS

- 21 Shape programs with go/node/python/rust counterparts and budget TSVs
  (`ci_jit_node_budget.tsv`, `v8_goal_budget.tsv`), weekly tracking TSVs under
  `benchmarks/tracking/`.
- **Integrity rule enforcement is real and two-layer**: (1) `git log` over `benchmarks/shape/`
  shows exactly two commits — the initial import and 31c17f1f, which only ADDED
  `07b_dot_product.shape` and explicitly cites the rule ("CLAUDE.md forbids modifying existing
  benchmark fixtures; 07b is a new measurement point"); (2) xtask's
  `benchmark_specialization_guard` (main.rs, regex over shape-jit/vm/runtime sources for
  benchmark names like `01_fib|…|benchmark_kernel`) fails workspace-smoke if the compiler ever
  special-cases a benchmark. This is the strongest benchmark-integrity story in the audit set.

### 2.11 Miri gate — EXISTS (narrow by design, manual cadence)

check-miri-provenance.sh runs named test filters under rustup nightly Miri with an explicit
"targeted evidence, not a whole-runtime no-UB proof … Do not summarize a passing run as
UB-free" disclaimer (lines 6-10). Not wired into CI; justfile `miri-provenance` recipe exists.

## 3. Code quality

### 3.1 What the scripts get right

- Every shell gate uses `set -euo pipefail` (verify-merge.sh:37, check-no-dynamic.sh:15,
  differential-gate.sh:2, check-heapkind-wildcards.sh:9, coverage.sh:41, miri:2).
- Exit-code-based verdicts, never `grep -c` of cargo output — and CHECK 11 *actively greps the
  repo's own tooling* for the `cargo check | grep -c` anti-pattern that caused three false
  "workspace clean" declarations (verify-merge.sh:8-25, 471-481). Self-aware gate design.
- check-no-dynamic.sh's baseline format is genuinely well-designed: per-symbol limits that may
  only decrease, "once 0 stays 0 forever", docs-scope exclusion so enforcement text can name the
  patterns (check-no-dynamic.sh:5-13; baseline header rules). Run live: exit 0, all 40 rows at
  their limits (no regression, no unrecorded progress).
- Python gates are "intentionally source-only and cheap" (no cargo), with count-based rather than
  line-number-based baselines (check-typed-opcode-proof-coverage.py:3-12) — robust to refactors.
- The fuzz harness separates `HarnessError` (driver failure) from `Divergence` (successful
  comparison with a bad answer) (lib.rs:41-46) — a distinction many harnesses botch.

### 3.2 Defects and smells

1. **Error-swallowing broke two checks** (P1, §9.3). CHECK 4/7 pipe rg's stderr to /dev/null and
   `|| true` the result, so a *flag-parsing error* (rg ≥13 has no `--include`; `-E` means
   `--encoding`) is indistinguishable from "no matches". The script's own header explains that
   silent tool-output mismatches greenlit three broken merges — then commits the same class of
   bug two checks later. Any gate that swallows stderr must assert the tool ran (e.g. check exit
   code ∈ {0,1}).
2. **gawk-only constructs** in CHECK 5/6/8 (`match($0, /re/, m)` 3-arg form,
   verify-merge.sh:134-148,180-185,384-398). Works here (GNU Awk 5.4.0 confirmed); silently
   extracts nothing on mawk/BSD awk — same silent-no-op failure class as CHECK 4/7. The variant
   extraction was verified live: 36 HeapKind variants parsed.
3. **Redundant `unsafe impl`** (tools/shape-test/src/shape_test.rs:55-56):
   `unsafe impl Send/Sync for CaptureAdapter` whose only field is `Arc<Mutex<Vec<String>>>` —
   already Send+Sync; the impls are dead unsafe that will mask a future non-Send field. Only
   unsafe in the audited territory (2 blocks; zero in shape-fuzz/xtask/scripts).
4. **Complexity hotspots**: verify-merge.sh 605 lines / 15 checks in one file with inline awk
   programs; tools/xtask/src/main.rs 1,296 lines, 10 subcommands, zero `#[test]`s (the
   quality-gate binary is itself untested); shape_test.rs 1,568 lines mixing LSP + runtime +
   snapshot assertions; largest test file stress_methods.rs 1,595 lines.
5. **Panic-happy framework internals**: `Uri::from_file_path("/test.shape").unwrap()`
   (shape_test.rs:189), `tempfile::tempdir().unwrap()` (shape_test.rs:144), `extract_*` panics
   with decent messages — acceptable in test code, but `eval_with_output`'s
   `captured_lines.lock().unwrap()` will poison-cascade across a suite after any panic inside a
   print (observed class: the mutable_capture SIGABRT residual is suite-state-dependent).
6. **Dead recipes**: `verify-phase-2` (an early-phase error-count gate), `verify-phase-5` with
   its stale TODO (justfile:193-197), `ci-test` (unreferenced + would abort, §9.9).
7. **Root-directory debris** in-territory: untracked `d5_perms_open.shape` at repo root
   (`git status --short` = `??`), `test-arena/` reduced to a single ad-hoc file,
   `testdata/` a single CSV. Harmless but drift-y.

### 3.3 Naming/idiom

Naming is consistent and self-describing across the gate corpus (`check-*`, `verify-*`,
`*-gate`). Rust code follows workspace idiom (builder chains, `Result<_, String>` at test
boundaries). The one nomenclature problem is "fuzz": `shape-fuzz`, `nightly-fuzz.yml`,
`differential-fuzz harness` — none of it fuzzes (§2.4); the accurate name is differential
regression replay. Names shape expectations; a reader of `.github/workflows/nightly-fuzz.yml`
reasonably believes input generation is happening nightly.

## 4. Duplication & DRY violations

### 4.1 Five VM-vs-JIT comparison implementations

| Implementation | Language | Corpus | Classification scheme |
|---|---|---|---|
| shape-fuzz `compare_outputs` (tools/shape-fuzz/src/lib.rs) | Rust | 59 seeds | 8-class `Divergence` + 5-level `Signal` |
| vmjit-diff run-diff.mjs (tools/vmjit-diff/run-diff.mjs) | Node | 467 programs | MATCH/DIVERGED/VM_FAIL/JIT_FAIL/TIMEOUT + known-red.json |
| book truth-gate run-book-truth-gate.mjs (shape-web) | Node | 565 snippets | 8 failBuckets + expected-divergence pins |
| smokes/-fallback manual F' harness (tests/smokes/README.md) | bash convention | 13 fixtures | (last, ec) tuple compare |
| ShapeTest `ExecMode::Jit` (shape_test.rs:93-98) | Rust in-process | per-test | assertion equality |

All five re-implement: spawn/execute in two modes, capture (stdout tail, exit code), timeout,
compare, allowlist expected divergence. The allowlist mechanism alone exists in three
incompatible forms: `known-red.json` (vmjit-diff, with "MATCH on a listed id is flagged for
removal" hygiene), `expected-divergence`/`expected-fail` manifest keys (book gate), and — for
shape-fuzz — **nothing** (the nightly corpus's 5 negative seeds just force exit 1 forever,
§9.7). Divergence risk is real and already observable: the three taxonomies disagree about
whether "[jit-fallback] stderr emission" counts (shape-fuzz says no per audit §2.1 — README
"Discipline"; the book gate compares stdout only; the smoke F' harness captures `2>&1` in the
fallback variant and `2>/dev/null` in the smoke variant *by documented design*,
tests/smokes-fallback/README.md:19-21).

**Danger**: a semantics change in what counts as "divergent" (e.g. exit-code-only differences,
extension-load stderr noise) must be updated in five places; three are likely, five are not.

### 4.2 Merge-marker detection ×3

- pre-commit hook staged-diff grep (works; .git/hooks/pre-commit:26)
- verify-merge CHECK 4 tree scan (no-op, §9.3)
- verify-merge CHECK 7 orphan-close-marker scan (no-op, §9.3)

The two broken copies were presumably never missed *because* the working copy exists — which is
exactly how silent no-ops survive.

### 4.3 The Bool-default forbidden shape ×3 encodings

Baseline rows `unwrap_or\(\(0,\s*NativeKind::Bool\)\)` + `unwrap_or\(NativeKind::Bool\)`
(check-no-dynamic-baseline.txt), and the sentinel `no_dynamic.rs` fragment-assembled matcher
(no_dynamic.rs:56-99). Deliberate redundancy (the sentinel survives when the recipe is skipped
— its header says so), but the sentinel covers 1 of 40 rows; the other 39 have no Rust-layer
mirror, so the "survives even when the recipe is skipped" property holds for exactly one
pattern (§6.4).

### 4.4 Lockstep-table knowledge ×2

verify-merge CHECK 6 hardcodes the 4 dispatch-table paths (verify-merge.sh:171-177) and CHECK 6b
hardcodes a 23-variant `jit_lockstep_baseline` array (verify-merge.sh:248-272) — a shell-embedded
copy of facts whose source of truth is `heap_variants.rs` + `ownership.rs`. The stale-baseline
rule ("baseline may only SHRINK", verify-merge.sh:230-234) is well-designed mitigation, but the
list still duplicates code structure into a script.

### 4.5 Rationale-comment triplication

The tarpaulin cadence + install pattern paragraph exists in coverage.sh:1-40, coverage.yml:1-18,
and justfile:204-219; the V8-cage ulimit explanation in justfile:112-121 duplicates
ffi-rebuild §4.8.3; the 83 GB bulk-hang narrative exists in justfile:5-12, shape_test.rs:308-324,
and commit 1fff04da's message. Currently consistent; three-way drift is a matter of time.

## 5. Split-brain analysis

### 5.1 CLAUDE.md/coverage.yml vs ci.yml — enforcement claims that are false

- CLAUDE.md "Mechanical enforcement": "`just check-no-dynamic` … greps for forbidden symbols on
  **every CI run and pre-commit**. Build fails on hit." Reality: ci.yml (all 77 lines) contains
  no check-no-dynamic/verify-merge step; `.git/hooks/pre-commit` (149 LOC) checks stash reflog +
  staged conflict markers only (`grep -n "check-no-dynamic" .git/hooks/pre-commit` → no match).
- coverage.yml:16-18 asserts "The per-commit gates are `just check-clean` +
  `bash scripts/verify-merge.sh` + `bash scripts/check-no-dynamic.sh` in the main CI workflow
  (.github/workflows/ci.yml)" — none of the three appears in ci.yml.
- Compensating control: the sentinel test runs under `cargo test` (verified live: 1 passed,
  §6.4) — but that is 1 of 40 rows.

### 5.2 origin/main vs main — the project-level split brain

1,872 unpushed commits (§2.6). Every externally visible quality artifact (Actions history,
badge state, nightly artifacts) describes v0.3.2. Everything after — GC-on default, W17
snapshot completion, strict-flip enforcement, the entire wave-7 series — exists only in local
verification lore (worktree gate runs described in merge-commit messages). This is the exact
"stale signal presented as live" failure mode the project's own feedback memory warns about,
at infrastructure scale.

### 5.3 justfile internal contradictions

- Tier 2 `test` ENABLES `shape-jit/deep-tests` at default parallelism (justfile:64) while
  `test-all`'s comment 12 lines below says those same tests "SIGILL the JIT code cache under
  default n-cpu parallelism" and must be run with `--test-threads=1` (justfile:76-81). One of
  the two is wrong: either tier 2 is a flaky-by-design "before committing" gate, or the SIGILL
  constraint is stale and test-all should re-include the feature. (CLAUDE.md Known Constraints
  still asserts the SIGILL race is live, root-caused to stdlib JIT-compile caching.)
- `test-deep`'s doc-comment ("Run only deep/soak tests") vs its filter behavior (§9.4).

### 5.4 Doc-vs-code drift inventory (verified individually)

| Claim | Where | Reality |
|---|---|---|
| "~11,800 tests" | CLAUDE.md Commands | grep counts 15,058 `#[test]` + 26 async/case attributes across the workspace (per-crate table §7.1); the runnable no-feature subset is smaller (deep-gated modules, 44 of shape-vm's ignores are inside deep-tests-gated files per classifier output) — the number is plausibly a stale snapshot of a particular tier, not the source-level count |
| "4 `bin/shape-cli/tests/stdlib/simulation.rs` ignored tests" remain | CLAUDE.md Known Constraints ×2 | simulation.rs has 26 test fns, **0** `#[ignore]` — fixed but doc never updated |
| "~23 shape-jit `#[ignore]`'s (`test_jit_width_aware_*`, …)" | CLAUDE.md | 21 attribute-level ignores; named tests exist (core.rs:485-1246, worker.rs:349) — close but stale; classifier's own `REPORTED_LIB_IGNORED_BASELINE` (23) is flagged "not refreshed" in its output |
| shape-vm ignores undocumented | CLAUDE.md silent | 78 attribute-level ignores (72 phase-2c-surface + 5 deleted-v1 + 1 diagnostic) — the largest ignore population in the workspace has no CLAUDE.md mention |
| "sentinel test … not yet wired up; see CLAUDE.md" | justfile:194-197 (`verify-phase-5`) | wired at crates/shape-vm/src/executor/tests/mod.rs:43 since long before; recipe TODO stale |
| "~48 pre-existing shape-test failure clusters" | CLAUDE.md | describes the May baseline; no current artifact backs the number (§2.9) |
| "No `#[ignore]`" test philosophy | tools/shape-test/README.md:8 | 2 ignores exist in the suite (both reasoned) |
| check-ignored classifier EXPECTED_COUNTS shape-vm phase_2c_surface=76 | scripts/check-ignored-test-classification.py:29-35 | source now 72 — gate fails, baseline unmaintained (§9.8) |

### 5.5 Where split-brain is handled WELL

- vmjit-diff `known-red.json` requires audit citations per pin and flags now-matching pins for
  removal — an anti-drift mechanism working as designed (the pb3 nondeterministic pin's
  documentation in baseline-2026-07-05.txt is exemplary).
- CHECK 6b's may-only-shrink baseline (verify-merge.sh:230-234, with stale-baseline failure
  arm at :291-295) — codified drift direction.
- The F'-harness README pins exact expected `(last, ec)` per smoke fixture and forbids the two
  broken capture idioms by name (tests/smokes/README.md:19-28).

## 6. ADR & spec conformance

The testing vertical is unusual: it *is* the enforcement layer for ADR-005/006 and the
Forbidden Patterns contract. Conformance therefore has two directions: (a) does infra code obey
the ADRs; (b) do the enforcement mechanisms the ADRs/CLAUDE.md promise actually exist and run.

### 6.1 ADR-005 §1 single-discriminator / §4 uniform slot ABI — CONFORMS (in-territory)

The framework's only value-carrier code is `CaptureAdapter::print(&mut self, PrintResult) ->
KindedSlot` returning `KindedSlot::none()` (shape_test.rs:44-47) — output-adapter return is a
named GENERIC_CARRIER site under ADR-006 §2.7, and `KindedSlot` is the mandated carrier. No sum
types projecting 1:1 onto HeapKind exist in tools/shape-test, tools/shape-fuzz, or xtask
(grep for enum definitions: only `ExecMode`, `Divergence`, `Signal`, CLI enums — none
HeapKind-shaped).

### 6.2 ADR-006 §2.7.7 forbidden shapes — CONFORMS + ENFORCED

- Live run of check-no-dynamic.sh: exit 0; the Bool-default rows
  (`unwrap_or((0, NativeKind::Bool))`, `unwrap_or(NativeKind::Bool)`,
  `let expected = NativeKind::Bool`) are all pinned at 0 and hold.
- Sentinel `no_bool_default_slot_fabrication` executed live:
  `test executor::tests::no_dynamic::no_bool_default_slot_fabrication ... ok` (1 passed,
  2,625 filtered, 0.05 s). The needle-assembled-from-fragments trick (no_dynamic.rs:13-16) keeps
  the sentinel itself out of the shell gate's plain-text grep — correct self-exclusion.

### 6.3 Forbidden Patterns (CLAUDE.md) — no live code hits; comment references verified benign

The baseline rows with non-zero limits (synthesize_value_word_from_raw=12,
exec_arithmetic_dynamic_fallback=5, capture_as_value=12, nan_box-family=17, …) were spot-checked:
all 12 `synthesize_value_word_from_raw` and all 5 `exec_arithmetic_dynamic_fallback` occurrences
are **comments describing the deleted code by name** (e.g. execution.rs:439 "and the deleted
`synthesize_value_word_from_raw`", helpers.rs:2427 "bulldozed per ADR-006 §2.7.7") — the exact
by-name style the contract mandates. No live definitions. NOT a P0.

### 6.4 CLAUDE.md "Mechanical enforcement" bullet-by-bullet

| Promised mechanism | Status |
|---|---|
| `prove_native_kind` private-constructor ProofGap | out of territory; the *guard over its usage* exists as check-typed-opcode-proof-coverage.py — passes live (`unproven_gap: 0`, 22 prove_native_kind + 519 equivalent-helper + 138 metadata-only sites) but the script is wired into no recipe/workflow |
| check-no-dynamic "on every CI run and pre-commit" | **VIOLATED** — manual/verify-merge-only (§5.1) |
| Sentinel no_dynamic.rs | exists, wired, passes — but mirrors 1/40 baseline rows; the justfile verify-phase-5 TODO claiming it doesn't exist is stale |
| verify-merge "15 checks, exit-code-based, NOT grep -c" | 15 checks confirmed (1,2,3,4,5,6,6b,7,8,9,10,11,12,13,14); exit-code discipline confirmed for cargo checks; **CHECK 4/7 functionally dead** (§9.3); CHECK 14 currently failing on working tree (§9.8) |

### 6.5 ADR-006 §2.7.24 / phase-2d handover rules binding this territory

- 4-table lockstep (handover §0) is enforced by CHECK 6, JIT tables 5/6 by CHECK 6b with the
  frozen 23-variant legacy baseline. Both use the gawk extraction verified live (36 variants).
- Surface-and-stop discipline shows up correctly in ignore texts (e.g. iterator_ops.rs ignores
  cite ADR-006 §2.7.4 per test; v2_handlers/integration_tests.rs:140 cites Q25.A SUPERSEDED) —
  ignored tests carry the ADR paper-trail the handover demands.
- The wildcard guard's audited-baseline catalog (docs/cluster-audits/w83b-heapkind-wildcards.md
  referenced at check-heapkind-wildcards.sh failure output) — the working-tree failure it
  currently reports is *the guard doing its job* on in-progress code
  (`closure_layout_fallback.rs:174`, untracked): the new file routes
  `NativeKind::Ptr(_)` into a nested match that explicitly rejects Closure/Reference/SharedCell/
  IoHandle/Future/TaskGroup captures and pointer-types the rest — a deliberate boundary, but one
  that per the guard's contract must be made exhaustive or added to the audited catalog with a
  wave note before merge.

### 6.6 Benchmark-integrity rule (CLAUDE.md) — CONFORMS with mechanical backing (§2.10)

## 7. Test coverage in-territory

### 7.1 Test-function census (grep `#[test]` per crate, working tree 2026-07-11)

| Crate | `#[test]` | async/case attrs | `#[ignore]` (grep) | attribute-level ignores |
|---|---|---|---|---|
| crates/shape-ast | 608 | 0 | 0 | 0 |
| crates/shape-value | 480 | 1 | 0 | 0 |
| crates/shape-common | **0** | 0 | 0 | 0 |
| crates/shape-runtime | 1,518 | 2 | 0 | 0 |
| crates/shape-vm | 3,109 | 0 | 80 | **78** |
| crates/shape-jit | 849 | 0 | 26 | **21** |
| crates/shape-wire | 60 | 0 | 0 | 0 |
| crates/shape-abi-v1 | 43 | 0 | 0 | 0 |
| crates/shape-macros | **0** | 0 | 0 | 0 |
| crates/shape-diagnostics | 14 | 0 | 0 | 0 |
| crates/shape-viz | 22 | 4 | 0 | 0 |
| bin/shape-cli | 284 | 15 | 21 | ~18-21 |
| tools/shape-lsp | 763 | 4 | 0 | 0 |
| tools/shape-test | 7,260 | 0 | 2 | 2 |
| tools/shape-fuzz | 38 | 0 | 0 | 0 |
| tools/xtask | **0** | 0 | 0 | 0 |
| extensions/python | 8 | 0 | 0 | 0 |
| extensions/typescript | 2 | 0 | 0 | 0 |
| **Total** | **15,058** | 26 | 129 | ~120 |

Cross-check: the live sentinel run showed shape-vm `--lib` (no deep-tests) = 2,626 runnable
tests; the classifier reports 44 of shape-vm's ignores sit inside deep-tests-gated files, i.e.
the default-tier ignored count (56 per its unrefreshed baseline) differs from the source count
(78). The CLAUDE.md "~11,800" figure is between the no-feature runnable set and the source-level
15k count and cites no tier — it should be regenerated with a defined denominator.

### 7.2 Assertion quality (sampled)

- `strings_formatting/fstring*.rs`: exact-string expectations (`expect_string("hello world")`,
  `expect_string("sum is 7")`) — no substring-of-substring laxity.
- Framework asserts carry contextual messages (expected vs found lists in
  `expect_completion`, hover text dumps, shape_test.rs:396-470).
- The suite's stress files (stress_methods.rs 1,595 LOC, stress_dispatch_advanced.rs 1,075 LOC)
  follow one-behavior-per-test granularity rather than mega-tests; `tests/regression/jit.rs`
  (1,252 LOC) pins prior JIT bugs individually.
- shape-fuzz corpus seeds pin exact `(stdout_tail, exit_code)` convergence; the harness
  self-test executes the same program as tests/smokes/s1.shape (README cross-reference), keeping
  fixture meaning aligned across harnesses.

### 7.3 Complete ignored-test inventory with reason-currency assessment

**shape-vm — 78** (classifier buckets, verified live: 72 phase_2c_surface + 5 deleted_v1_path +
1 diagnostic_only):

- `executor/tests/iterator_ops.rs` — ~30 ignores, all "Phase-2c surface: iterator terminal
  materialization requires the host-tier eval/marshal API rebuild (ADR-006 §2.7.4)"
  (iterator_ops.rs:117-395). Reason currency: **holds** (the host-tier rebuild is V3-S5
  ckpt-5/6 territory, still open per CLAUDE.md Known Constraints) — but this is ~30 tests of
  the iterator subsystem dark since May.
- `executor/tests/mod.rs` — ~25 ignores "T1 class-shift surface (ADR-006 §2.7.4) — depends on
  deleted host-tier helpers / typed-Arc accessors" (mod.rs:891-2439) + 1 explicit rewrite-me
  (mod.rs:2052 deleted v1 VMArray alias test). Holds, same caveat.
- `compiler/functions.rs` — 14 ignores "Phase-2c comptime emit surface … deleted host argument
  conversion" (functions.rs:2913-3727). Given WF-3D "comptime flagship verified" merges landed
  (git log 0d1eebae), these reasons deserve re-audit: the comptime emit path they blocked on has
  moved.
- `compiler/comptime_target.rs` (3) + `comptime.rs` (1) — "phase-2c comptime rebuild against
  typed-Arc HeapValue layout (§2.4)". Same re-audit candidate.
- `executor/state_builtins_tests.rs:748`, `v2_handlers/integration_tests.rs:140,154`
  (Array<int>.map/filter through deleted TypedArrayData — cites Q25.A SUPERSEDED; still true),
  `compiler/v2_typed_emission.rs:2782` (diagnostic-only tracer, legitimately permanent).

**shape-jit — 21** (19 deleted_v1_path + 2 process_aborting_extern_c_todo):

- core.rs — 16× "v2: tests deleted BytecodeToIR path; covered by
  mir_compiler::integration_tests" + worker.rs 2× deleted Tier-1 whole-function JIT
  (core.rs:484-1246, worker.rs:348,433). Reason technically true but these are **tests of
  deleted architecture** — they should be deleted, not ignored; each carries maintenance cost
  and pollutes every `--ignored` sweep. core.rs:692 (deleted JitArray/jit_array_info API) same.
- ffi/async_ops.rs:296 + ffi_symbols/simulation/mod.rs:118 — extern-C `todo!()` SIGABRT
  documentation-grade ignores (multi-paragraph, cite ADR-006 §2.7.10/11, name the exact abort
  mechanics). Reasons hold; these two are what makes `--include-ignored` runs abort (§9.9).

**bin/shape-cli — ~21**: 9× distributed_async_cancellation_e2e "timing-sensitive … run
serialized under the supervisor cgroup lane" (holds — timing tests in shared-runner CI are a
legitimate exclusion, though no automated lane runs them now); 7× ffi_e2e "needs built
extension + CPython/V8; run via `just test-ffi`" (holds — and the CI ffi job runs them with
`--include-ignored`, ci.yml:72-73); 1× distributed_snapshot_polyglot_e2e manual SIGINT e2e
(holds). CLAUDE.md's claim that simulation.rs holds 4 ignores is **stale — zero remain** there.

**tools/shape-test — 2**: objects.rs:265 (`len()` on TypedObject — reason verified still true:
design-B removed global `len()`, PHF has no TypedObject `.len()`); mutable_capture.rs:219
(v2-raw-heap string-capture-mutation SIGABRT on accumulated suite state — "correct result
standalone"; holds, and is the one residual suite-state-poisoning known bug in the framework's
domain).

### 7.4 Coverage gaps in the quality infrastructure itself

- **tools/xtask: zero tests** for 1,296 lines of gate logic (perf_regression_gate's rolling-
  median math, vmvalue allowlist diffing, grammar-parity — all unasserted).
- **verify-merge.sh / check-no-dynamic.sh: no self-tests.** The CHECK 4/7 no-op (§9.3) is
  precisely the bug a planted-fixture self-test (`tests/gate-fixtures/merge-marker.rs.txt`)
  would have caught the day rg was upgraded. Contrast: the book gate ships
  run-book-truth-gate.test.mjs + extract-shape-snippets.test.mjs; vmjit-diff ships none but
  writes progress.jsonl for resumability audit.
- **shape-fuzz mutation/minimizer**: unit-tested (14+6 tests) but no integration path exercises
  them — coverage of code that cannot be reached in production is coverage theater.
- Crates with zero tests: shape-common, shape-macros (proc-macros are hard to unit-test but
  trybuild-style coverage is absent), xtask.

### 7.5 Deep-tests feature accounting

Feature declared in 4 Cargo.tomls (shape-vm:58, shape-runtime:89, shape-ast:12, shape-jit:64).
Gate sites: 8 modules in shape-vm executor/tests/mod.rs:74-89 (differential_trusted,
drop_deep_tests, extend_blocks, hashmap_ops, iterator_ops, module_deep_tests, operator_overload,
trusted_edge_cases) + comptime_builtins/functions/lib_tests_parts in shape-vm, module_loader +
lib.rs in shape-runtime, parser/tests in shape-ast, compiler+mir_compiler in shape-jit. The
naming inconsistency (only 2 of 8 shape-vm modules contain "deep") is what breaks `just
test-deep` (§9.4).

## 8. Book/docs vs reality for this vertical

### 8.1 The book truth-gate itself (the vertical's flagship doc-vs-reality mechanism)

- Manifest: 707 extracted fences; 565 runnable; 142 `runnable=false` (measured from
  `.book-truth-gate/snippets/manifest.json`).
- Latest run 2026-07-10T07:13Z against the working tree's release binary: **565/565 pass, all 8
  failure buckets empty** (report.json). Both modes per snippet, plus serve/snapshot-resume
  fixture modes — this satisfies the owner's "every implemented feature … gate-runnable example
  (executes green vm+jit)" hard gate for the gated set.
- Residual honesty gap: 142 fences (20%) are out of the denominator. The 2026-07-05 denominator
  audit found the "240/240 green" claim concealed ~388 failing non-runnable fences; the campaign
  since then moved the gated set to 565 — the remaining 142 need either gating or an explicit
  non-runnable taxonomy in the manifest to prevent the same trap recurring.
- The gate lives in shape-web and is invoked manually / via book campaign waves — it is NOT in
  shape's CI (and could not be: ci.yml checks out only this repo; see §9.10 for the silent-skip
  interaction with book_doctests.rs).

### 8.2 CLAUDE.md testing claims audited

| Claim | Verdict |
|---|---|
| Tier timings ("~5-8s", "~15-30s", "~2-4 min", "~10-15 min") | not re-measured (budget); plausible for the hardware; the tier *shapes* are accurate |
| "`just check-clean` … Every workspace member … covered" | recipe matches claim (justfile:164-165); crate list in comment matches Cargo.toml members |
| "Deep tests are gated behind a deep-tests Cargo feature on shape-vm, shape-runtime, shape-ast" | UNDERCOUNTS: shape-jit also declares it (Cargo.toml:64) and tier-2 enables it |
| "`just test-all` = everything that should currently pass … Pre-existing #[ignore]'s stay ignored" | recipe matches; the enumerated ignore inventory in the same paragraph is stale (§7.3) |
| "Benchmark files must NEVER be modified…" | enforced socially (git history clean, §2.10) and mechanically (xtask guard) |
| "check-no-dynamic … every CI run and pre-commit" | **FALSE** (§5.1) |
| "verify-merge … 15 checks as of 2026-07-05" | count TRUE; 2 checks inert (§9.3) |

### 8.3 README claims

- tools/shape-test/README.md documents the builder API accurately (methods cross-checked against
  shape_test.rs), the 70-area taxonomy matches the directory listing, and the LSP-lockstep
  binding is dated and attributed. Its "No `#[ignore]`" philosophy line conflicts with 2 live
  ignores — trivial but exactly the kind of absolutism that rots.
- tools/shape-fuzz/README.md is accurate about corpus counts (59 verified by find) and honest
  about the mutation-engine gap, but the "Full-corpus nightly cadence" section describes a
  monitoring loop (artifact triage) that has no operator: nothing consumes the artifacts, and
  the job can't fail (§9.7).
- tests/smokes/README.md + smokes-fallback/README.md pin harness idioms and per-fixture expected
  outputs; the s1 fixture re-verified live in both modes (§9.11 transcript) — READMEs match
  reality.
- benchmarks/RESULTS.md is dated 2026-02-19 ("Post-Refactor Phase 3.5", geomean 3.11× vs Node)
  — five months stale as a "Current Results" document; the tracking TSVs under
  benchmarks/tracking/ are the live record instead.

## 9. Bugs & correctness risks found

### 9.1 P0 — CI validates a 6-week-old snapshot; 1,872 commits never saw CI

```
$ git log -1 origin/main --format='%ci %h %s'
2026-05-26 19:50:02 +0200 82f049dd v0.3.2: fix print() to route through OutputAdapter (hosted-embedder regression)
$ git rev-list --count origin/main..main
1872
$ gh run list --limit 30        # (columns: status, conclusion, event, run-id)
completed success schedule 29142563587    # 2026-07-11 nightly-fuzz  — against v0.3.2
completed success schedule 29142558759    # 2026-07-11 coverage      — against v0.3.2
... (all 30 most recent runs: schedule-triggered, success, stale target)
```

Everything merged after 2026-05-26 — including the two-tier GC default flip (ce332ca2), W17
snapshot completion, the strict-typing flip, numeric-conversion model — has zero CI executions.
The green scheduled runs create a false-confidence surface. Severity P0 because it silently
disables every other control in this vertical.

### 9.2 P0 — Releases ship without tests; CI was red at every release tag

```
$ gh run list --workflow=ci.yml --limit 8
completed failure v0.3.2: fix print() ...            push 2026-05-26
completed failure v0.3.1: republish workspace ...    push 2026-05-26
completed failure v0.3.0 release-readiness fixes ... push 2026-05-26
completed failure v0.3.0: bump workspace ...         push 2026-05-26  (x2)
completed failure v0.3 close-summary audit-day ...   push 2026-05-18
completed failure Merge w11-fup-c-print-typed-array  push 2026-05-18
completed failure W2.3 post-merge fix ...            push 2026-05-18
```

Last run's failures (via `gh run view 26465337505 --log-failed`): 9 tests —
`cli::jit_fallback_diagnostic_matrix::*` (6) + `cli::script_execution::test_expand_comptime_*`
(3), `test result: FAILED. 6 passed; 9 failed` in the shape-cli integration binary.
release.yml (`on: push: tags: v*`) contains build/package/publish steps only — no `cargo test`,
no gate dependency on the CI workflow. v0.3.0/1/2 were therefore released from commits whose CI
was red. (The failing matrix is the smokes-fallback fixture suite — the fixtures whose local
harness passes today, §9.11 — suggesting a CI-environment mismatch nobody triaged because red
was normal.)

### 9.3 P1 — verify-merge.sh CHECK 4, CHECK 7, and CHECK 11 are inert (rg flag incompatibility)

verify-merge.sh:115-117 (CHECK 4) and :361-363 (CHECK 7) invoke:

```
rg --no-heading -nE '^<<<<<<<|^=======$|^>>>>>>>' --include='*.rs' ... 2>/dev/null || true
```

ripgrep 15.1.0 (the installed version) parses `-E` as `--encoding` and has no `--include`:

```
$ rg --no-heading -nE '^<<<<<<<|^=======$|^>>>>>>>' --include='*.rs' ... 
rg: error parsing flag -E: grep config error: unknown encoding: ^<<<<<<<|^=======$|^>>>>>>>
```

With stderr discarded and `|| true`, `merge_hits` is always empty → the checks always PASS.
Planted-fixture proof:

```
$ printf '<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n' > scratch/marker.rs
$ hits=$(rg --no-heading -nE '^<<<<<<<|...' --include='*.rs' scratch/ 2>/dev/null || true)
$ echo "planted-hits=[${hits}]"
planted-hits=[]
```

A correct-syntax sweep of the tree found no live markers today (`rg -e '^<<<<<<< ' -e
'^=======$' -e '^>>>>>>> ' -g '*.rs' crates bin tools` → 0), so nothing is currently masked —
but the gate that exists specifically because "8 take-both regex misses" once slipped through
merges (verify-merge.sh:16-25) has been detecting nothing for as long as rg ≥13 has been the
host binary. Fix: `-n -e PAT1 -e PAT2 -g '*.rs'`, and assert rg exit ∈ {0,1}.

CHECK 11 (verify-merge.sh:472-473) — the self-referential tripwire that is supposed to detect
re-introduction of the `cargo check | grep -c` anti-pattern in tooling — has the SAME bug and is
equally inert (re-verified live this session):

```
$ rg --no-heading -nE 'cargo check.*\|.*grep\s+-c' scripts justfile docs Makefile
rg: error parsing flag -E: grep config error: unknown encoding: cargo check.*\|.*grep\s+-c
```

With `2>/dev/null || true`, `anti` is always empty → CHECK 11 always records PASS. The check
guarding against the historical "declared clean while broken" failure mode is itself an instance
of the silent-no-op class it was written to prevent. Same fix shape as CHECK 4/7.

### 9.4 P1 — `just test-deep` filters out most deep tests

justfile:93 runs `cargo test ... --features ...deep-tests -- deep --include-ignored`. The
trailing `deep` is a libtest name substring filter. Deep-gated shape-vm modules
(executor/tests/mod.rs:74-89): differential_trusted, drop_deep_tests, extend_blocks,
hashmap_ops, iterator_ops, module_deep_tests, operator_overload, trusted_edge_cases — only
`drop_deep_tests` and `module_deep_tests` contain "deep" in their test paths. `hashmap_ops::*`,
`iterator_ops::*`, `operator_overload::*`, `extend_blocks::*`, `differential_trusted::*`,
`trusted_edge_cases::*`, shape-jit's `mir_compiler::integration_tests`, `v2_array_tests`,
`a1d2_tests`, `a1e_tests`, shape-ast's and shape-runtime's gated suites all silently don't run
under the recipe named "Run only deep/soak tests". Additionally the recipe enables
shape-jit/deep-tests but omits the `--test-threads=1` that justfile:76-81 says is required to
avoid the SIGILL race — so the subset it *does* run includes the known-racy configuration.
(Tier 2 `just test` covers the gated modules correctly since it passes no name filter; the bug
is confined to test-deep, plus the tier-2 SIGILL contradiction of §5.3.)

### 9.5 P1 — Tier 2 (`just test`, the documented pre-commit tier) enables the SIGILL-racy config

justfile:64 enables `shape-jit/deep-tests` at default parallelism. justfile:76-81 and CLAUDE.md
Known Constraints assert those tests "SIGILL the JIT code cache under default n-cpu
parallelism". If the constraint is still live, the recommended before-commit tier is flaky by
construction; if it was fixed, three documentation sites are stale. Either way one artifact is
wrong; not resolved here (running it would violate the audit's cargo budget).

### 9.6 P1 — Coverage gate cannot fail

- scripts/coverage.sh:229-237: "This wrapper propagates tarpaulin's exit code.
  Phase-4b-batch-merge gate wires `--fail-under 99` once the per-feature exception registry is
  wired" — i.e. the threshold is not wired; measurement completion ⇒ exit 0.
- .github/workflows/coverage.yml:66-83: both measurement steps and the dead-code step carry
  `continue-on-error: true`, so even a tarpaulin crash cannot fail the job.
- Result: the ratified acceptance criterion "test coverage ≥99% per-feature with documented
  exceptions" (coverage.sh:7-8, user 2026-05-18) has never had an enforcement point. The 1h15m
  nightly runs (coverage job durations in gh run list) produce artifacts nobody is recorded as
  consuming.

### 9.7 P1 — nightly-fuzz is unfailable and allowlist-free

nightly-fuzz.yml:44-56: the harness step is `continue-on-error: true` because "the corpus
carries 5 negative-class seeds … this is expected, not a CI failure". shape-fuzz has no
known-red mechanism (unlike vmjit-diff), so expected-negative and NEW divergences both surface
only as exit 1 + findings artifacts. Consequences: (a) a new VM/JIT divergence introduced
tomorrow changes nothing visible — same green job, artifact nobody opens; (b) even the artifact
is computed against stale origin/main (§9.1). The fix is mechanical: port known-red.json
semantics into shape-fuzz (pin the 5 negative seeds; exit 1 only on unpinned divergence; drop
continue-on-error).

### 9.8 P1 — Two gates fail on the working tree; two gates are wired nowhere

```
$ python3 scripts/check-ignored-test-classification.py ; echo EXIT=$?
Ignored test classification check FAILED:
  - shape-vm classification drift: got {'phase_2c_surface': 72, ...}, expected {'phase_2c_surface': 76, ...}
EXIT=1
$ bash scripts/check-heapkind-wildcards.sh ; echo EXIT=$?
FAILED: new HeapKind wildcard dispatch patterns found.
ptr-wildcard-arm  crates/shape-vm/src/bytecode/closure_layout_fallback.rs:174  NativeKind::StringV2 | NativeKind::DecimalV2 | NativeKind::Ptr(_) => match kind {
EXIT=1
```

- The wildcard hit is in an **untracked** file (`git status --short` → `??`) belonging to
  in-progress W-work — the guard is functioning; the risk is that verify-merge CHECK 14 (which
  wraps this script) hard-fails right now, and if the in-flight branch "fixes" that by adding a
  baseline entry rather than exhaustive arms, that becomes the W-series defection shape the
  guard exists to catch. Flagged for the merge reviewer.
- The classification drift (76→72: four phase-2c-surface ignores removed without baseline
  update) is unmaintained-gate rot: since check-ignored-test-classification.py is referenced by
  **no** justfile recipe, workflow, or verify-merge check (`rg` across all three → no hits;
  same for check-typed-opcode-proof-coverage.py), nothing ever runs it, so its baseline decays
  invisibly. A gate wired nowhere is documentation wearing a gate costume.

### 9.9 P2 — `just ci-test` would abort if ever used

justfile:133-134 runs `cargo test --workspace ... -- --include-ignored`. Two shape-jit ignored
tests document that running them aborts the whole test process: ffi/async_ops.rs:296 ("extern C
can't unwind, so the todo!() body aborts the test process (SIGABRT)") and
ffi_symbols/simulation/mod.rs:118 (same). `--include-ignored` runs them ⇒ the shape-jit test
binary dies mid-run. No workflow references `just ci-test` (ci.yml uses raw cargo commands), so
this is a landmine recipe rather than a live breakage — but it contradicts its own name.

### 9.10 P2 — book_doctests.rs silently greens without the book checkout

tools/shape-test/tests/book_doctests.rs:17-21:

```rust
let snippets = collect_book_snippets(&snippets_dir());
if snippets.is_empty() {
    // Book snippets dir doesn't exist yet — nothing to test
    return;
}
```

`snippets_dir()` resolves to `../../../shape-web/book/snippets` — absent in the GitHub Actions
checkout (ci.yml checks out only this repo), absent in any standalone clone. The test passes in
exactly the environments where it verifies nothing. Given the owner's book-gate HARD GATE
memory, this should at minimum distinguish "dir missing" (skip with visible marker or
env-gated failure) from "0 snippets found".

### 9.11 Verification transcripts for things that DO work (control group)

Curated differential gate, working-tree binaries (debug shape built 2026-07-11 10:04):

```
$ bash scripts/differential-gate.sh
differential-gate: running 13 curated VM-vs-JIT seeds
.../a01_add_int.shape :: convergent ... (all 13 seeds)
.../w01_module_read.shape :: convergent (vm="100" ec=Some(0) | jit="100" ec=Some(0))
differential-gate: curated VM-vs-JIT subset converged
EXIT=0
```

Smoke fixtures per the F' harness (tests/smokes/README.md):

```
s1 vm:  last=4950 ec=0
s1 jit: last=4950 ec=0
f1 jit: ec=0 fallback-lines=1 last=100     # [jit-fallback] emitted once, falls through to VM, correct result
```

Sentinel test (narrow cargo invocation):

```
$ cargo test -p shape-vm --lib no_dynamic
test executor::tests::no_dynamic::no_bool_default_slot_fabrication ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2625 filtered out; finished in 0.05s
```

check-no-dynamic.sh: exit 0, zero drift rows. check-typed-opcode-proof-coverage.py: exit 0,
`unproven_gap: 0`.

## 10. What is done well

1. **Exit-code discipline as a learned institution.** verify-merge.sh exists because grep-count
   verification greenlit three broken merges; the script both fixes that and installs a
   tripwire against its reintroduction (CHECK 11). Institutionalizing your own postmortems into
   executable checks is rare and valuable.
2. **The frozen-baseline monotonic guard** (check-no-dynamic.sh + baseline). Per-symbol limits
   that can only shrink, doc-scope exclusion so enforcement text can name patterns, and a
   Rust-layer sentinel with a fragment-assembled needle so the sentinel doesn't trip the grep
   (no_dynamic.rs:13-16). This is a thoughtful, layered anti-defection design even where wiring
   gaps undercut it.
3. **known-red.json hygiene** (tools/vmjit-diff). Every pin needs an audit citation; MATCHes on
   pinned ids are flagged for removal; the pb3 nondeterministic-flap pin documents *why a single
   now-matching sample must not remove the pin* (baseline-2026-07-05.txt) — precise allowlist
   epistemology, the exact opposite of a dumping ground.
4. **Benchmark integrity done twice**: a social rule (CLAUDE.md), a clean git history for
   `benchmarks/shape/` (one additive commit that itself cites the rule), and a mechanical
   `benchmark_specialization_guard` scanning compiler sources for benchmark-name special-casing.
   The combination makes the failure mode (compiler learns the benchmarks) genuinely hard.
5. **Two-layer runaway containment** for the bulk-hang class: root-cause fix in-process
   (per-buffer alloc ceiling on the doubling-realloc paths, commit 1fff04da) plus a coarse
   `ulimit -v` backstop in every test recipe (justfile:5-12), each documented with the geometric-
   vs-linear growth argument. The lane went diagnosis-first as mandated, and the diagnosis is
   written where the next engineer will look (shape_test.rs:308-324).
6. **Reasoned ignores.** Nearly every `#[ignore]` in the workspace carries a structured reason
   with an ADR/wave citation; two even document their own abort mechanics in paragraph form
   (async_ops.rs:296). A classifier script buckets them by cause. Most projects have bare
   `#[ignore]`; this one has an ignore *ontology* (even if its baseline is currently stale).
7. **In-repo, pinned smoke fixtures** with a binding harness convention that names forbidden
   capture idioms (`ec=$?` after a pipe) — tests/smokes/README.md retired a documented
   fixture-drift failure class.
8. **The ffi CI job as audit remediation** (ci.yml:39-46): the 2026-07-04 finding "foreign e2e
   gated out of every tier" was answered with a tier that builds real extensions and runs the
   full matrix with `--include-ignored`, plus a PANIC-not-skip harness contract for missing
   extensions. The right shape of fix — undermined only by the push freeze.
9. **Resource-limited tests by default** (shape_test.rs:216-223): every integration test runs
   under instruction/memory/wall caps, so an infinite loop is a test failure, not a hung suite.
10. **The book truth-gate's failure taxonomy** (8 buckets incl. expected-fail-succeeded /
    expected-fail-missing) and its fixture modes (serve, serve-snapshot-resume,
    local-snapshot-resume) — a doc gate that can express "this example SHOULD fail with this
    diagnostic" is well past typical doctest capability.

## 11. What is done poorly / tech debt

1. **Enforcement-point rot.** Four gates exist that nothing invokes (check-ignored-test-
   classification.py, check-typed-opcode-proof-coverage.py, `just ci-test`,
   compare_cargo_failures.sh is manual-by-design but undocumented in CLAUDE.md); two more
   invoke but can't fail (coverage.yml, nightly-fuzz.yml); two more run but detect nothing
   (CHECK 4/7). The ratio of gate-LOC to enforced-gate-LOC is the vertical's core debt.
2. **The push freeze** (§9.1) converted the entire GitHub Actions layer into a stale-signal
   generator. Whatever the reason for not pushing (open-core split, secrecy, churn), the cost
   is that "CI green" now means nothing and everyone has adapted to local-only verification
   lore in merge-commit messages.
3. **Red-CI normalization.** Eight consecutive failing CI runs before the freeze, releases
   tagged on red, and a benchmarks workflow that stayed green alongside — a classic broken-
   windows dynamic. The failing tests (jit_fallback_diagnostic_matrix) pass locally today,
   which means the CI-vs-local divergence was an environment issue nobody owned.
4. **Ignored-test debt concentration**: 78 ignores in shape-vm, 72 of them one bucket
   (phase-2c host-tier rebuild) frozen since May. That's a whole subsystem (iterators,
   comptime emit directives, typed-annotation accessors) whose regression alarm is off. The
   19 deleted-BytecodeToIR jit ignores should be deletions, not ignores.
5. **Fuzzing that isn't.** 1,039 LOC of mutation+minimizer machinery, unit-tested, never
   callable. Either wire `--seed`/`--mutations-per-seed` into the CLI and nightly job, or
   delete and rename the crate to shape-diff — the current state misleads.
6. **Framework/monolith hotspots**: shape_test.rs (1,568) mixes four assertion domains;
   xtask main.rs (1,296, untested); verify-merge.sh (605) embeds four awk programs. All three
   are past the point where extraction pays.
7. **Manifest-denominator residue**: 142 non-runnable book fences remain ungated with no
   taxonomy explaining each exclusion (the prior denominator trap shrank but its mechanism —
   quiet exclusion — persists).
8. **Docs describing the May world**: CLAUDE.md's ignore inventory, the ~48-failure paragraph,
   the "~11,800 tests" figure, verify-phase-5's TODO, RESULTS.md "Current Results" from
   February. This vertical's docs decay faster than its code because the code has gates and
   the docs don't (ironically, CHECK 13 gates one narrow doc-drift class — colon-return-types —
   while whole stale paragraphs sit un-gated).
9. **Single-point-of-failure knowledge**: the supervisor cgroup lane for timing-sensitive
   distributed tests (9 ignores reference it) exists only as ignore-reason folklore; no script
   under scripts/ or ci/ defines that lane.

## 12. Prioritized recommendations

### P0 (do first; hours-to-days each)

1. **Re-establish a live CI target.** Push main (or stand up an internal runner/mirror if the
   repo must stay private-ahead). Then make ci.yml's `rust` job blocking-by-culture again:
   triage the 9 environment-divergent failures (jit_fallback matrix + expand_comptime) on the
   runner, fix or quarantine-with-issue. Effort: 1-2 days incl. triage. Until this lands, no
   other CI fix has any effect.
2. **Gate releases on tests.** release.yml: add a needs-CI job or an explicit
   `cargo test --workspace` before packaging. Effort: <1 hour.
3. **Fix verify-merge CHECK 4/7** (`-e` patterns + `-g` globs; assert rg exit ∈ {0,1}) and add a
   planted-fixture self-test for the gate (a `tests/gate-fixtures/` file with markers that the
   script must detect in a self-check mode). Effort: 1-2 hours.

### P1 (this sprint)

4. **Wire the orphan gates**: add check-ignored-test-classification.py and
   check-typed-opcode-proof-coverage.py to verify-merge.sh (as CHECK 15/16) or ci.yml; refresh
   the classifier baseline (76→72) in the same commit. Add check-no-dynamic to ci.yml and the
   pre-commit hook, or edit CLAUDE.md to stop claiming it. Effort: half a day.
5. **Fix `just test-deep`**: drop the `deep` name filter (the feature flags already select the
   deep set), add `--test-threads=1` for the shape-jit portion, or split into per-crate
   invocations. Resolve the tier-2 SIGILL contradiction one way or the other and update the
   three doc sites. Effort: half a day incl. one verification run.
6. **Make nightly-fuzz failable**: add a known-red allowlist to shape-fuzz (reuse the
   vmjit-diff JSON schema), pin the 5 negative seeds, remove `continue-on-error`. Effort: 1 day.
7. **Wire `--fail-under` into coverage.sh** at whatever honest threshold current measurement
   supports (even 60% enforced beats 99% aspirational), and drop `continue-on-error` from the
   measurement steps (keep it only on artifact upload). Effort: hours, plus one full nightly to
   calibrate.
8. **Ship or delete the mutation engine.** Wiring `mutate_seed` behind
   `shape-fuzz run --mutations-per-seed N --seed S` is ~1 day given the library exists and is
   tested; nightly gets `--mutations-per-seed 10`. Otherwise rename the crate and workflow to
   stop claiming fuzz coverage.

### P2 (backlog, bounded)

9. Delete the 19 deleted-BytecodeToIR shape-jit ignored tests (their ignore text already names
   the replacement coverage); rewrite mod.rs:2052 as the v2 mutation/share test it asks for.
10. Re-audit the 14 comptime-emit ignores in compiler/functions.rs against the post-WF-3D
    comptime surface — the blocking dependency has moved.
11. book_doctests.rs: fail (or visibly skip via env flag) when the snippets dir is absent;
    document that the canonical gate lives in shape-web.
12. Regenerate the CLAUDE.md testing paragraphs from source (test census, ignore inventory,
    tier description) and date them; delete the stale verify-phase-5 TODO; retire or date
    RESULTS.md.
13. Add a minimal `#[test]` layer to xtask's guards (fixture-driven: a fake criterion tree for
    perf_regression_gate, a fixture source tree for the specialization guard). Effort: 1-2 days.
14. Portability-harden verify-merge's awk programs (gawk-detect or rewrite extraction in
    python3, which the repo already requires for other gates).
15. Give the 142 non-runnable book fences a manifest taxonomy (`skip-reason:` per fence) so the
    excluded denominator is auditable — the same discipline known-red.json already applies to
    divergences.

---

## Appendix A — verify-merge.sh: per-check status as audited

| Check | What it verifies | Mechanism | Audit verdict |
|---|---|---|---|
| CHECK 1 | `cargo check --workspace --lib` exit 0 | exit code | sound design; not re-run (cargo budget) |
| CHECK 2 | `cargo check --workspace --all-targets` (canonical gate) | exit code; skipped in `--fast` | sound; matches justfile check-clean target set |
| CHECK 3 | check-no-dynamic.sh | 40-row frozen baseline | **RAN LIVE: PASS** (exit 0, zero drift) |
| CHECK 4 | residual merge markers tree-wide | rg scan | **INERT** — rg flag error swallowed; planted marker undetected (§9.3) |
| CHECK 5 | HeapKind ordinal collisions | gawk parse of heap_variants.rs | extraction verified live (36 variants parsed); gawk-only 3-arg `match()` |
| CHECK 6 | 4-table HeapKind lockstep | per-variant rg over 4 hardcoded paths | sound; path list duplicated from code structure (§4.4) |
| CHECK 6b | JIT retain/release lockstep (tables 5/6) | arm-count + FFI-symbol resolution + 23-variant shrink-only baseline | best-designed check in the file; stale-baseline arm turns silent progress into failure |
| CHECK 7 | orphan `>>>>>>>` close-markers | rg scan | **INERT** — same flag bug as CHECK 4 |
| CHECK 8 | dispatch-table missing-brace stitches | gawk state machine over the 4 tables | plausible; untested against a planted fixture |
| CHECK 9 | duplicate `use` lines | awk count per file over `rg --files` | sound |
| CHECK 10 | receiver-recovery suspicious patterns | multiline rg heuristic | explicitly review-not-fail; records pass even when suspicious (by design) |
| CHECK 11 | `cargo check \| grep -c` anti-pattern in tooling | rg over scripts/justfile/docs | **INERT** — same `-nE` flag bug as CHECK 4/7; error swallowed, always passes (§9.3, re-verified live) |
| CHECK 12 | JIT-private HK_* ordinal guard | gawk over value_ffi.rs (alias or ≥256 rule) | sound |
| CHECK 13 | colon-return-type doc drift | rg PCRE over living docs + stdlib + LSP text | sound; narrow but real doc gate |
| CHECK 14 | HeapKind wildcard guard | check-heapkind-wildcards.sh | **RAN LIVE: FAIL** on untracked closure_layout_fallback.rs:174 (§9.8) |

Net: of 15 checks, 3 are inert (4, 7, 11), 1 currently fails on the working tree (14), 11 are
sound as designed. The file has no self-test fixture; the inert trio proves it needs one.

## Appendix B — check-no-dynamic baseline rows (docs/check-no-dynamic-baseline.txt, 40 rows)

Grouped by function, with the live-run result (exit 0 = every row exactly at limit):

- **Deletion-progress counters (non-zero limits, all comment-references verified §6.3):**
  synthesize_value_word_from_raw=12, normalize_persisted_for_slot=1,
  exec_arithmetic_dynamic_fallback=5, last_emitted_native_kind=8, capture_as_value=12,
  nan_box/NanBox/NanTag=17.
- **Hard-zero deleted symbols:** last_program_return_kind, exec_comparison_dynamic_fallback,
  ConvertBoolToString, rebox_native_bits, SlotKind::(Dynamic|Unknown).
- **Rename-family tripwires (all 0):** "ValueBits shim", "FFI-boundary bridge", "boundary
  translation", "host-boundary normalization", "decode hop", "tag normalization",
  "compatibility layer", "dynamic-fallback retained", tag-decode-{bridge,probe,helper,hop,
  translator,adapter} family, decoder/decode-bridge family, synthesis/tag-bridge family,
  value-call/closure-callback/frame-setup/callee-kind/capture-injection family,
  call_value_{legacy,raw_u64}, dispatch_value_call_handler_raw, call_value_with_u64_slice.
- **ADR-006 §2.7.7/§2.7.8 forbidden shapes (all 0):** `unwrap_or((0, NativeKind::Bool))`,
  `unwrap_or(NativeKind::Bool)`, `let expected = NativeKind::Bool` (W17 Stage-0).
- **R6 carrier-UB regexes (0):** Arc ptr-ops on `_new`-allocated TypedObjectStorage /
  TraitObjectStorage carriers.

Design note: the note column makes each row self-documenting, and the "adding a forbidden
pattern is fine — start it at the current count" rule (baseline header) makes the guard cheap
to extend. The scheme's one blind spot: it counts *occurrences*, so moving a comment mention
between files is invisible (good), but so is replacing a comment with live code at the same
count (bad — a live `synthesize_value_word_from_raw` definition replacing one comment mention
would not trip the gate). Rows for supposed-deleted symbols should distinguish
definition-shaped matches (`fn synthesize_…`) at limit 0 from mention counts.

## Appendix C — deep-tests gate map (all `feature = "deep-tests"` sites)

| Crate | Gated sites | Contains "deep" in test path? |
|---|---|---|
| shape-vm | executor/tests/mod.rs:74-89 → differential_trusted, drop_deep_tests, extend_blocks, hashmap_ops, iterator_ops, module_deep_tests, operator_overload, trusted_edge_cases | only drop_deep_tests, module_deep_tests |
| shape-vm | lib_tests_parts/{module_qualified_type,typed_object_regression}_tests.rs:5, compiler/comptime_builtins.rs:1382, compiler/functions.rs:2765, executor/tests/test_utils.rs | no |
| shape-runtime | lib.rs:69, module_loader/mod.rs:9 | no |
| shape-ast | parser/tests/mod.rs | no |
| shape-jit | compiler/mod.rs:19,22,25 (a1d2/a1e), mir_compiler/mod.rs:43 (integration_tests) | no |

This table is why `just test-deep`'s `-- deep` filter (justfile:93) contradicts its "Run only
deep/soak tests" doc-comment (§9.4): the filter selects by name substring while the gating is
by feature, and the two disagree for ~80% of the gated surface.

## Appendix D — GitHub Actions inventory vs reality (queried via gh, 2026-07-11)

| Workflow | Trigger | Last run vs today | Can it fail on regression? |
|---|---|---|---|
| CI (ci.yml) | push/PR main | 2026-05-26, **failure** (9 tests) | yes — and it did, repeatedly, without consequence |
| Performance Benchmarks | push/PR main | 2026-05-26, success | regression check runs only `if: pull_request` (benchmarks.yml:36-39); pushes just save a baseline; perf-regression-gate step is `continue-on-error` |
| coverage | nightly + path-filtered push | 2026-07-11, success (78-82 min) | no — all steps continue-on-error + no --fail-under (§9.6) |
| nightly-fuzz-differential | nightly + dispatch | 2026-07-11, success (~9-15 min) | no — continue-on-error because corpus has 5 expected-negative seeds (§9.7) |
| Release | tag v* | v0.3.2 era | no tests at all (§9.2) |
| vscode-publish | (not audited in depth) | — | — |

Additional wiring gap: `ci/bench-gate.sh` (the documented criterion regression gate with
warmup + threshold + trusted-vs-guarded assertions) is referenced by **nothing** — benchmarks.yml
calls `cargo bench` + `ci/check_regressions.py` directly (`rg bench-gate` across workflows,
justfile, scripts → zero hits). Two parallel bench-gate implementations, one orphaned.

## Appendix E — harness comparison matrix (the five VM-vs-JIT mechanisms, detail)

| | shape-fuzz | vmjit-diff | book truth-gate | smokes F' | ShapeTest::with_jit |
|---|---|---|---|---|---|
| Process model | subprocess ×2 | subprocess ×2 | subprocess ×2 (+serve fixture procs) | subprocess ×2 (manual) | in-process |
| Timeout | 30 s default | 10 s | 15 s | 30 s convention | resource-limit caps |
| Compared | stdout tail + exit | stdout + exit | stdout + exit (+expected pins) | (last line, ec) | assertion values |
| stderr handling | piped to null; `[jit-fallback]` not a divergence | captured for report | captured tail for serve diagnostics | `2>/dev/null` smoke / `2>&1` fallback variants | n/a |
| Allowlist | **none** | known-red.json (5 pins, citation-required) | manifest expected-divergence/expected-fail keys | per-fixture expected table in README | n/a |
| Resume | no | progress.jsonl, resume-by-default, `--fresh` (run-diff.mjs:39-46,194-205) | no | no | n/a |
| Corpus | 59 seeds | 467 programs | 565 snippets | 13 fixtures | per-test |

The resume design in run-diff.mjs is worth copying into shape-fuzz if the corpora ever merge:
allowlist status "recomputed at report time so allowlist edits between resumed calls" apply
retroactively (run-diff.mjs:196-197) — that is the correct ordering (pin edits should not
require re-execution).

## Appendix F — supplementary findings not sized as P-items

- **book_policy.rs shares the silent-skip class** with book_doctests.rs: all three of its tests
  (`book_summary_links_resolve` — explicit skip for Astro; `book_md_links_and_includes_resolve`
  and `book_shape_examples_use_current_syntax` — vacuous over empty file lists) pass trivially
  when `../shape-web/book` is absent, i.e. in CI.
- **create-agent-worktree.sh + install-git-stash-wrapper.sh** are process-enforcement tooling
  (worktree + PATH-wrapper + direnv shim to mechanically block `git stash` in dispatched agent
  worktrees, with a documented compliance history: 12 violations before mechanization). They
  work as designed per their own Q3-verification notes; they are governance, not test infra,
  but live in scripts/ and are counted in this vertical's LOC.
- **compare_cargo_failures.sh** implements exactly the blast-radius differential methodology the
  project's feedback memory mandates for behavior-changing fixes (extract `failures:` sets from
  two logs, diff by name). It is referenced by zero docs in CLAUDE.md — institutional knowledge
  at risk of being re-invented.
- **The pre-commit conflict-marker guard** (.git/hooks/pre-commit:26) scans only the *staged
  diff* (`git diff --cached`), so markers already committed on a branch being merged locally
  bypass it — which is precisely the case verify-merge CHECK 4 was supposed to cover (§9.3).
  Until CHECK 4 is fixed, the project has no post-commit marker detection at all.
- **testdata/ vs fixtures sprawl**: fixture roots now include tests/smokes, tests/smokes-fallback,
  tools/shape-fuzz/tests/corpus, tools/shape-fuzz/tests/smoke-self-test, tools/vmjit-diff/corpus,
  tools/vmjit-diff/synthetic, benchmarks/shape, testdata/, test-arena/ — nine locations with
  four different README conventions. Consolidation is not urgent; an index in
  docs/codebase-index would prevent the next auditor's scavenger hunt.

---

*Report generated 2026-07-11 by vertical auditor 15 (testing & quality infrastructure). All
file:line citations refer to the working tree at commit ce332ca2 + uncommitted changes. Live
command transcripts were captured during the audit session; cargo usage: 2 narrow invocations
(one failed-cd retry + one sentinel test run), within budget.*

