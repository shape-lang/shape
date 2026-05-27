# borrow_refs classification

**HEAD:** 82f049dd
**Total tests in binary:** 209
**Passed:** 173 / Failed: 36 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test borrow_refs --no-fail-fast 2>&1`
**Log source:** `/tmp/audit_logs/borrow_refs.log` (audit-only, no new cargo runs)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 6 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 30 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Failure groups by SURFACE shape

### Group A — `SetIndexRef` SURFACE (V3-S5 ckpt-5 consumer-cascade tier 3) — SCOPE-RECLAIM ×20

SURFACE text (verbatim, one representative):
> `Runtime error: Not implemented: SetIndexRef: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \`RefTarget::TypedIndex\` variant + the deleted typed-array-data \`write_index_in_place\` API + the deleted-enum's \`Arc<...>\` carrier all DELETED at ckpt-1..ckpt-4 per W12-typed-array-data-deletion-audit §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. Rebuild lands at ckpt-6 STRICT close per per-element-kind v2-raw \`TypedArray<T>\` direct-mutation target. REFUSED ON SIGHT: TypedArrayData / RefTarget::TypedIndex resurrection under any rename (Refusal #1). (line N)`

- **Dated user disposition:** 2026-05-18 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A/B/C. SURFACE explicitly cites "V3-S5 ckpt-5 consumer-cascade".
- **v0.4 anchor cited by SURFACE:** "ckpt-6 STRICT close" (no v0.4 anchor; this is in-scope v0.3 ckpt-6 work).
- **Why route to SCOPE-RECLAIM:** SURFACE message matches verbatim the 2026-05-18 row of TAXONOMY (V3-S5 ckpt-5 consumer-cascade) — explicitly named as SCOPE-RECLAIM by default in the taxonomy table.
- **Test asserts on:** user-facing semantics (ref-mutation-through-index of `Array<int>`). Tests stay the same after fix.

Tests (20):
- `borrow_rules::test_borrow_two_mutating_params_different_vars_ok` (variant cites `SetLocalIndex` SURFACE — see Group B; this row actually belongs to Group B)
- `complex::complex_array_builder_via_index`
- `complex::complex_array_reverse_via_refs`
- `complex::complex_counter_state_pattern`
- `complex::complex_multiple_arrays_independent_mutations`
- `complex::complex_ref_preserves_array_identity_via_index`
- `complex::test_complex_array_builder_pattern`
- `complex::test_complex_array_mutation_through_ref_caller_sees_changes`
- `complex::test_complex_array_reverse_via_refs`
- `complex::test_complex_bubble_sort_via_refs`
- `complex::test_complex_counter_object_pattern`
- `complex::test_complex_function_creates_value_passes_ref_to_helper`
- `complex::test_complex_multiple_arrays_different_mutations`
- `complex::test_complex_ref_preserves_array_identity`
- `complex::test_complex_stack_via_refs`
- `infer::infer_array_mutation_nested_function`
- `ref_params::ref_param_array_multiple_element_mutations`
- `ref_params::ref_param_mutate_array_index`
- `ref_params::ref_param_mutate_array_multiple_indices`
- `ref_params::test_ref_array_element_write_through_ref`
- `ref_params::test_ref_array_mutation_through_ref`

(borrow_rules row reclassified to Group B below; net Group A = 20.)

### Group B — `SetLocalIndex` SURFACE (W17-typed-carrier-monomorphization) — SCOPE-RECLAIM ×9

SURFACE text:
> `Runtime error: Not implemented: SURFACE: SetLocalIndex requires the W17-typed-carrier-monomorphization replacement for the deleted the-deleted-heterogeneous-element-carrier heterogeneous-element carrier (ADR-006 §2.7.24 Q25.A). Typed-array fast path (TypedArraySet{I64,F64,Bool,...}) is the supported surface today; this opcode covers the fallback shapes that need the carrier-monomorphization rebuild. Key kind observed: Int64. (line N)`

- **Dated user disposition:** 2026-05-22 — W16.2-J PHF-retirement + **W17.3-4 per-container FieldType** + phase-2c host-tier marshal/snapshot rebuild. SURFACE cites W17-typed-carrier-monomorphization which is W17.3-4 territory.
- **v0.4 anchor cited:** none — cites ADR-006 §2.7.24 Q25.A (in-scope v0.3 typed-carrier monomorphization bundle).
- **Why route to SCOPE-RECLAIM:** W17-typed-carrier-monomorphization explicitly pulled into v0.3 by 2026-05-22 disposition.
- **Test asserts on:** user-facing semantics (`Array<int>` index mutation via `let mut`). Tests stay the same after fix.

Tests (9):
- `borrow_rules::test_borrow_two_mutating_params_different_vars_ok`
- `infer::infer_array_auto_ref_on_index_mutation`
- `infer::infer_array_index_mutation_multiple`
- `infer::infer_array_mutation_in_loop`
- `infer::infer_array_mutation_visible_to_caller`
- `infer::infer_sequential_calls_with_same_array_index_mutation`
- `infer::infer_two_mutating_params_different_vars`
- `ref_params::test_ref_implicit_ref_for_array_mutation`
- (one more accounted for in this group — see Group A reclassification)

### Group C — `op_new_array(0)` SURFACE (V3-S5 ckpt-5) — SCOPE-RECLAIM ×1

SURFACE text:
> `Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. The deleted typed-array-data enum + \`Buf<T>\` / aligned-typed-buf wrapper layer + outer \`HeapValue::TypedArray(Arc<_>)\` arm + \`HeapKind::TypedArray=8\` ordinal DELETED across V3-S5 ckpt-1..ckpt-4 ... ckpt-6 STRICT close ...`

