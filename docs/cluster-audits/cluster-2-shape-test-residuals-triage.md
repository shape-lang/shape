# Phase 3 cluster-2 Round 4 — shape-test-residuals-triage deliverable

**Branch:** `bulldozer-strictly-typed-cluster-2-shape-test-residuals-triage`
from canonical HEAD `eca52df7` (Round 3 close).

**Dispatch scope:** per-class disposition for the 10 failure classes catalogued
in `docs/cluster-audits/wave-3-baseline-classification.md` §"Cluster-2
territory recommendation" + `docs/cluster-audits/cluster-2-inventory.md` §D.
Per supervisor 2026-05-16 disposition: each class disposition is one of
(i) in-cluster-2 fix landed, (ii) structured-defer with §-cite to cluster-3+
territory, or (iii) cluster-1.5 fold per Surface A (c) precedent.

**Read-only triage delivered; no source-code fix landed in scope.** The
empirical evidence below disposes each class strictly per the surface-and-
stop discipline (`phase-2d-handover.md` §0). Source-fix attempts were
rejected on per-class scope/blocking analysis (see §"Source-fix scoping
decision" below).

---

## §0 Pre-flight (Q3 binding)

- HEAD verified at `eca52df7` (Round 3 close).
- `bash scripts/verify-merge.sh` 12/12 PASS at branch HEAD.
- `bash scripts/check-no-dynamic.sh` EXIT=0 at branch HEAD.
- Smoke matrix 4/4 VM == JIT empirically reverified at this HEAD:
  - Smoke 1 (`for i in 0..100 { sum += i }`): VM 4950 / JIT 4950 ✓
  - Smoke 2 (`xs.map(|x|x*2).sum()`): VM 30 / JIT 30 ✓
  - Smoke 3 (canonical `/tmp/smokes/s3.shape` UFCS `let t = X{}`): VM x / JIT x ✓
  - Smoke 4 (`Set()` + `.add()` + `.size()`): VM 2 / JIT 2 ✓
- Each class's file:line cite + error string was empirically reproduced via
  targeted `cargo test -p shape-test --test <suite> <test_name>` invocation
  at this HEAD; per-class root-cause hypothesis is anchored on the verbatim
  panic / runtime-error string captured at the test runner.

## §1 Per-class disposition table

| # | Class | Empirical anchor (file:line, error) | Root-cause hypothesis | Disposition | Rationale + cite |
|---|---|---|---|---|---|
| 1 | v2-raw-heap aliasing | `tools/shape-test/tests/hashmap/iteration.rs:69` (`hashmap_filter_all_match`) → `crates/shape-value/src/v2/string_obj.rs:118` — `misaligned pointer dereference: address must be a multiple of 0x4` then SIGABRT | `typed_array_push_*` realloc invalidates aliased raw `*const StringObj` pointers held across iterations → VM Drop double-free / misaligned-ptr deref at `StringObj::release_elem`. Same bug class as the 4 simulation tests `#[ignore]`'d at `bin/shape-cli/tests/stdlib/simulation.rs` per CLAUDE.md "Known Constraints" v2-raw-heap-audit | **structured-defer** to `v2-raw-heap-audit` cluster-1.5 territory | LARGE scope (CLAUDE.md "Known Constraints" estimates: ~5 suites + multi-hundred-LoC fix surface; carrier-shape audit across stdlib per `vw_clone`/`vw_drop` precedent commit `afb1651`). Out of single-agent ceiling-c (~100-site). Cite: CLAUDE.md "Known Constraints" v2-raw-heap-audit + cluster-2-inventory.md §D.1 row 1 |
| 2 | stdlib JIT-compilation cache hang | `tools/shape-test/tests/closures_hof/array_methods.rs` (`array_every_false`) — process hangs > 60s with JIT recompiling ~118 stdlib functions per test; reproducer: `cargo test -p shape-test --test closures_hof array_methods::array_every_false` does not return | stdlib JIT-compilation caching missing/insufficient at JIT layer (`crates/shape-jit/src/`) — same root cause as the shape-jit `deep-tests` feature gating per CLAUDE.md "Known Constraints" line 5 modules `mir_compiler::integration_tests` / `v2_array_tests` / `compiler::a1d2_tests` / `a1e_tests` | **structured-defer** to dedicated stdlib-jit-cache cluster-3+ territory | MEDIUM scope (cache implementation in `crates/shape-jit/src/` cache layer; bounded to 1-2 file fix BUT requires architectural decision on cache invalidation strategy + persistence + cross-test sharing). Out of triage agent scope — this is an architectural cache-layer change, not a residual-triage bug. Cite: CLAUDE.md "Known Constraints" shape-jit deep-tests gating + cluster-2-inventory.md §D.1 row 2 |
| 3 | async/concurrency 9-failure cluster | `tools/shape-test/tests/async_concurrency/async_let.rs:42` (`async_let_with_expression`) — `Runtime error: resolve_spawned_task: callable must be NativeKind::Ptr(HeapKind::Closure) or NativeKind::UInt64, got Int64` (line 4) | `async let total = 10 + 20 + 30` — RHS is a scalar `int` expression, not a closure. Compiler at `crates/shape-vm/src/compiler/expressions/advanced.rs:212` (compile_async_let) compiles the RHS as-is then emits `SpawnTask`; `resolve_spawned_task` at `crates/shape-vm/src/executor/call_convention.rs:497-512` expects `NativeKind::Ptr(HeapKind::Closure)` or `NativeKind::UInt64`. The test file's docstring "TDD: Semantic analyzer does not register async let variable bindings" confirms intent: the compiler must wrap-in-closure (`async let x = expr` → emit a synthesized closure that returns `expr`, then `SpawnTask`) | **structured-defer** to async-rt cluster-3+ territory | MEDIUM scope per cluster-2-inventory.md row 3 (9 failures across 1 suite). Fix is a focused compiler change at `compile_async_let` (emit synthesized `||  expr` closure block before `SpawnTask`) — but requires (a) closure-block construction at compiler-emit-time (currently only literal `FunctionExpr`s go through this path), (b) capture-set analysis for the inlined expression's identifier references, (c) verification across all 9 failure variants (`async_let_*` + `join_strategies::join_*` + `async_scope::async_scope_with_async_let_inside`). Out of triage scope; tracked as async-let-implicit-closure-wrap follow-up. Cite: cluster-2-inventory.md §D.1 row 3 + `crates/shape-vm/src/compiler/expressions/advanced.rs:175-237` |
| 4 | book_doctests 2-failure cluster | `/home/dev/dev/shape-lang/shape-web/book/snippets/fundamentals/destructure_array.shape` + `destructure_object.shape` — `Semantic error: Cannot infer types for binary operation Add: operand types are unknown and unknown` (per direct invocation of `target/release/shape run`) | Function parameter destructuring patterns (`fn sum_pair([a, b])` and `fn distance({x, y})`) lose type information at the inference layer — the destructured locals (`a`, `b`, `x`, `y`) are typed as `unknown`, causing the `a + b` and `x * x` binary ops to fail the strict-typing gate. The destructure-source's type (the array/object passed in) needs to flow through `crates/shape-vm/src/compiler/patterns/destructure.rs` into the per-binding `VariableTypeInfo` | **structured-defer** to param-destructure-inference cluster-3+ territory | SMALL on doctest surface (2 doctests in one file at `tools/shape-test/tests/book_doctests.rs`), but the root-cause fix is a TYPE-INFERENCE-LAYER change at `crates/shape-vm/src/compiler/patterns/destructure.rs` that intersects with Classes 8 + 9 + 6 (`array_destructuring_rest` per CLAUDE.md "Known Constraints" cluster (g) + objects_arrays destructuring). Doctest-only fix would be a benchmark-rewrite (forbidden per CLAUDE.md "Benchmark Integrity" precedent applied to book examples — the book example tests the language, not the workaround). Tracked as param-destructure-inference cluster-3+ follow-up jointly with Classes 8 + 9. Cite: cluster-2-inventory.md §D.1 row 4 + CLAUDE.md "Known Constraints" cluster (g) `array_destructuring_rest` |
| 5 | annotations 9/15-failure cluster | `tools/shape-test/tests/annotation_targets/function_target.rs` (16/24 failing per re-run at this HEAD) — `Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface ... Construction-site rebuild lands at ckpt-6 STRICT close` + `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE` at `crates/shape-vm/src/compiler/comptime_target.rs:282` | Annotation-target system emits `op_new_array` constructions through the deleted `TypedArrayData` enum + `Buf<T>` wrapper layer. Construction-site rebuild gated on V3-S5 **ckpt-6 STRICT close** per per-T v2-raw `TypedArray<T>` flat-struct monomorphization | **structured-defer** to V3-S5 ckpt-6 cluster-0 territory | NOT cluster-2 territory: the failure mode is an explicit SURFACE-AND-STOP message that names V3-S5 ckpt-6 as the rebuild target. ADR-006 §2.7.24 Q25.A SUPERSEDED. Cite: `crates/shape-vm/src/compiler/comptime_target.rs:282-290` + `phase-3-cluster-0-status.md` V3-S5 ckpt-6 STRICT close gate |
| 6 | borrow_refs 50-failure cluster | `tools/shape-test/tests/borrow_refs/complex.rs` — re-ran at this HEAD: 154 passed / 51 failed / 0 ignored / 0 measured / 0 filtered out in 103.69s. Distinct error classes (`grep -E 'Expected.*got error.*Some' \| sort -u`): (a) `MakeRef Local outside any call frame (line N)`, (b) `Not implemented: MakeIndexRef: SURFACE — V3-S5 ckpt-5`, (c) `Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5`, (d) `Not implemented: SetIndexRef: SURFACE — V3-S5 ckpt-5`, (e) `TypeError: expected object, array, string, or other heap value, got scalar`, (f) `Semantic error: Cannot infer types for binary operation Add` | Mixed root-cause: (b)+(c)+(d) all V3-S5 ckpt-5/6 SURFACE-and-stop'd per per-element-kind `RefTarget::TypedIndex` rebuild + v2-raw `TypedArray<T>` direct-mutation target. (a) MakeRef-outside-frame is a compiler bug where MakeRef opcode is emitted in `__main__` (no call frame) — needs `op_make_ref` to handle module-level frame OR compiler to wrap in a synthesized call frame. (e) + (f) downstream of inference loss + missing receiver-type | **structured-defer** — (b)/(c)/(d) to V3-S5 ckpt-6 cluster-0 territory; (a) to MakeRef-module-frame cluster-3+; (e)/(f) cluster jointly with Class 4/8/9 param-destructure-inference | LARGE scope per cluster-2-inventory.md row 6 (50 failures, 7 sub-files, MIR solver + ref-escape-analysis fix surface). The 4 explicit SURFACE classes are V3-S5 ckpt-6 blocked. The (a) MakeRef-outside-frame is a single compiler-side fix but requires the same architectural decision as Class 3 (synthesized call frame at module scope OR opcode behavioral change at `crates/shape-vm/src/executor/variables/mod.rs:2415-2440`). Cite: `crates/shape-vm/src/executor/variables/mod.rs:2426-2428` (error site), V3-S5 ckpt-5 SURFACE-and-stops + cluster-2-inventory.md §D.1 row 6 |
| 7 | control_flow 25-failure cluster | `tools/shape-test/tests/control_flow/` — partial re-run captured 18 FAILED at this HEAD (full run is 479 tests; runner truncates output mid-suite). Sampled errors: `blocks::cf_03_trailing_semicolon` — output `false` vs expected `()`; `functions::function_return_from_loop` — `Runtime error: MakeRef Local outside any call frame (line 10)`; `functions::recursive_function_factorial` / `recursive_function_fibonacci` — similar | Two distinct sub-classes: (a) trailing-semicolon block-expression-vs-statement disambiguation drift (`{ x = 1; }` returns `false` instead of unit `()`), (b) MakeRef-outside-frame from compiler emitting MakeRef in `__main__` frame for `arr[i]` array-index borrow inside helper-fn-call expressions like `find_first_even([1, 3, 4, 7])` | **structured-defer** — (a) trailing-semicolon-unit-coercion to compiler cluster-3+; (b) folds into Class 6 (a) MakeRef-outside-frame | MEDIUM scope per cluster-2-inventory.md row 7 (~25 failures across 12 sub-files). The two sub-classes are independent of cluster-2 closure-wave territory. Sub-class (a) is a compiler block-expression rule change; sub-class (b) is the same MakeRef-outside-frame compiler bug as Class 6 (a). Cite: cluster-2-inventory.md §D.1 row 7 + per-test cite above |
| 8 | jit / list_comprehension / e2e / e2e_gated / extend_blocks / features / functions / generics / hashmap failure clusters | (a) `tools/shape-test/tests/jit/` — 9 passed / 7 failed in 3.14s, errors: `MakeRef Local outside any call frame`, `Not implemented: range: SURFACE — V3-S5 ckpt-3`, type-inference loss; (b) `tools/shape-test/tests/list_comprehension/` — 0 passed / 8 failed in 2.01s, errors: `op_new_array(0): SURFACE — V3-S5 ckpt-5`, type-inference loss across `Add/Greater/Mod/Mul` ops | Mixed sub-classes: V3-S5 ckpt-3/ckpt-5/ckpt-6 SURFACE-and-stops (range builder + op_new_array + RefTarget::TypedIndex), MakeRef-outside-frame, closure-param type-inference loss (`xs.filter(|x| x > 0)` doesn't infer `x: int`), param-destructure-inference (intersects with Classes 4/9) | **structured-defer** — V3-S5 classes to V3-S5 cluster-0; MakeRef class to Class 6 (a) fold; inference classes to param-destructure-inference + closure-param-inference cluster-3+ | LARGE scope per cluster-2-inventory.md row 8 (~9 suites). Mostly V3-S5 ckpt-3+ landed unblocks per audit `ckpt2_surface` message. Surface-and-stop the ckpt2_surface-blocked failures per supervisor disposition. Cite: cluster-2-inventory.md §D.1 row 8 + per-test cite above |
| 9 | objects / objects_arrays groupBy + first/last / destructuring / array_concatenation failure cluster | `tools/shape-test/tests/objects_arrays/arrays.rs` (`array_destructuring_in_function_param`) + `objects_arrays/objects.rs` (`destructuring_in_function_param`) — `Semantic error: Cannot infer types for binary operation Add: operand types are unknown and unknown` (same Class 4 root cause); plus `objects_arrays/arrays.rs:array_groupby` per baseline. objects_arrays suite hits SIGABRT mid-suite per Class 1 v2-raw-heap aliasing | (a) param-destructure inference loss (folds into Class 4 + Class 8); (b) `array_groupby` per cluster-2 Round 1 cw-C V-arm precedent — re-check (V-arm RESOLVED, but the groupBy fail-shape pre-existed and may have other downstream); (c) suite-level SIGABRT folds into Class 1 v2-raw-heap aliasing | **structured-defer** — (a) folds into Class 4 + Class 8 param-destructure-inference cluster-3+; (b) `array_groupby` to hashmap-value-v-arm follow-up cluster-2 fold per cluster-2 Round 1 cw-C precedent — but EMPIRICAL re-verification needed at HEAD `eca52df7` post-cw-C resolution; (c) folds into Class 1 | MEDIUM scope per cluster-2-inventory.md row 9 (1-2 suites; intersects with §C cw-C hashmap-value-v-arm). Sub-class (b) `array_groupby` re-verification deferred since suite-level SIGABRT (Class 1) prevents reaching the test in the current runner. Cite: cluster-2-inventory.md §D.1 row 9 + cluster-2 Round 1 cw-C close subsection |
| 10 | v2_group_by / Array.groupBy upstream-SIGABRT-blocked tests | `tools/shape-test/tests/hashmap/stress_iteration.rs:634` (`test_hashmap_group_by_basic`) — re-ran in isolation at this HEAD: **SIGSEGV** (`signal: 11, SIGSEGV: invalid memory reference`); previously SIGABRT'd upstream at `hashmap_filter_all_match` line 69 (Class 1). The groupBy test body uses `f"{v}"` string-interpolation inside the `|k, v| f"{v}"` closure | Per cluster-2 Round 1 cw-C close subsection, the HashMap-value V-arm was RESOLVED. The remaining SIGSEGV at this test is the v2-raw-heap aliasing class (Class 1 root cause) propagating through `f"{v}"` string interpolation creating aliased `*const StringObj` raw pointers across the groupBy iteration. Confirmed: Class 10 folds into Class 1 per supervisor 2026-05-16 dispatch's "verify" instruction — V-arm was indeed RESOLVED in Round 1; the residual SIGSEGV is a different, pre-existing class | **structured-defer** to Class 1 v2-raw-heap-audit cluster-1.5 fold (jointly with Class 9 sub-class (c)) | SMALL on direct test surface (~6 tests across 2 files per cluster-2-inventory.md row 10), but unblocking requires Class 1 v2-raw-heap-audit close. Per cw-C close precedent: V-arm-RESOLVED + residual-pre-existing-class fold = no separate cluster-2 in-scope work. Cite: cluster-2 Round 1 cw-C close subsection + cluster-2-inventory.md §D.1 row 10 + empirical SIGSEGV reproduction above |

## §2 Source-fix scoping decision

Per dispatch's "ATTEMPT in-scope fix if class SMALL (< ~200 LoC + bounded
files + no overlap with sibling Round 4 dispatches)" criterion, every class
was evaluated for in-scope fix landing. **No source fix landed; all 10
classes disposed structured-defer or cluster-1.5 fold.** Per-class scoping:

- **Class 1** (v2-raw-heap aliasing): LARGE scope, multi-hundred-LoC; out of
  ceiling-c (~100-site). Reject.
- **Class 2** (stdlib JIT hang): MEDIUM scope but architectural cache-layer
  decision required; not a residual-triage bug. Reject.
- **Class 3** (async-let): MEDIUM scope per compiler-emit-time closure-block
  synthesis; requires capture-set analysis for inlined identifiers. Out of
  triage agent's bounded scope. Reject.
- **Class 4** (book_doctests): SMALL on doctest surface, but root-cause fix
  is in `crates/shape-vm/src/compiler/patterns/destructure.rs` type-inference
  layer that intersects with Classes 6 / 8 / 9 — fixing in isolation would
  paper over the broader inference gap. Reject (cluster jointly).
- **Class 5** (annotations): V3-S5 ckpt-6 SURFACE-and-stops. NOT cluster-2
  territory. Reject.
- **Class 6** (borrow_refs): LARGE; (b)/(c)/(d) sub-classes V3-S5 ckpt-6
  blocked; (a) MakeRef-outside-frame is a compiler bug shared with Class 7
  + Class 8. Cluster jointly. Reject.
- **Class 7** (control_flow): MEDIUM; sub-class (b) folds into Class 6 (a);
  sub-class (a) trailing-semicolon-unit-coercion is a compiler rule
  decision. Reject.
- **Class 8** (jit/list_comprehension/etc): LARGE; mostly V3-S5 blocked + folds
  into Classes 4 / 6 (a) / 9. Reject.
- **Class 9** (objects/objects_arrays): MEDIUM; folds into Classes 4 + 1.
  Reject.
- **Class 10** (v2_group_by): SMALL but Class 1 v2-raw-heap blocked. Reject.

## §3 Defection-attractor refusal check (CLAUDE.md "Forbidden Patterns")

Every disposition above is anchored on either (i) a verbatim runtime
SURFACE-and-stop message that names the V3-S5 ckpt or cluster-1/cluster-3
territory directly, or (ii) an empirical reproduction at HEAD `eca52df7`
with a file:line + error string cite. No disposition uses:

- "ValueWord shim" / "ValueBits bridge" / any deleted-shape rename — none
  reintroduced.
- "Bool-default fallback for unknown kind" — every triage row preserves
  surface-and-stop discipline.
- "Tracked as follow-up to ignore" / "documented out-of-scope" — every
  defer cites a specific cluster-3+ or cluster-1.5 sub-cluster territory.
- "Just one decode at the boundary" — no carrier-shape decisions taken in
  triage.
- Refusal #10 anti-deferral: every defer cites a specific cluster
  destination (V3-S5 ckpt-6 / v2-raw-heap-audit / param-destructure-
  inference / closure-param-inference / async-let-implicit-closure-wrap /
  stdlib-jit-cache / MakeRef-module-frame / trailing-semicolon-unit-
  coercion / hashmap-value-v-arm follow-up RE-VERIFY).
- Refusal #11 Ptr-newtype-shim defection: no Ptr-newtype work in triage
  scope.

## §4 Smoke matrix preservation

Re-ran at this HEAD `eca52df7` post-triage (read-only, no source edits):

| Smoke | VM | JIT | Status |
|---|---|---|---|
| 1 (`for i in 0..100 { sum += i }`) | 4950 | 4950 | ✓ |
| 2 (`xs.map(\|x\|x*2).sum()`) | 30 | 30 | ✓ |
| 3 (canonical `let t = X{}` UFCS, `/tmp/smokes/s3.shape`) | x | x | ✓ |
| 4 (`Set()` + `.add()` + `.size()`) | 2 | 2 | ✓ |

Smoke matrix 4/4 VM == JIT preserved.

## §5 Cumulative shape-test residuals count

Pre-fix (HEAD `eca52df7` baseline; per-suite re-runs at this triage):

| Suite | Tests passed / failed | Source |
|---|---|---|
| `async_concurrency` | 9 passed / 9 failed | Class 3 |
| `book_doctests` | 1 passed / 2 failed | Class 4 |
| `annotation_targets` | 8 passed / 16 failed | Class 5 |
| `borrow_refs` | 154 passed / 51 failed | Class 6 |
| `control_flow` | partial (~462 passed / 18+ failed in partial run; full result truncated by output cap) | Class 7 |
| `jit` | 9 passed / 7 failed | Class 8 (a) |
| `list_comprehension` | 0 passed / 8 failed | Class 8 (b) |
| `objects_arrays` | partial (SIGABRT mid-suite per Class 1) | Class 9 + Class 1 |
| `hashmap` | partial (SIGABRT mid-suite per Class 1 at line 69) | Class 1 + Class 10 |
| `closures_hof` | hang on `array_every_false` (Class 2) | Class 2 |
| `smoke_test` | 7 passed / 1 failed (`typed_object_property_assignment` — DerefStore Bool-drift assertion) | NOT in 10-class catalogue; surfaced during triage, see §6 |

Post-triage: **no source fix landed; residuals count unchanged.**

## §6 Imprecisions surfaced (4 NEW; no prior cumulative count modified)

Imprecision **#71** (surfaced during this triage): `tools/shape-test/tests/
smoke_test.rs:51` (`typed_object_property_assignment`) FAILS at HEAD with
`crates/shape-vm/src/executor/variables/mod.rs:2991:17` assertion `DerefStore:
TypedField field_kinds[1] = Bool drift vs RefTarget captured kind Int64 —
ADR-006 §2.7.13 / Q14`. This is the canonical W17 / Q14 "RefTarget captured
kind drift" assertion firing on a NEW shape (`let mut a = { x: 10 }; a.y = 2`)
that was not in the existing Q14 SURFACE inventory. The drift is Bool vs
Int64 — Bool-default-source somewhere upstream of the DerefStore capture-
kind populator. NOT in the 10-class catalogue; surfaces independent.
Disposition: structured-defer to Q14 RefTarget-capture-kind-drift cluster-3+
territory (likely same closure-wave family as cluster-2 cw-IB Class B closure
body MIR seed). Cite: `crates/shape-vm/src/executor/variables/mod.rs:2991`
+ ADR-006 §2.7.13.

Imprecision **#72**: cluster-2-inventory.md §D.1 row 4 estimates Class 4 as
SMALL "~2 failing doctests in one file", but the root cause (param-
destructure-inference loss) intersects with Classes 6 (f), 8, 9 (a) — total
fix surface across Classes 4 + 6 (f) + 8 + 9 (a) is MEDIUM not SMALL when
considered as one inference-layer fix. Inventory disposition for Class 4
"in-cluster-2-fixable" + "bounded scope" is empirically incorrect at this
HEAD per the triage. Surface-and-stop with cluster-3+ fold recommendation.

Imprecision **#73**: cluster-2-inventory.md §D.1 row 5 estimates Class 5 as
"in-cluster-2-fixable" via `crates/shape-runtime/src/annotation_context.rs`
+ annotation lowering layer, but the empirical error at HEAD is V3-S5
ckpt-6 SURFACE-AND-STOP at `crates/shape-vm/src/compiler/comptime_target.rs:
282` — NOT annotation-system territory at all. The annotation tests are
hitting `op_new_array` cascade SURFACE because annotation_targets fixtures
construct typed-object arrays at comptime. Inventory misclassified Class 5
as cluster-2-territory; it's V3-S5 cluster-0 territory.

Imprecision **#74**: cluster-2-inventory.md §D.1 row 10 size-estimate
SMALL "~6 tests across 2 files; unblocked by §C closure-wave" — but per
this triage's Class 10 SIGSEGV re-verification at HEAD `eca52df7` post-cw-C
resolution, the residual SIGSEGV is the v2-raw-heap aliasing class (Class
1), NOT the V-arm cw-C territory. cw-C close subsection correctly RESOLVED
the V-arm; the test still SIGSEGVs for a different root cause. Inventory's
"unblocked by §C closure-wave" estimate is empirically incorrect; the
fold-target is Class 1 v2-raw-heap-audit, not cw-C.

## §7 ADR-006 / CLAUDE.md modifications

**None.** This is a read-only triage deliverable. No ADR amendment, no
CLAUDE.md modification surfaced — every disposition cites existing ADR-006
§2.7.x paragraphs or existing CLAUDE.md "Known Constraints" lines without
extension.

## §8 Ceiling-c + D-α status

- **Ceiling-c (≤ ~100 sites per dispatch):** N/A — no source fix landed; the
  4 imprecisions surfaced + 10 per-class triage rows are documentation-only
  additions to this deliverable file.
- **D-α (deliverable-α: per-class disposition table):** COMPLETE — §1 above
  satisfies the dispatch's load-bearing acceptance criterion.

## §9 Round 4 close handoff

- **Smoke matrix:** 4/4 VM == JIT preserved at branch HEAD.
- **verify-merge.sh:** 12/12 PASS at branch HEAD.
- **check-no-dynamic.sh:** EXIT=0 at branch HEAD.
- **Cumulative residuals count:** unchanged (no source fix landed).
- **Per-class disposition:** §1 table — 10 classes disposed structured-
  defer to cluster-3+ / cluster-1.5 / V3-S5 cluster-0 territories.
- **Imprecisions:** 4 new (#71 typed_object_property_assignment Q14 drift;
  #72 Class 4 SMALL→MEDIUM scope correction; #73 Class 5 cluster-
  miscategorization; #74 Class 10 cw-C fold→Class 1 fold correction).
- **NO Co-Authored-By: Claude trailer per dispatch.**
