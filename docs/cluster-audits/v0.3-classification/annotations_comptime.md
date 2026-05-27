# annotations_comptime classification

**HEAD:** 82f049dd
**Total tests in binary:** 23
**Passed:** 0 / Failed: 23 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test annotations_comptime --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 23 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 23 failures share an identical SURFACE message originating from
`comptime_target::nb_object_array` (the V3-S5 ckpt-5 construction-cascade
tier-3 SURFACE for the deleted `Arc<Buf<TypedObjectPtr>>` typed-array-data
result carrier). Per the TAXONOMY 2026-05-18 dated user pull-in row, the
**annotation_targets + annotations_comptime cluster IS this work** — it
was pulled into v0.3 scope on 2026-05-18 and has not been re-dispositioned
to v0.4 by any later dated authorization. The SURFACE cite of "§5.16
JIT-lowering followup workstream (v0.4 / planned)" is a mis-cite: §5.16's
actual scope (per TAXONOMY: aliased-CoW SEGFAULT + imported-const ident-eval +
W17-marshal + Drop codegen + B2 EnumPayload) does NOT absorb V3-S5
construction-cascade work. All 23 entries are therefore SCOPE-RECLAIM.

The shared mis-cite is the same SURFACE text and the same SCOPE-RECLAIM
rationale for every entry; the per-test entries below reproduce the
required fields verbatim per the TAXONOMY format.

## Per-test classification

### code_gen::annotation_generates_to_string_method

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_generates_to_string_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. the deleted typed-array-data TypedObject `Arc<Buf<TypedObjectPtr>>` result carrier DELETED at ckpt-1..ckpt-4 per W12 audit §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. Rebuild lands at ckpt-6 STRICT close per v2-raw `TypedArray<TypedObjectPtr>` direct-access. REFUSED ON SIGHT (Refusal #1). Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A typed-object-element + W16.2-B trait-object-element + W16.2-C empty-literal/spread/comprehension. The annotation_targets + annotations_comptime cluster IS THIS WORK.
- **SURFACE text:** `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per docs/v0.3-close-summary.md §5.16 JIT-lowering followup workstream).`
- **Incorrect v0.4 anchor cited:** `docs/v0.3-close-summary.md §5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16's actual scope (per TAXONOMY) is aliased-CoW SEGFAULT + imported-const ident-eval + W17-marshal + Drop codegen + B2 EnumPayload. §5.16 does NOT absorb V3-S5 ckpt-5/ckpt-6 construction-cascade work. The 2026-05-18 user pull-in explicitly names the annotations_comptime cluster as v0.3 scope.
- **Test asserts on:** user-facing semantics (`Expected run ok` — the test expects the annotation to apply successfully and produce a working `to_string` method). Test stays the same after fix.

### code_gen::stacked_annotations_both_extend_type

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::stacked_annotations_both_extend_type' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — annotations_comptime cluster IS this work.
- **SURFACE text:** same `V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE ... §5.16 JIT-lowering followup workstream` mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### code_gen::annotation_extends_type_with_equality_check

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_extends_type_with_equality_check' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### code_gen::annotation_replace_body_generates_constant_function

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_replace_body_generates_constant_function' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### code_gen::annotation_generates_display_method

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_generates_display_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### code_gen::annotation_generates_getter_method

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_generates_getter_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### code_gen::annotation_generates_predicate_method

Class: **SCOPE-RECLAIM**

```
thread 'code_gen::annotation_generates_predicate_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_extend_adds_method_to_type

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_extend_adds_method_to_type' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_extend_adds_computed_method

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_extend_adds_computed_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_extend_adds_multiple_methods

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_extend_adds_multiple_methods' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_remove_target_eliminates_type

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_remove_target_eliminates_type' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_replace_body_overrides_function

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_replace_body_overrides_function' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_set_param_default

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_set_param_default' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::comptime_post_with_annotation_param_in_method

Class: **SCOPE-RECLAIM**

```
thread 'on_define::comptime_post_with_annotation_param_in_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### on_define::targets_declaration_restricts_to_type

Class: **SCOPE-RECLAIM**

```
thread 'on_define::targets_declaration_restricts_to_type' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::annotation_extends_type_with_boolean_method

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::annotation_extends_type_with_boolean_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::annotation_extends_type_with_string_method

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::annotation_extends_type_with_string_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::annotation_removes_and_replaces_type

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::annotation_removes_and_replaces_type' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::annotation_with_param_used_in_generated_method

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::annotation_with_param_used_in_generated_method' panicked at tools/shape-test/src/shape_test.rs:1280:9:
Error should contain 'Undefined variable: default_val', got: Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite. (Test expected a `'Undefined variable: default_val'` compile error and was masked by the earlier-firing SURFACE.)
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics — specifically the diagnostic message `'Undefined variable: default_val'`. Test stays the same after fix (once the SURFACE clears, the underlying compile-time check should re-emit the expected diagnostic).

### type_mutation::extend_target_adds_derived_method

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::extend_target_adds_derived_method' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::extend_target_adds_method_using_multiple_fields

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::extend_target_adds_method_using_multiple_fields' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::extend_target_method_with_parameters

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::extend_target_method_with_parameters' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.

### type_mutation::replace_body_on_function_target

Class: **SCOPE-RECLAIM**

```
thread 'type_mutation::replace_body_on_function_target' panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE. ... Feature impl pending (v0.4 / planned per `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup workstream).")
```

- **Dated user pull-in:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — annotations_comptime cluster IS this work.
- **SURFACE text:** identical V3-S5 ckpt-5 tier-3 SURFACE with §5.16 mis-cite.
- **Incorrect v0.4 anchor cited:** `§5.16 JIT-lowering followup workstream`.
- **Why the cite is incorrect:** §5.16 actual scope excludes V3-S5 construction-cascade; 2026-05-18 pull-in covers this cluster.
- **Test asserts on:** user-facing semantics (`Expected run ok`). Test stays the same after fix.
