# jit classification

**HEAD:** 82f049dd
**Total tests in binary:** 44
**Passed:** 41 / Failed: 3 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test jit --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 3 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### correctness::jit_loop_accumulator

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade
tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
... UNREACHABLE until ckpt-6 STRICT close.
```

- **Dated user disposition:** 2026-05-18 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A/B/C. The SURFACE cites a V3-S5 ckpt-3 tier-2 surface, which is upstream of (and part of the same construction-cascade as) ckpt-5/ckpt-6. The whole V3-S5 cascade is the 2026-05-18 user pull-in.
- **SURFACE message (excerpt):** "V3-S5 ckpt-3 consumer-cascade tier 2 surface … UNREACHABLE until ckpt-6 STRICT close."
- **Incorrect anchor cited:** none cited as v0.4 / §5.16 — SURFACE itself names "ckpt-6 STRICT close" (in-v0.3 work). Per §5.16 supervisor scope (aliased-CoW SEGFAULT / imported-const ident-eval / W17-marshal / Drop codegen / B2 EnumPayload), TypedArrayData construction-cascade is NOT §5.16 territory.
- **Why cite is correct→SCOPE-RECLAIM:** V3-S5 construction-cascade is the 2026-05-18 user pull-in row. Not v0.4 territory.
- **Test asserts on:** user-facing semantics (`Expected run ok`) — test stays the same after fix.

### tiering::tier2_hot_loop_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade
tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) ...
UNREACHABLE until ckpt-6 STRICT close.
```

- **Dated user disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 construction-cascade pull-in.
- **SURFACE message (excerpt):** identical V3-S5 ckpt-3 tier-2 SURFACE as above.
- **Incorrect anchor cited:** none — SURFACE self-routes to ckpt-6 in-v0.3.
- **Why cite is correct→SCOPE-RECLAIM:** same V3-S5 cascade row.
- **Test asserts on:** user-facing semantics — stays same after fix.

### tiering::vm_aggregate_object_spread_simple

Class: **SCOPE-RECLAIM**

```
Semantic error: [E0900] post-inference FieldType::Any in user-facing schema
`__merged_44_45` at field `z` (resolved type: any). User-introduced
FieldType::Any outside the named-exception classes is the schema-side analogue
of the deleted dynamic-slot-kind variants per CLAUDE.md Forbidden Patterns
(strict-typing plan). See ADR-006 §2.7.5 + §2.7.26 + audit §5.
```

- **Dated user disposition:** 2026-05-21 — "Object destructuring must fully work." Object-spread typing is in the same surface-area; merged-schema inference on `{...a, ...b}` is user-facing object composition that must work.
- **SURFACE message (excerpt):** "post-inference FieldType::Any in user-facing schema `__merged_44_45` … strict-typing plan."
- **Incorrect anchor cited:** none v0.4 cited; surface routes to ADR-006 §2.7.5 / §2.7.26 (in-v0.3 strict-typing).
- **Why cite is correct→SCOPE-RECLAIM:** 2026-05-21 object-destructuring pull-in row.
- **Test asserts on:** user-facing semantics — stays same after fix.
