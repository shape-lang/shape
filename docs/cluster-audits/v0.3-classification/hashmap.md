# hashmap classification

**HEAD:** 82f049dd
**Total tests in binary:** 185
**Passed:** 120 / Failed: 65 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test hashmap --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 65 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 65 failures cluster into the V3-S5 ckpt-1..ckpt-6 typed-array-data
deletion + HashMap per-V monomorphization rebuild lineage. Both the
`Arc<TypedArrayData>` result-carrier deletion (TAXONOMY 2026-05-18 row,
explicitly names op_new_array + V3-S5 ckpt-5/ckpt-6 + Q25.A SUPERSEDED)
and the `Arc<HashMapKindedRef>` outer-carrier flip with per-V mutation
API (HEAD-recent W13-hashmap-mutation + Phase 3 cluster-0+1 Wave 2
Round 3b C2-joint ckpt-2..ckpt-4 + Wave 3 V3-S5 ckpt-5/ckpt-6 STRICT
FINAL CLOSE: TAXONOMY 2026-05-22 row "W16.2-J PHF-retirement + W17.3-4
per-container FieldType" + ADR-006 §2.7.24 Q25.A/B SUPERSEDED) are
dated user-pulled-in v0.3 scope. None of the SURFACE messages cites
"v0.4" — they pin themselves to ckpt-6 STRICT close.

## Per-test classification (grouped by SURFACE family — same dated
## pull-in applies to every test in the group)

### Family A — KVE_SURFACE (22 tests): HashMap.keys/values/entries/toArray V3-S5 ckpt-5 SURFACE

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: HashMap.{keys,values,entries,toArray}:
SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface.
`Arc<TypedArrayData>` result carrier DELETED at V3-S5 ckpt-1..ckpt-4
per W12-typed-array-data-deletion audit §3.5 + §3.6 + §B + ADR-006
§2.7.24 Q25.A SUPERSEDED. Rebuild lands at ckpt-6 STRICT close per the
per-T v2-raw `TypedArray<T>` carrier shape. REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1).
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade — the HashMap iterator-returning method bodies
  consume the deleted `Arc<TypedArrayData>` carrier the same way the
  annotation_targets cluster does).
- SURFACE text: see block above.
- v0.4 anchor cited: none (SURFACE pins itself to "ckpt-6 STRICT close").
- Why cite-as-SCOPE-RECLAIM: 2026-05-18 row binds keys/values/entries
  rebuild; the carrier they used was deleted by the dated pull-in.
- Test asserts on: user-facing semantics (.length / pair content).
  Tests stay the same after fix.

Tests: `methods::hashmap_entries_returns_array`,
`methods::hashmap_keys_contains_entries`,
`methods::hashmap_keys_returns_array`,
`methods::hashmap_values_returns_array`,
`stress_iteration::test_hashmap_entries_are_pairs`,
`stress_iteration::test_hashmap_entries_count`,
`stress_iteration::test_hashmap_entries_empty`,
`stress_iteration::test_hashmap_entries_pair_values`,
`stress_iteration::test_hashmap_filter_then_keys`,
`stress_iteration::test_hashmap_keys_count`,
`stress_iteration::test_hashmap_keys_empty`,
`stress_iteration::test_hashmap_keys_returns_array`,
`stress_iteration::test_hashmap_keys_values_entries_consistent`,
`stress_iteration::test_hashmap_single_entry_keys`,
`stress_iteration::test_hashmap_single_entry_values`,
`stress_iteration::test_hashmap_to_array_empty`,
`stress_iteration::test_hashmap_to_array_length`,
`stress_iteration::test_hashmap_to_array_pair_content`,
`stress_iteration::test_hashmap_to_array_produces_pairs`,
`stress_iteration::test_hashmap_values_count`,
`stress_iteration::test_hashmap_values_empty`,
`stress_iteration::test_hashmap_values_returns_array`.

