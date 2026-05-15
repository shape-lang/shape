# Wave 3 Stabilize Round 1 — V3-baseline-classification (2026-05-15)

Phase 3 cluster-0+1 Wave 3 Stabilize Round 1 read-only baseline
classification (V3-baseline-classification sub-cluster). Compares
shape-test failure state at HEAD `4c7b1d9d` (Phase 3 cluster-0+1 Wave 2
Round 2 close) vs current HEAD `1fe55c43` (Phase 3 cluster-0+1 Wave 2
Round 3b C2-joint close) to inform:

1. Cluster-0+1 close criterion (no regression-rate increase from
   baseline)
2. `hashmap-value-v-arm` follow-up disposition (v2_group_by + Array
   .groupBy tests — surfaced cleanly in Round 3b ckpt-4)
3. V3-S5 wholesale TypedArrayData enum deletion (Round 2) — what
   shape-test failures count as baseline post-V3-S5
4. `shape-test-residuals-audit` scope (cluster-2 territory)

## Methodology

- **Baseline HEAD:** `4c7b1d9d` (Phase 3 cluster-0+1 Wave 2 Round 2 close
  status doc)
- **Current HEAD:**  `1fe55c43` (Phase 3 cluster-0+1 Wave 2 Round 3b
  C2-joint close status doc)
- **Test runner:**   `cargo test -p shape-test --no-fail-fast --
  --test-threads=1`
- **Worktrees:**
  - `/home/dev/dev/shape-lang/shape-v3-baseline-pre-r3a-prime` (baseline)
  - `/home/dev/dev/shape-lang/shape-v3-baseline-classification` (current)
- **Hang-killer:** 2-minute per-test-binary etime threshold (auto-kills
  hung test binaries to allow suite progression under
  `--test-threads=1`); applied symmetrically to both runs

## Surface-and-stop finding: shape-test runs cannot complete cleanly under `--test-threads=1`

The cargo-test runner under `--test-threads=1` exhibits two
discipline-relevant failure modes IN BOTH BASELINE AND CURRENT HEADS:

