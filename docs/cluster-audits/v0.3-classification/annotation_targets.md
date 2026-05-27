# annotation_targets classification

**HEAD:** 82f049dd
**Total tests in binary:** 24
**Passed:** 8 / Failed: 16 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test annotation_targets --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 16 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 16 failures route to **SCOPE-RECLAIM** under the dated 2026-05-18 user
disposition: *"V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade.
W16.2-A typed-object-element + W16.2-B trait-object-element + W16.2-C
empty-literal/spread/comprehension. **The annotation_targets +
annotations_comptime cluster IS THIS WORK.**"* (TAXONOMY §SCOPE-RECLAIM
row 1). All failures are `op_new_array(N)` runtime SURFACEs or
`comptime_target::nb_object_array` comptime-array SURFACEs — both cited in
the dated disposition. The `nb_object_array` SURFACE additionally mis-cites
"v0.4 / planned per §5.16 JIT-lowering followup workstream"; per TAXONOMY
this is a mis-cite (§5.16 actual scope = aliased-CoW SEGFAULT + imported-
const ident-eval + W17-marshal + Drop codegen + B2 EnumPayload, NOT V3-S5
construction-cascade), so the failures still route to SCOPE-RECLAIM, not
V0.4-DEFER.

All tests assert on user-facing semantics (annotation `@before` / `@after`
hooks must run the function, return values must round-trip, etc.) — they
will need NO update once V3-S5 ckpt-5/ckpt-6 construction-cascade lands;
the same fixture text becomes green.

## Per-test classification

### function_target::annotation_with_targets_function_on_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... REFUSED ON SIGHT: TypedArrayData
resurrection under any rename (Refusal #1). (line 15)
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade; annotation_targets cluster named explicitly).
- SURFACE: `op_new_array(2): SURFACE — V3-S5 ckpt-5 consumer-cascade tier
  3 surface`.
- v0.4 anchor cited: none (correctly routes through ckpt-5 / ckpt-6).
- Why mis-cite would be incorrect: N/A — SURFACE correctly names ckpt-5;
  but the failure remains SCOPE-RECLAIM because the work is in v0.3.
- Asserts on: user-facing semantics (annotation runs the function).

### function_target::multiple_annotations_on_same_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 13)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (both annotations chain through).

### other_targets::annotation_on_module_item

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (annotation applies to module item).

### function_target::annotation_on_multi_param_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(3): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 15)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(3): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (multi-param function with annotation).

### function_target::annotation_on_recursive_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(1): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (recursive function + annotation).

### function_target::annotation_on_function_with_return_value

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 10)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(1): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (return value round-trip).

### type_target::two_annotations_on_same_type

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per
`docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 construction-cascade;
  annotation_targets cluster named).
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16 JIT-lowering followup workstream`.
- Why cite is incorrect: §5.16 actual scope per TAXONOMY = aliased-CoW
  SEGFAULT + imported-const ident-eval + W17-marshal + Drop codegen + B2
  EnumPayload. V3-S5 construction-cascade is NOT §5.16 scope; the 2026-05-
  18 dated pull-in keeps this in v0.3.
- Asserts on: user-facing semantics (two annotations chain on type).

### type_target::annotation_on_type_with_before_after_hooks

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per
... §5.16 JIT-lowering followup workstream).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16 JIT-lowering followup workstream` (mis-cite —
  §5.16 ≠ V3-S5 construction-cascade).
- Asserts on: user-facing semantics (`@before` / `@after` hooks fire).

### other_targets::targets_declaration_function_on_function_works

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (`@targets function` declaration on
  function works).

### type_target::annotation_on_type_with_remove

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... (v0.4 / planned per ... §5.16 ...).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16` (mis-cite).
- Asserts on: user-facing semantics (annotation `@remove` clause).

### function_target::annotation_on_simple_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (basic annotation on simple function).

### other_targets::targets_declaration_type_on_type_works

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... (v0.4 / planned per ... §5.16 ...).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16` (mis-cite).
- Asserts on: user-facing semantics (`@targets type` declaration applies).

### type_target::annotation_on_type_adds_boolean_method

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... (v0.4 / planned per ... §5.16 ...).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16` (mis-cite).
- Asserts on: user-facing semantics (annotation injects boolean method).

### type_target::annotation_on_type_with_extend

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... (v0.4 / planned per ... §5.16 ...).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16` (mis-cite).
- Asserts on: user-facing semantics (annotation `@extend` clause).

### function_target::annotation_on_void_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 1)
```

- Dated pull-in: 2026-05-18.
- SURFACE: `op_new_array(1): SURFACE — V3-S5 ckpt-5 consumer-cascade`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (void-return function + annotation).

### type_target::annotation_on_type_adds_computed_method

Class: **SCOPE-RECLAIM**

```
Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-
cascade tier 3 SURFACE. ... (v0.4 / planned per ... §5.16 ...).
```

- Dated pull-in: 2026-05-18.
- SURFACE: `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade
  tier 3 SURFACE`.
- v0.4 anchor cited: `§5.16` (mis-cite).
- Asserts on: user-facing semantics (annotation injects computed method).
