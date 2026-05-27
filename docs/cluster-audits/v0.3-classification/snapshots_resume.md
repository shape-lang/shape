# snapshots_resume classification

**HEAD:** 82f049dd
**Total tests in binary:** 13
**Passed:** 12 / Failed: 1 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test snapshots_resume --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### advanced::recompile_same_source_runs_ok

Class: **SCOPE-RECLAIM**

```
thread 'advanced::recompile_same_source_runs_ok' panicked at tools/shape-test/src/shape_test.rs:1292:9:
Expected run ok, got error: Some("Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3
consumer-cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. ... Post-deletion
target is the v2-raw `TypedArray<T>` flat-struct carrier ... per-T monomorphization landing across
ckpt-3 (this file plus typed_array_methods/iterator_methods/array_sort/concat/property_access/
array_query) + ckpt-4 (Buf<T> / HeapValue::TypedArray arm / HeapKind::TypedArray ordinal) +
ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 (JIT FFI). Receiver kind: Int64.
UNREACHABLE until ckpt-6 STRICT close. REFUSED ON SIGHT: TypedArrayData resurrection under any
rename (Refusal #1, W12 audit §7). (line 4)")
```

Repro:
```shape
fn compute() {
    let mut sum = 0
    for i in range(1, 11) {
        sum = sum + i
    }
    sum
}
compute()
```

- **Dated user disposition the underlying work was pulled-in by:** 2026-05-22 (W16.2-J PHF-retirement + W17.3-4 per-container FieldType + phase-2c host-tier marshal/snapshot rebuild — the W12 TypedArrayData-deletion cascade is the v2-raw `TypedArray<T>` per-container FieldType migration named here). Snapshot/resume work was additionally explicitly pulled-in via the same 2026-05-22 disposition ("phase-2c host-tier marshal/snapshot rebuild") which is also the audit-trigger phrasing for this binary.
- **Exact SURFACE message text:** `range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. TypedArrayData enum DELETED at ckpt-1 ... UNREACHABLE until ckpt-6 STRICT close.`
- **(Incorrect) v0.4 anchor cited by the SURFACE:** None directly — SURFACE cites "ckpt-3 ... UNREACHABLE until ckpt-6 STRICT close" (i.e., in-flight V3-S5 construction-cascade work). Per TAXONOMY §SCOPE-RECLAIM, the 2026-05-18 disposition explicitly names "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade" as in v0.3 scope; ckpt-3 is an earlier checkpoint of the same construction-cascade workstream and routes to SCOPE-RECLAIM by default.
- **Why the cite is incorrect:** The SURFACE truthfully labels itself as v0.3 ckpt-3 work; the failure is not a v0.4 mis-cite, it is in-flight v0.3 work that has not landed yet. The `range()` builtin (a load-bearing iteration primitive used by `for i in range(...)`) is broken at HEAD as a downstream consumer of the W12 TypedArrayData deletion cascade. Per the 2026-05-21 disposition ("Object destructuring must fully work" et al., implicitly: load-bearing iteration must work) and the 2026-05-22 disposition pulling in the phase-2c host-tier marshal/snapshot rebuild, `range()` returning a usable iterable IS v0.3-gating.
- **Test asserts on SURFACE or on user-facing semantics:** Asserts on user-facing semantics (`expect_number(55.0)` — sum 1..=10). Test stays the same after the underlying ckpt-3/4/5/6 cascade fix; no fixture update needed.

## Notes

The 12 passing tests cover snapshot builder + simple/string/bool/typed-object/multi-type/nested-type/recompile/modified-source/schema-change/determinism paths — `.with_snapshots()` is a no-op-shaped enablement and the underlying snapshot store API is not yet exposed (multiple test bodies carry TDD comments noting "ShapeTest does not expose snapshot save/restore API"). Only the single test that exercises `range()` in a `for` loop trips the V3-S5 ckpt-3 SURFACE.