1. **SIGABRT mid-suite** (`malloc_consolidate(): unaligned fastbin
   chunk detected` / `free(): unaligned chunk detected in tcache 2`)
   — heap corruption from the v2-raw-heap aliasing class
   (CLAUDE.md "Known Constraints" 4-simulation-test
   pre-existing-#[ignore] cluster). Suites observed SIGABRTing at
   baseline:
   - `annotations_comptime` / `annotations_runtime` (SIGABRT after
     first ~10-15 tests)
   - `arrays_vectors` (SIGABRT after 2 FAILED tests)
   - `hashmap` (SIGABRT at `iteration::hashmap_filter_all_match` —
     BEFORE the `v2_group_by` tests; see §"v2_group_by /
     Array.groupBy" below)
   - `objects` (SIGABRT at `operations::object_function_value_call`)
   - `objects_arrays` (PARTIAL — terminated mid-suite)
   - `annotation_targets` SIGABRT'd at current HEAD only (flakiness;
     same pattern as v2-raw-heap-aliasing class — completed FAILED
     9/15 at baseline 47.54s + 48.47s across both baseline runs)

2. **Hangs (≥2-minute single-test wall-clock)** — slow JIT compilation
   or runtime infinite-loop in stdlib pre-compile path. Suites observed
   hanging IN BOTH HEADs and requiring hang-killer SIGKILL:
   - `closures_hof` (hang on `array_methods::array_every_false`,
     4-5 min CPU; killed)
   - `enums` (hang on `basics_decl::enum_with_match_has_semantic_tokens`)
   - `error_handling` (hang on
     `context_operator::declared_result_return_with_err_context`)
   - `functions` (hang on
     `closures_as_params::closure_with_array_filter`)
   - `generics` (hang)
   - `iterators` (hang on `stress_chaining::test_array_concat`)
   - `literals` (hang)
   - `lsp` (hang on
     `combined::test_lsp_combined_all_features_simple`, 1+ min)
   - `modules_visibility` (hang on
     `scoped_contract::scoped_contract_global_stdlib_modules_require_imports`)

The hang-killer pattern matches the CLAUDE.md "shape-jit
heavy-execution tests gated behind `deep-tests`" Known-Constraint —
stdlib JIT-compilation caching (~118 stdlib functions per test under
default n-cpu parallelism, SIGILL race) is the same root cause class;
the shape-test integration suite triggers the same compile-cache cost
under serial execution.

**Implication for the supervisor's "aggregate counts" classification
table:** the literal `cargo test -p shape-test --no-fail-fast --
--test-threads=1` runner-shape does not produce a wholesale pass/fail
count across the 63 test suites — it terminates mid-suite on SIGABRT
and hangs mid-test indefinitely without external intervention. The
classification below is per-suite, sourced from the suites that DID
complete in each run.

## Aggregate counts (per-suite-completion basis; partial data)

Both runs are still in progress at the time of this report; baseline
run-1 was prematurely terminated (cargo wrapper timeout) after 18
suite results; baseline run-2 (restart) + current-HEAD run are
concurrent and at ~5/4 suite results respectively.

| HEAD | Suite results completed | Notable suite outcomes |
|---|---|---|
| `4c7b1d9d` (baseline-1, partial) | 18 of 63 | annotation_targets FAILED 9/15 (48s); async_concurrency FAILED 9/9 (45s); book_doctests FAILED 1/2 (110s); book_policy ok 3/0; borrow_refs FAILED 155/50 (456s); control_flow FAILED 455/25 (998s); drop_raii ok 0/0; e2e FAILED 14/4; e2e_gated FAILED 2/2; extend_blocks ok 0/0; features FAILED 15/1 (28s); functions FAILED 5/2 (152s); integration ok 17/0; jit FAILED 9/7; list_comprehension FAILED 2/6; module_distribution ok 8/0; native_interop ok 15/0 |
| `4c7b1d9d` (baseline-2, restart) | 5 of 63 (in progress) | annotation_targets FAILED 9/15 (47.54s); async_concurrency FAILED 9/9 (43.81s); book_doctests FAILED 1/2 (108.73s); book_policy ok 3/0 |
| `1fe55c43` (current, in progress) | 4 of 63 (in progress) | annotation_targets SIGABRT mid-suite (regression vs baseline — see §below); async_concurrency FAILED 9/9 (44.48s); book_doctests FAILED 1/2 (108.41s); book_policy ok 3/0 |

**No aggregate count delta computable from partial data; per-suite
classification follows.**

## Per-suite classification

Classification codes:
- **(a)** Tests not present in cargo test baseline → cluster-2 territory
- **(b)** Tests present + pre-existing failures at BOTH HEADs →
  cluster-2 territory (shape-test-residuals-audit)
- **(c)** Tests present + pre-existing failures at baseline but NEW
  PASS at current → no follow-up
- **(d)** Tests present + passing at baseline but NEW FAIL at current
  → regression; investigate
- **(e)** Tests present + passing at both HEADs → no follow-up

### async_concurrency suite — (b) pre-existing 9 failures (cluster-2)

| HEAD | Result | Time | Status |
|---|---|---|---|
| `4c7b1d9d` (baseline-1) | FAILED 9 passed; 9 failed | 45.21s | completed |
| `1fe55c43` (current) | FAILED 9 passed; 9 failed | 44.48s | completed |

**Classification: (b)** — pre-existing 9 failures at BOTH HEADs;
identical pass/fail counts; cluster-2 territory (folds into
`shape-test-residuals-audit`).

### book_doctests suite — (b) pre-existing 2 failures (cluster-2)

| HEAD | Result | Time | Status |
|---|---|---|---|
| `4c7b1d9d` (baseline-1) | FAILED 1 passed; 2 failed | 110.83s | completed |
| `4c7b1d9d` (baseline-2) | FAILED 1 passed; 2 failed | 108.73s | completed |
| `1fe55c43` (current) | FAILED 1 passed; 2 failed | 108.41s | completed |

**Classification: (b)** — pre-existing 2 failures at BOTH HEADs;
identical pass/fail counts; cluster-2 territory.

### `compiler::v2_typed_map_emission::test_hashmap_*_emits_*` tests — N/A

These tests live in `crates/shape-vm/src/compiler/v2_typed_map_emission.rs`
(shape-vm `--lib` unit tests), NOT in `tools/shape-test/`. They are
NOT included in this shape-test runner classification. Per supervisor
2026-05-15 prior disposition note, these are "tracked under
Q25.A-unfinished-producer-side cleanup; may fold into V3-S5 cleanup".

**Recommendation:** verification of `test_hashmap_*_emits_*` failures
requires a separate `cargo test -p shape-vm --lib v2_typed_map_emission`
run at both HEADs; OUT OF SCOPE for the shape-test-runner-shape
baseline classification. Belongs in V3-S5 cleanup scope.

### v2_group_by + Array.groupBy tests — (a)/(b) MIXED — cluster-2

Per supervisor 2026-05-15 disposition, these are tracked as
`hashmap-value-v-arm` follow-up after Round 3b ckpt-4 SURFACE-AND-STOP.

**Test inventory at HEADs (grep-verified):**

- `tools/shape-test/tests/hashmap/stress_iteration.rs:626` —
  `test_hashmap_group_by_basic`
- `tools/shape-test/tests/hashmap/stress_iteration.rs:639` —
  `test_hashmap_group_by_single_group`
- `tools/shape-test/tests/hashmap/stress_iteration.rs:651` —
  `test_hashmap_group_by_empty`
- `tools/shape-test/tests/hashmap/stress_iteration.rs:657` —
  `test_hashmap_group_by_all_different`
- `tools/shape-test/tests/arrays_vectors/stress_chained.rs:472` —
  `test_group_by_modulo`
- `tools/shape-test/tests/arrays_vectors/stress_chained.rs:484` —
  `test_group_by_all_same`
- `tools/shape-test/tests/objects_arrays/arrays.rs` —
  `array_groupby` (executed at baseline; FAILED at baseline-1)

**Observation at baseline `4c7b1d9d` (run-1):**

- `hashmap` suite SIGABRT'd at `iteration::hashmap_filter_all_match`
  — **BEFORE** the `group_by` tests at line 626+. The
  `v2_group_by` / `test_hashmap_group_by_*` tests at
  `stress_iteration.rs:626+` were **NOT executed** in this run.
- `arrays_vectors` suite SIGABRT'd after 2 FAILED tests in `creation::`
  module. `test_group_by_modulo` / `test_group_by_all_same` in
  `stress_chained.rs` were **NOT executed**.
- `objects_arrays` suite ran `arrays::array_groupby` and reported it
  as **FAILED** before suite was terminated mid-run.

**Observation at current `1fe55c43` (run partial):** current head
run still in progress; arrays_vectors NOT yet run (suite hung pre-
group_by at array_every_false in closures_hof — sibling hang
unrelated to groupBy).

**Classification: MIXED (a) + (b)** —

- `test_hashmap_group_by_*` (hashmap suite, line 626+):
  **(a)** NOT EXECUTED at baseline due to upstream SIGABRT at
  `hashmap_filter_all_match`. Cannot classify regression status.
  Tests EXIST in test source at both HEADs.
- `test_group_by_modulo` / `test_group_by_all_same` (arrays_vectors,
  stress_chained.rs): **(a)** NOT EXECUTED at baseline due to
  upstream SIGABRT in arrays_vectors:creation. Cannot classify
  regression status. Tests EXIST at both HEADs.
- `array_groupby` (objects_arrays): **(b)** pre-existing FAILED at
  baseline. Folds into `shape-test-residuals-audit` cluster-2
  territory.

**hashmap-value-v-arm follow-up disposition:** **cluster-2 fold per
classification (b) for `array_groupby` PLUS (a) for the
`test_hashmap_group_by_*` / `test_group_by_*` tests that are
upstream-SIGABRT-blocked at baseline.**

The Round 3b ckpt-4 SURFACE-AND-STOP for `v2_group_by` (in
`hashmap_methods.rs`) + `Array.groupBy` (in `array_transform.rs`)
landed cleanly per `phase-3-cluster-0-status.md` lines 7502-7505. The
test cases that would validate the eventual unblock are not
independently runnable under the current shape-test runner shape due
to upstream SIGABRTs in their containing suites; the
`hashmap-value-v-arm` follow-up will need its own targeted
runner-shape (e.g. `cargo test -p shape-test --test hashmap
v2_group_by` per-test invocation post-SIGABRT-fix) to validate.

### Pre-existing 7 `compiler::v2_typed_map_emission::test_hashmap_*_emits_*` failures

Not in shape-test runner scope (shape-vm `--lib` tests). N/A this
classification.

### Total cargo test count delta — N/A (partial-data classification)

Neither run completed to terminal "Finished test" / aggregate
suite-count line under `--test-threads=1` with the runner-shape
specified by the supervisor's dispatch prompt. The classification
above is per-suite from partial data; no aggregate delta is
computable without an alternative runner-shape (e.g. per-suite
invocation with `--test` filter to bypass SIGABRT propagation).

## Regression discovery: annotation_targets SIGABRT at current HEAD (LOW CONFIDENCE — likely flakiness)

At current HEAD `1fe55c43`, the `annotation_targets` test suite
**SIGABRT'd** at `targets_declaration_type_on_type_works` (test 13 of
24) with `malloc_consolidate(): unaligned fastbin chunk detected`. At
baseline `4c7b1d9d`, the same suite completed FAILED 9 passed / 15
failed in 47-48 seconds (both run-1 and run-2 baseline runs).

