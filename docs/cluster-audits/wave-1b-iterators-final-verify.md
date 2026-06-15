# Wave 1b Iterators — FINAL VERIFY status artifact

Date: 2026-06-15
Worktree: `shape-strict-flip-collection-dispatch` (cumulative strict-flip working set, WARM)
HEAD: `907a48d4` (Wave 1b SEAM C: for-x-in-iter loop drive over Arc<IteratorState>)
Base for delta/regression: `7a51fcd3` (strict-flip W1a-B COMPLETENESS) + identical uncommitted working set.

## Verdict

SEAM A/B/C land run-verified and additive. Custom `impl Iterator for MyType`
works end-to-end VM==JIT. No trait/method-resolution regression (sf-NEW=0).
All protected gates green. The iterator pass-delta is **+47 of 116**, below the
~+100 target — the shortfall is concentrated in two surfaced/deferred clusters
(heap-element-T Array source + empty-array → V3-S5), NOT in the lazy-iterator
design or in any regression.

## Iterators binary pass-delta (single-threaded, deterministic)

| | pass | fail | total |
|---|---|---|---|
| BASE 7a51fcd3 (+working set) | 49 | 116 | 165 |
| HEAD 907a48d4 | 96 | 69 | 165 |

- Pass-delta: **+47** (96 − 49).
- Regressions (base-pass → HEAD-fail): **0** (verified via `comm`).
- Newly passing: 47 (purely additive).
- The "116" in the task framing = the 116 failing iterator tests at SEAM-A entry.

## Custom `impl Iterator for MyType` (user-trait ruling)

Program: `test-arena/wave1b_custom_iterator.shape` — `type Counter` with
`impl Iterator for Counter { method next() -> Option<int> { ... self.count = ... } }`,
driven by repeated `c.next()` + `match Some/None`.

- `--mode vm`  → `0 / 1 / 0`
- `--mode jit` → `0 / 1 / 0`
- VM==JIT parity CONFIRMED. Trait-method resolution, implicit-`self` field
  mutation, and `Option<int>` payload extraction all work.

Notes for the consumer side (NOT the trait dispatch):
- Method syntax is `method next() -> T` (implicit `self`), not `fn next(self)`
  — the explicit-`self` spelling is rejected by the compiler as expected.
- `total = total + n` where `n` binds from `Some(n)` surfaces
  "no method 'add' on receiver kind Int64" — a match-binding-payload kind-
  threading gap in arithmetic, INDEPENDENT of iterator/trait dispatch. The
  trait machinery itself (next() resolution + dispatch + Option payload) is
  sound, demonstrated by the print-based driver above.

## sf-NEW = 0 (no trait/method-resolution regression)

Failure SETS diffed base-vs-HEAD (real test names only, via `comm`):

| binary | base fail | HEAD fail | sf-NEW (HEAD-only) |
|---|---|---|---|
| traits | 36 | 36 | 0 |
| type_inference | 25 | 25 | 0 |
| closures_hof | 57 | 57 | 0 |
| iterators | 116 | 69 | 0 (all 47 deltas are fail→pass) |

The Iterator-trait seeds in `method_table.rs::register_iterator_methods` are
purely additive (register Iterator-trait methods onto the named receiver only;
cannot change resolution of any existing builtin/trait method) — empirically
confirmed: byte-identical failure sets on the three resolution-sensitive binaries.

## Gates

| gate | result |
|---|---|
| numeric_conversions | 104 / 0 PASS |
| smoke s1–s5 VM==JIT | 5 / 5 PASS (4950 / 30 / x / 2 / x) |
| borrow_refs | 215 / 4 (matches protected baseline) |
| check-clean | EXIT 0 |
| check-no-dynamic | EXIT 0 |
| verify-merge.sh | 13 / 13, EXIT 0 |

## What still surfaces (honest residuals) — 69 failures bucketed

1. **Heap-element-T Array source — ~43 tests (DOMINANT).** Two coupled layers:
   - Inference: `[1,2,3].iter()....collect()` resolves the collected element
     type to `unknown` (→ `Cannot infer types for binary operation Add: operand
     types are unknown and unknown`). `Array.iter()` does not thread element-T
     into the static `Iterator<T>` the chain resolves against. Range source
     threads correctly (`(0..3).iter().collect()` works), so this is
     Array-source-specific.
   - Runtime: even with explicit `Array<int>` annotations, the Array-source
     materialization path produces garbage / segfaults (e.g.
     `[1,2,3].iter().count()` → `-1407374883553280`; annotated collect → SIGSEGV).
     The Range source path is correct end-to-end.
   - Cluster: **V3-S5 heap-element-T Array source** (the per-T v2-raw
     `TypedArray<T>` carrier element-kind threading). Same family the task
     pre-disclosed as "may remain". NOT a ValueWord/dynamic-fallback issue.

2. **Empty-array bleed-over → V3-S5 ckpt-6 — ~18 tests.** `[]` / `for x in []`
   surfaces `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3`
   (construction-site rebuild lands at ckpt-6 STRICT close). Explicitly the
   **empty-array bleed-over → V3-S5 cluster**. Refuses TypedArrayData
   resurrection on sight (correct).

3. **enumerate — ~7 tests → V3-S5 nested-array.** `enumerate()` yields
   `[index, value]` inner arrays; the heap-element `Array<Array>` construction
   is V3-S5 nested-array territory (matches the in-code SURFACE for the HashMap
   sibling). Surfaced, not silently wrong.

4. **flatten / flatMap — ~3 tests.** Couples to the same heap-element nested-
   array construction (V3-S5).

5. **int-vs-number test-fixture expectations.** A subset of the above tests
   additionally use `.expect_number(N.0)` while the Shape computes `int`
   (strict `int != number`). These are FP test-fixture expectations from the
   pre-strict-flip `number`-unification era, layered ON TOP of the real
   inference/runtime root — they would still need rebaselining after the
   element-T root lands.

HashMap source: `Iterator(HashMap)` already SURFACEs as V3-S5 (per-entry
`[key,value]` inner-array materialization) — no test exercised it in this run,
consistent with the pre-disclosed V3-S5 deferral.

## Refused-on-sight compliance

No TypedArrayData / TypedBuffer resurrection. Array source uses the per-T
v2-raw `TypedArray<T>` flat-struct carrier (`TypedArrayArc`, kind-erased
`*mut TypedArray<T>` + stamped `_pad` discriminant). No dynamic fallback; no
Bool-default (deferred sites surface-and-stop with `NotImplemented(SURFACE)`).
Closures driven via `call_value_immediate_nb` (2.7.11/Q12). VM==JIT lockstep
on the custom-impl program.