### Family B — OP_NEW_ARRAY (1 test): array-literal construction-site SURFACE

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5
ckpt-5 consumer-cascade tier 3 surface. The deleted typed-array-data
enum + `Buf<T>` / aligned-typed-buf wrapper layer + outer
`HeapValue::TypedArray(Arc<_>)` arm + `HeapKind::TypedArray=8` ordinal
DELETED across V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion
audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array — exact
  match; the TAXONOMY row names this site by name).
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: TAXONOMY 2026-05-18 names op_new_array as
  v0.3 user-pulled-in scope.
- Test asserts on: user-facing semantics (HashMap inside `[a, b]` array
  literal).

Test: `stress_operations::test_hashmap_in_array`.

### Family C — INT_KEY (12 tests): integer keys rejected post per-V monomorphization

Class: **SCOPE-RECLAIM**

```
Runtime error: HashMap key must be a string (got kind Int64) (line N)
```

- Dated pull-in: 2026-05-22 ("W16.2-J PHF-retirement + W17.3-4
  per-container FieldType + phase-2c host-tier marshal/snapshot
  rebuild"). HEAD-recent `W13-hashmap-mutation` +
  `Phase 3 cluster-0+1 Wave 2 Round 3b C2-joint ckpt-2..ckpt-4` flipped
  HashMap to the `Arc<HashMapKindedRef>` outer-carrier shape with key
  kind narrowed to `string` per the per-V monomorphization recipe
  (ADR-006 §2.7.24 Q25.B SUPERSEDED amendment). Integer-key support is
  part of the per-V rebuild that has not yet landed; the runtime
  surface-and-stop fires from
  `crates/shape-vm/src/executor/objects/hashmap_methods.rs:497`.
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: the per-V rebuild that re-introduces
  per-key-kind narrowing is v0.3 user-pulled-in scope (2026-05-22).
- Test asserts on: user-facing semantics (HashMap with `int` keys).
  Tests stay the same after fix.

Tests: `basic::hashmap_get_integer_key`,
`stress_creation::test_hashmap_build_loop`,
`stress_creation::test_hashmap_loop_build_and_query`,
`stress_iteration::test_hashmap_conditional_set`,
`stress_operations::test_hashmap_has_integer_key`,
`stress_operations::test_hashmap_has_integer_key_missing`,
`stress_operations::test_hashmap_integer_key_delete`,
`stress_operations::test_hashmap_integer_key_has`,
`stress_operations::test_hashmap_integer_key_missing`,
`stress_operations::test_hashmap_integer_key_multiple`,
`stress_operations::test_hashmap_integer_key_set_get`,
`stress_operations::test_hashmap_mixed_key_types`.

### Family D — BOOL_KEY (3 tests): boolean keys rejected

Class: **SCOPE-RECLAIM**

```
Runtime error: HashMap key must be a string (got kind Bool) (line N)
```

- Dated pull-in: 2026-05-22 (same per-V rebuild as Family C).
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: same root cause as Family C.
- Test asserts on: user-facing semantics (HashMap with `bool` keys).

Tests: `stress_operations::test_hashmap_bool_key_both`,
`stress_operations::test_hashmap_bool_key_false`,
`stress_operations::test_hashmap_bool_key_true`.

### Family E — STRINGV2_KEY (1 test): StringV2 key kind rejected by string-only check

Class: **SCOPE-RECLAIM**

```
Runtime error: HashMap key must be a string (got kind StringV2) (line 6)
```

- Dated pull-in: 2026-05-22 (W17.3-4 per-container FieldType — the
  HashMap key-kind check at `hashmap_methods.rs:497` accepts a single
  `String` kind, but the W18.3 string-rebuild emits `StringV2` for
  loop-built keys).
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: TAXONOMY 2026-05-22 names W17.3-4 +
  2026-05-22 W18 content-rendering rebuild — both touch string carrier.
- Test asserts on: user-facing semantics (HashMap set in a loop with
  string keys).

Test: `stress_iteration::test_hashmap_set_in_loop_with_string_keys`.

### Family F — VALUE_KIND_INCOMPAT (7 tests): value-kind narrowing on first .set()

Class: **SCOPE-RECLAIM**

```
Runtime error: HashMap.set(): value kind {Int64|Null|Ptr(TypedArray)}
incompatible with HashMap<string, string> (line N)
```

- Dated pull-in: 2026-05-22 (W17.3-4 per-container FieldType +
  ADR-006 §2.7.24 Q25.B per-V HashMap monomorphization). The first
  `.set()` call narrows the V parameter; subsequent sets with a
  different kind fail. Also surfaces on `set("k", None)` (Null kind),
  `set("k", [1,2,3])` (Ptr(TypedArray)), and `set("k", 30)` after
  `set("name", "Alice")` (Int64 vs Ptr(String)).
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: the per-V rebuild is v0.3 user-pulled-in
  scope. Mixed-V HashMap support (or first-set V-inference of a
  permissive supertype) needs to land at ckpt-6 STRICT close.
- Test asserts on: user-facing semantics (mixed-V HashMap, Null values,
  Array values).

Tests: `basic::hashmap_get_existing_key`,
`stress_iteration::test_hashmap_config_pattern`,
`stress_operations::test_hashmap_array_value_access`,
`stress_operations::test_hashmap_get_or_default_with_none_value`,
`stress_operations::test_hashmap_has_with_none_value`,
`stress_operations::test_hashmap_set_array_value`,
`stress_operations::test_hashmap_set_get_none_value`.

### Family G — MERGE_KIND (2 tests): HashMap.merge() value-kind mismatch

Class: **SCOPE-RECLAIM**

```
Runtime error: HashMap.merge(): value-kind mismatch
(Ptr(String) vs Int64); merge requires same-V receivers at this layer
```

- Dated pull-in: 2026-05-22 (per-V monomorphization + W13-hashmap-mutation
  per-V merge API — same root cause as Family F).
- SURFACE text: see block above.
- v0.4 anchor cited: none ("at this layer" pins to ckpt-6 rebuild).
- Why cite-as-SCOPE-RECLAIM: same root cause as Family F.
- Test asserts on: user-facing semantics (merge empty with non-empty).

Tests: `stress_iteration::test_hashmap_merge_empty_with_nonempty`,
`stress_iteration::test_hashmap_merge_with_empty`.

### Family H — CLOSURE_UNKNOWN (11 tests): closure `|k, v|` param-kind inference returns `unknown`

Class: **SCOPE-RECLAIM**

```
Semantic error: Cannot infer types for binary operation
`{Add|Mul|Greater|Equal}`: operand types are `unknown` and {`unknown`|`int`}.
Strict typing requires both operands to have a known concrete type at
compile time. Add a type annotation to disambiguate.
```

- Dated pull-in: 2026-05-22 (W17.3-4 per-container FieldType — the
  bidirectional closure-param inference rule that resolves `|k, v|
  ...` against HashMap's per-V monomorphization signature needs to land
  with the rebuild). Per TAXONOMY 2026-05-21 ("Array<string> must
  work") the same family of generic-container element-kind closure
  inference is v0.3 user-pulled-in scope; HashMap is the same shape
  modulo K kind.
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: 2026-05-21 + 2026-05-22 rows bind generic-
  container closure-param inference; the per-V HashMap rebuild needs to
  re-wire `MethodFnV2` per-V receiver-recovery into closure param-kind
  resolution.
- Test asserts on: user-facing semantics (`.map(|k,v| v*2)`,
  `.reduce(0, |a,k,v| a+v)`, `.filter(|k,v| v>2)`, `m.len() == 3`).

Tests: `stress_iteration::test_hashmap_counter_pattern`,
`stress_iteration::test_hashmap_map_preserves_len`,
`stress_iteration::test_hashmap_map_squared`,
`stress_iteration::test_hashmap_map_then_reduce`,
`stress_iteration::test_hashmap_map_then_values`,
`stress_iteration::test_hashmap_reduce_empty`,
`stress_iteration::test_hashmap_reduce_max_value`,
`stress_iteration::test_hashmap_reduce_product`,
`stress_iteration::test_hashmap_reduce_string_initial`,
`stress_iteration::test_hashmap_reduce_sum_values`,
`stress_operations::test_hashmap_len_equals_length`.

### Family I — IMMUT_BIND (4 tests): let-binding rejected as receiver of `.set()`/`.delete()`

Class: **SCOPE-RECLAIM**

```
Semantic error: cannot assign to immutable binding 'm'
Semantic error: Cannot reassign immutable variable 'm'. Use `let mut`
or `var` for mutable bindings
```

- Dated pull-in: 2026-05-22 (W13-hashmap-mutation per-V mutation API
  flipped `.set()` / `.delete()` from "returns new HashMap" to "mutates
  receiver"; BindingStorageClass for `let` then rejects the mutating
  call). The user-facing semantics changed under the rebuild; the test
  source asserts the pre-rebuild "returns-new" contract that the W13
  + Phase 3 cluster-0+1 Wave 2 Round 3b ckpt-2..ckpt-4 monomorphization
  intentionally retired.
- SURFACE text: see block above.
- v0.4 anchor cited: none.
- Why cite-as-SCOPE-RECLAIM: the user-pulled-in scope (W13 + ADR-006
  §2.7.24 Q25.B) drove the contract change. Tests will need updating
  to `let mut` / `var` after the rebuild reaches a stable surface.
- Test asserts on: SURFACE-itself (the test source presumes the
  returns-new contract; once the per-V rebuild is stable, the tests'
  expected outputs may still hold via the new mutation contract, but
  `let` → `let mut` / `var` rewrites are likely needed). Both
  classifications above (asserts-on-SURFACE vs asserts-on-semantics)
  are arguable for this family — flagging for supervisor disposition.

Tests: `basic::hashmap_delete_key`, `basic::hashmap_set_returns_new_map`,
`stress_operations::test_hashmap_has_after_set`,
`stress_operations::test_hashmap_set_immutability`.

### Family J — B0005 (2 tests): `let mut` accumulator captured by `.forEach` closure

Class: **SCOPE-RECLAIM**

```
Semantic error: [B0005] `let mut` binding 'X' was moved into a closure
here and cannot be read in the outer scope afterwards (Rust-move
semantics). Use `var X` if the binding needs to be observed or mutated
in the outer scope after capture, or observe mutations via the
closure's return value.
```

- Dated pull-in: 2026-05-22 (ADR-006 §2.7 Q7 — `var` smart-default
  carrier with BindingStorageClass `SharedAtomicMut` for cross-closure
  observability; `let mut` is bounded to non-escaping `Direct`/
  `UniqueHeap`). The B0005 borrow-checker diagnostic is the
  surface-and-stop for the new BindingStorageClass discipline that
  ADR-006 §2.7 ratified.
- SURFACE text: see block above.
- v0.4 anchor cited: none (B0005 is a structured surface-and-stop with
  remediation guidance — "Use `var X`").
- Why cite-as-SCOPE-RECLAIM: ADR-006 binding-class redesign is v0.3
  user-pulled-in scope (2026-05-22 W17.3-4 row + ADR-006 §2.7 amendment
  cluster). Tests need rewriting to `var` after rebuild stabilizes.
- Test asserts on: SURFACE-itself (test source uses `let mut sum = 0`
  / `let mut count = 0` then captures in `.forEach`; rewrite to `var`
  preserves user-facing semantics).

Tests: `stress_iteration::test_hashmap_foreach_single_entry`,
`stress_iteration::test_hashmap_foreach_with_accumulator`.

## UNKNOWN

None.