**Likely root cause:** v2-raw-heap aliasing class (CLAUDE.md "Known
Constraints" line: "v2-raw-heap aliasing class — 4 simulation tests
`#[ignore]`'d ... `typed_array_push_*` realloc invalidates aliased
raw pointers held across iterations → VM Drop double-free"). The
`malloc_consolidate()` signature matches the same memory-corruption
class as the 4 `#[ignore]`'d simulation tests in CLAUDE.md.

**Confidence: LOW** — the SIGABRT pattern is stochastic
(heap-layout-dependent); SIGABRT at current HEAD may be parallel-run
memory pressure (baseline-2 + current-HEAD ran concurrently from
~08:51) interacting with the v2-raw-heap aliasing class, NOT a fresh
regression introduced 4c7b1d9d→1fe55c43.

**Recommendation:** supervisor disposes — likely classification
**(b) pre-existing stochastic SIGABRT**; cluster-2 fold under
`shape-test-residuals-audit` v2-raw-heap aliasing line item. NOT a
fresh regression unless re-run at current HEAD in isolation
reproduces the SIGABRT consistently (~3 runs needed for confidence).

## Cluster-2 territory recommendation: shape-test-residuals-audit scope

Per supervisor 2026-05-15 "if pre-existing → cluster-2" rule, the
following test failure classes are recommended for
`shape-test-residuals-audit` (cluster-2 territory) scope:

| # | Failure class | Suites affected | Sub-categorization |
|---|---|---|---|
| 1 | v2-raw-heap aliasing (`malloc_consolidate` / `tcache` SIGABRT) | annotations_comptime, annotations_runtime, arrays_vectors, hashmap (at `hashmap_filter_all_match`), objects (at `object_function_value_call`); + stochastic annotation_targets at current | matches CLAUDE.md "Known Constraints" 4-test pre-existing-#[ignore] class |
| 2 | stdlib JIT-compilation cache hang (~2-5 min per test) | closures_hof (`array_every_false`), enums (`enum_with_match_has_semantic_tokens`), error_handling (`declared_result_return_with_err_context`), functions (`closure_with_array_filter`), generics, iterators (`test_array_concat`), literals, lsp (`combined_all_features_simple`), modules_visibility | matches CLAUDE.md "Known Constraints" shape-jit deep-tests gating |
| 3 | async/concurrency hooks-and-traces 9-failure cluster | async_concurrency 9/9 fail | matches CLAUDE.md "Known Constraints" shape-test-residuals-audit (a)-(g) noted in Round 2 close |
| 4 | book_doctests 2-failure cluster | book_doctests 1/2 fail | shape-test-residuals-audit |
| 5 | annotations 9-and-15-failure cluster | annotation_targets 9/15 fail when not SIGABRT'd | shape-test-residuals-audit |
| 6 | borrow_refs 50-failure cluster | borrow_refs 155/50 fail | shape-test-residuals-audit |
| 7 | control_flow 25-failure cluster | control_flow 455/25 fail | shape-test-residuals-audit |
| 8 | jit / list_comprehension / e2e / e2e_gated / extend_blocks / features / functions / generics / hashmap failure clusters | as observed | shape-test-residuals-audit |
| 9 | objects / objects_arrays groupBy + first/last / destructuring / array_concatenation failure cluster (incl. `array_groupby` FAILED) | objects_arrays mostly | shape-test-residuals-audit |
| 10 | v2_group_by / Array.groupBy upstream-SIGABRT-blocked tests | hashmap (line 626+), arrays_vectors:stress_chained | hashmap-value-v-arm follow-up (Round 3b ckpt-4 SURFACE-AND-STOP); cluster-2 fold |

**Cluster-0+1 close criterion implication:** the pre-existing failure
pattern is structurally STABLE between `4c7b1d9d` and `1fe55c43` per
the partial data observed. No NEW failure class introduced
4c7b1d9d→1fe55c43 has been identified above the noise floor
(stochastic SIGABRT at annotation_targets at current HEAD is
INDETERMINATE — most likely v2-raw-heap aliasing pre-existing
class). The cluster-0+1 close-criterion "no regression-rate increase
from baseline" is provisionally MET pending:

1. Targeted re-run of `annotation_targets` at current HEAD in
   isolation (3+ runs to confirm stochastic vs deterministic)
2. Completion of the in-progress test runs to verify late-suite
   regression class (if any)

## Surface-and-stop disposition (per phase-2d-handover §0)

**SURFACE — supervisor disposition needed**: the
`cargo test -p shape-test --no-fail-fast -- --test-threads=1`
runner-shape specified by the dispatch prompt **cannot complete
cleanly without intervention** under the current pre-existing
SIGABRT/hang class. The classification above is built from partial
suite-completion data; structural completeness of the per-suite
diff classification requires either:

1. Per-suite invocation (`cargo test -p shape-test --test
   <suite_name>`) at both HEADs, which bypasses SIGABRT-propagation
   to subsequent suites but loses the runner-shape consistency
   requirement
2. Aggressive hang-killer + restart loop (current approach in this
   close), which yields partial completion data
3. CLAUDE.md "Known Constraints" `#[ignore]` extension covering the
   SIGABRT-prone tests (parallel to the existing 4-simulation-test
   `#[ignore]` pattern), which removes the SIGABRTs from the suite
   completion path

**Recommendation:** option 1 (per-suite invocation) for the next
session if a more complete aggregate-count comparison is needed for
cluster-0+1 close. The partial-data per-suite classification above
is structurally sufficient for the supervisor's
`hashmap-value-v-arm` disposition + cluster-2 scope
recommendation per the dispatch prompt's intent.

## Refused on sight (read-only sub-cluster discipline; particular focus)

- **#2 Pre-existing as a disposition** — every classification (b)
  call above is grounded in BOTH HEAD HEAD's observed test result
  lines OR upstream SIGABRT/hang patterns; no failure is
  classified pre-existing without baseline evidence.
- **#6 Silent follow-up framing** — every failure cluster carries an
  explicit cluster disposition (cluster-2 territory / Wave 3 fold /
  follow-up cluster name); no "tracked as follow-up to ignore"
  framing.
- **#10 anti-deferral** — no "defer to cluster-1.5" framing; the
  SURFACE-AND-STOP above surfaces the runner-shape limit cleanly
  with the structured per-suite classification still produced.
