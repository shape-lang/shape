# drop_raii classification

**HEAD:** 82f049dd
**Total tests in binary:** 18
**Passed:** 15 / Failed: 3 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test drop_raii --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 3 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 3 failures share an identical SURFACE on the `range()` builtin
(loop iteration fixture); none are Drop-codegen failures. Drop-codegen
itself is exercised cleanly by the 15 passing tests (LIFO ordering,
nested scopes, function return, early return, sequential blocks, mixed
drop/non-drop, deeply-nested, block exit, scope exit, multiple drops,
function return). The failing 3 are all `for i in range(0, N)` fixtures
where `range()` hits the V3-S5 ckpt-3 TypedArrayData-deletion consumer
cascade before Drop codegen ever runs.

## Per-test classification

### control_flow::drop_in_loop_body_each_iteration

Class: **SCOPE-RECLAIM**

```
thread 'control_flow::drop_in_loop_body_each_iteration' panicked at
tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: Not implemented:
range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.
`TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A
SUPERSEDED. ... Receiver kind: Int64. UNREACHABLE until ckpt-6
STRICT close. ...")
```

- **Dated user disposition pulling in the work:** 2026-05-18 — V3-S5
  ckpt-5/ckpt-6 op_new_array construction-cascade (W16.2-A/B/C
  typed-object-element / trait-object-element / empty-literal /
  spread / comprehension). The W12 TypedArrayData deletion +
  per-T monomorphization landing across ckpt-3/4/5/6 is the same
  construction-cascade workstream authorized for v0.3.
- **Exact SURFACE message text:** `range: SURFACE — V3-S5 ckpt-3
  consumer-cascade tier 2 surface. TypedArrayData enum DELETED at
  ckpt-1 (2026-05-15) ... UNREACHABLE until ckpt-6 STRICT close.`
- **(Incorrect) v0.4 anchor cited by the SURFACE:** none — SURFACE
  cites `ckpt-6 STRICT close` (a v0.3 in-flight checkpoint), not v0.4
  / planned. The fact that no v0.4 anchor is cited is itself
  diagnostic: the work is not v0.4-territory; it is v0.3 in-flight
  per the 2026-05-18 pull-in.
- **Why the cite is incorrect:** the SURFACE describes the work as
  awaiting ckpt-6 close, which is in v0.3 scope per the 2026-05-18
  row of the TAXONOMY table. Routing this to V0.4-DEFER would
  require a dated re-disposition that does not exist.
- **Test-asserts-on-SURFACE or user-facing-semantics:** user-facing
  semantics. Fixture is `for i in range(0, 3) { let it = Iter {...} }`
  + `expect_output_contains("drop:0")`. Test stays the same once
  ckpt-6 lands `range()` on the v2-raw `TypedArray<T>` carrier.

### control_flow::drop_on_break

Class: **SCOPE-RECLAIM**

```
thread 'control_flow::drop_on_break' panicked at ...:
Expected run ok, got error: Some("Runtime error: Not implemented:
range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.
`TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) ...
UNREACHABLE until ckpt-6 STRICT close. ...")
```

- **Dated user disposition pulling in the work:** 2026-05-18 — V3-S5
  ckpt-5/ckpt-6 op_new_array construction-cascade. Same W12
  monomorphization landing.
- **Exact SURFACE message text:** identical `range: SURFACE` body as
  above (V3-S5 ckpt-3 consumer-cascade tier 2).
- **(Incorrect) v0.4 anchor cited by the SURFACE:** none; cites
  ckpt-6 STRICT close (v0.3 in-flight).
- **Why the cite is incorrect:** ckpt-6 is in v0.3 scope per the
  2026-05-18 row. No dated re-disposition to v0.4 exists.
- **Test-asserts-on-SURFACE or user-facing-semantics:** user-facing
  semantics. Fixture is `for i in range(0, 10) { let b = Brk {...};
  if i == 2 { break } }` + `expect_output_contains("drop:2")`. Test
  stays the same after ckpt-6 close.

### control_flow::drop_on_continue

Class: **SCOPE-RECLAIM**

```
thread 'control_flow::drop_on_continue' panicked at ...:
Expected run ok, got error: Some("Runtime error: Not implemented:
range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface.
`TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) ...
UNREACHABLE until ckpt-6 STRICT close. ...")
```

- **Dated user disposition pulling in the work:** 2026-05-18 — V3-S5
  ckpt-5/ckpt-6 op_new_array construction-cascade.
- **Exact SURFACE message text:** identical `range: SURFACE` body as
  above.
- **(Incorrect) v0.4 anchor cited by the SURFACE:** none; cites
  ckpt-6 STRICT close (v0.3 in-flight).
- **Why the cite is incorrect:** ckpt-6 is in v0.3 scope per the
  2026-05-18 row.
- **Test-asserts-on-SURFACE or user-facing-semantics:** user-facing
  semantics. Fixture is `for i in range(0, 3) { let c = Cnt {...};
  if i == 1 { continue } }` + `expect_output_contains("drop:1")`.
  Test stays the same after ckpt-6 close.
