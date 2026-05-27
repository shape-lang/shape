# annotations_runtime classification

**HEAD:** 82f049dd
**Total tests in binary:** 23
**Passed:** 0 / Failed: 23 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test annotations_runtime --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 23 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 23 failures cluster into two SURFACE shapes:

1. **op_new_array V3-S5 ckpt-5 SURFACE (14 tests)** — Runtime error fired by
   the array-literal construction site rebuild at ckpt-6, cited as "V3-S5
   ckpt-5 consumer-cascade tier 3 surface". Per TAXONOMY 2026-05-18 row,
   the annotation_targets + annotations_comptime cluster **IS this work**;
   SURFACE messages citing "V3-S5 ckpt-5 consumer-cascade" or "§5.16 v0.4"
   are SCOPE-RECLAIM by default. The annotations_runtime binary exercises
   `@before`/`@after`/`@comptime` hook scaffolding that builds args/captures
   via array literals — same root cause family.

2. **"Unknown annotation '@X'" cascade (9 tests)** — Compiler fails to
   register the user-defined `@annotation` block; downstream call-sites
   then fail type inference (`unknown` operand types) because the hook's
   parameter binding never materializes. This is the same construction-
   cascade lineage: annotation-block lowering depends on the typed-array
   constructors that ckpt-6 STRICT close is rebuilding. Annotations are
   explicit v0.3 user-pulled-in scope (TAXONOMY 2026-05-22 "Comptime trait
   into v0.3" + the 2026-05-18 row naming the annotation cluster).

Per-test rows below cite the verbatim SURFACE / error and confirm none
references a dated re-disposition to v0.4.

## Per-test classification

### before_after::after_hook_fires_after_function_body

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... Construction-site rebuild lands at
ckpt-6 STRICT close after ckpt-5-prime ... REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1). (line 12)
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade; annotation_targets cluster IS this work).
- SURFACE text: "Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
  consumer-cascade tier 3 surface ... ckpt-6 STRICT close ..."
- Incorrect v0.4 anchor cited: none explicit; "v0.4" not invoked. SURFACE
  pins itself to ckpt-6 close.
- Why cite-as-SCOPE-RECLAIM: the 2026-05-18 row binds the annotations
  cluster to the V3-S5 ckpt-5/ckpt-6 rebuild that this SURFACE marks as
  not-yet-landed. No dated re-disposition pushes the cluster to v0.4.
- Test asserts on: user-facing semantics (expects the after-hook side
  effect). Test stays the same after fix.

### before_after::after_hook_receives_correct_result_value

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Equal`: operand
types are `unknown` and `unknown`. ...
Semantic error: Unknown annotation '@check_result'
```

- Dated pull-in: 2026-05-18 (annotation cluster) + 2026-05-22 (comptime
  trait into v0.3).
- SURFACE text: "Unknown annotation '@check_result'" — annotation-block
  registration never completed; downstream `unknown`-operand cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation-block lowering is gated on the
  same construction-cascade; the cluster has a dated v0.3 pull-in.
- Test asserts on: user-facing semantics. Stays the same after fix.

### before_after::after_hook_can_transform_result

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Mul`: operand
types are `unknown` and `int`. ...
Semantic error: Unknown annotation '@negate_result'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@negate_result'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation registration failure, same family.
- Test asserts on: user-facing semantics.

### before_after::before_and_after_both_fire_in_order

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 15)
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6).
- SURFACE text: op_new_array(2) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row binds the cluster.
- Test asserts on: user-facing semantics.

### before_after::before_hook_fires_before_function_body

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 1)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row binds the cluster.
- Test asserts on: user-facing semantics.

### before_after::before_hook_with_empty_params

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 15)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(0) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### before_after::same_annotation_reused_on_multiple_functions

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 10)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(2) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### before_after::stacked_annotations_execute_outer_first

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 25)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### injection::before_hook_clamps_argument_to_range

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Less`: operand
types are `unknown` and `unknown`. ...
Semantic error: Unknown annotation '@clamp_first'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@clamp_first'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster pulled in.
- Test asserts on: user-facing semantics.

### injection::before_hook_doubles_first_argument

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Mul`: operand
types are `unknown` and `int`. ...
Semantic error: Unknown annotation '@double_first'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@double_first'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster.
- Test asserts on: user-facing semantics.

### injection::before_hook_inspects_args_without_modification

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 1)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### injection::before_hook_logs_string_argument

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### injection::before_hook_passes_ctx_info

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(0) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### injection::before_hook_swaps_arguments

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 11)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(2) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### injection::chained_before_hooks_modify_args_sequentially

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `int`. ...
Semantic error: Unknown annotation '@add_ten'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@add_ten'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster.
- Test asserts on: user-facing semantics.

### wrapping::after_hook_conditionally_transforms_result

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Greater`: operand
types are `unknown` and `unknown`. ...
Semantic error: Unknown annotation '@cap_at'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@cap_at'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster.
- Test asserts on: user-facing semantics.

### wrapping::after_hook_doubles_numeric_result

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Mul`: operand
types are `unknown` and `int`. ...
Semantic error: Unknown annotation '@double_result'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@double_result'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster.
- Test asserts on: user-facing semantics.

### wrapping::after_hook_returns_original_on_passthrough

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 14)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### wrapping::after_hook_wraps_result_in_string

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 9)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(2) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### wrapping::annotation_memoize_pattern_basic

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 15)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### wrapping::annotation_with_string_result_transformation

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 9)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(0) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### wrapping::annotation_wrapping_void_function

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... (line 1)
```

- Dated pull-in: 2026-05-18.
- SURFACE text: op_new_array(1) ckpt-5 consumer-cascade.
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row.
- Test asserts on: user-facing semantics.

### wrapping::stacked_after_hooks_transform_result_in_order

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `int`. ...
Semantic error: Cannot infer types for binary operation `Mul`: operand
types are `unknown` and `int`. ...
Semantic error: Unknown annotation '@times_two'
```

- Dated pull-in: 2026-05-18 + 2026-05-22.
- SURFACE text: "Unknown annotation '@times_two'".
- Incorrect v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: annotation cluster.
- Test asserts on: user-facing semantics.