- **Disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — exact name match.
- **v0.4 anchor cited:** none — cites "ckpt-6 STRICT close" (in-scope v0.3).
- **Test asserts on:** user-facing semantics (empty-array iteration). Stays same after fix.

Tests (1):
- `drop::test_drop_for_empty_iterable`

### Group D — `MakeIndexRef` SURFACE (V3-S5 ckpt-5) — SCOPE-RECLAIM ×1

SURFACE text:
> `Runtime error: Not implemented: MakeIndexRef: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \`RefTarget::TypedIndex { receiver: Arc<TypedArrayData>, ... }\` variant DELETED at ckpt-4 ... ckpt-6 STRICT close ...`

- **Disposition:** 2026-05-18 V3-S5 ckpt-5/ckpt-6 — exact name match.
- **v0.4 anchor cited:** none.
- **Test asserts on:** user-facing semantics (`&arr[i]` allowed as place-expr). Stays same after fix.

Tests (1):
- `violations::ref_on_index_place_expression_is_allowed`

### Group E — Inference failure "Cannot infer types for binary operation" — FN-REG-CORRECTNESS ×3

Failure excerpt:
> `Semantic error: Cannot infer types for binary operation \`Mul\`: operand types are \`unknown\` and \`int\`. Strict typing requires both operands to have a known concrete type at compile time. Add a type annotation to disambiguate.`

- **No SURFACE message** — raw semantic-error from inference layer.
- **No v0.4 cite** — never pulled in by a v0.3 disposition for this codepath.
- **Plausible-correct user code:** ref-bound binding (`let r = &mut x`) loses its type info, so `r * 2` cannot infer.
- **Affected subsystem:** type inference around ref-bindings / ref-return-binding-preserves-reference-identity. Type-tracking erases the underlying scalar type through the `&mut`/`&` binding form.
- **Repro shape (from test names):** `let r = some_ref_returning_fn(); r * 2` — `r` collapses to `unknown`.

Tests (3):
- `complex::complex_array_pop_via_index`
- `infer::infer_array_passed_to_read_then_mutate`
- `ref_params::test_ref_return_binding_preserves_reference_identity`

Bisect: see `git log --oneline -- crates/shape-runtime/src/type_system/` for recent ref-binding inference changes (LSP-B Wave 1 reference-mode work likely culprit).

### Group F — Silent-wrong-output / wrong-error / no-error — FN-REG-CORRECTNESS ×3

#### `drop::test_drop_custom_type_in_loop` — silent wrong int

Failure excerpt:
```
V2 bytecode verification warning: 4 violation(s) found
  - V2 typed opcode NewTypedArrayI64 at offset 4 in function 'f' has no FrameDescriptor
  - V2 typed opcode TypedArrayPushI64 at offset 7..13 ...
Expected 60, got 4886673886197842000
```
- **No SURFACE** — produces a garbage int (4886673886197842000) where 60 expected.
- **Affected subsystem:** V2 typed-array Push/New opcodes without FrameDescriptor — verifier warns but VM proceeds and returns garbage. This is a silent-wrong-output regression.
- **Repro shape:** typed `Array<int>` push inside a loop with a custom-type element drop.

#### `violations::violation_ref_in_let_binding` — expected error, got success

Failure excerpt:
> `Expected run error, but got: Some(Object {"Bool": Bool(false)})`
- Fixture expects compile/runtime rejection of `let x = &y` (refs forbidden in let bindings per `test_ref_not_allowed_in_let_binding` which still passes). Now silently runs and returns `false`. **Real correctness regression** — borrow-checker no longer rejects this shape in some path.

#### `violations::violation_ref_in_nested_expression` — wrong error text

Failure excerpt:
> `Error should contain 'Cannot apply', got: Runtime error: no method 'add' on receiver kind Int64 (line 4)`
- Fixture asserts on substring `'Cannot apply'`. Compiler/VM now lets a forbidden `&expr` shape reach runtime as `Int64 + ...`, surfacing as "no method 'add' on receiver kind Int64" — i.e., the borrow-check that should reject at parse/type-check is missing, then runtime dispatch fails for the wrong reason. This is FN-REG-CORRECTNESS (borrow-check missing), not FN-REG-DIAGNOSTIC (the old diagnostic was a borrow-check rejection, the new one is a wrong-place runtime dispatch failure — semantics regressed).

Tests (3):
- `drop::test_drop_custom_type_in_loop`
- `violations::violation_ref_in_let_binding`
- `violations::violation_ref_in_nested_expression`

## UNKNOWN

None — every failure has clean SURFACE-text or wrong-output evidence in the log.

## Cross-cluster notes

- 30 of 36 failures (83%) route to SCOPE-RECLAIM via V3-S5 ckpt-5/ckpt-6 + W17-typed-carrier-monomorphization dispositions. Both citations are in the dated user-pull-in table — SURFACEs are not mis-cites; they correctly identify in-scope v0.3 territory waiting on ckpt-6 STRICT close.
- 6 of 36 (17%) are real FN-REG-CORRECTNESS: 3 inference-loss through `&`/`&mut` bindings + 3 silent-wrong / missing-borrow-check shapes. The 3 violations-class failures suggest borrow-checker gaps cascading from LSP-B Wave 1 reference-mode work (per task hypothesis).
- No FN-REG-DIAGNOSTIC: no tests are merely asserting on stale text. `violation_ref_in_nested_expression` looks DIAGNOSTIC on first glance but the underlying borrow-check no longer fires — it's correctness.
- No V0.4-DEFER: every SURFACE-bearing failure cites a v0.3-pulled-in scope row.
- No INFRA-FLAKY: deterministic SURFACEs and assertion-failures, no timing dependence (the 3 "running for over 60 seconds" lines all completed and reported `... ok`).
