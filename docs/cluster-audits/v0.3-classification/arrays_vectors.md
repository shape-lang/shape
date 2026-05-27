# arrays_vectors classification

**HEAD:** 82f049dd
**Total tests in binary:** 385
**Passed:** 271 / Failed: 114 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test arrays_vectors --no-fail-fast 2>&1`
**Run log:** `/tmp/claude-1000/-home-dev-dev-shape-lang-shape/98f37812-cc9d-4343-bc19-fe3a93b4e09f/tasks/b5jc80fx6.output` (cargo test wall-clock 2402.57s).
**Per-test panic excerpts:** ad-hoc invocations of the cached binary `target/debug/deps/arrays_vectors-ed85787651f81fbd --test-threads=1 --nocapture --exact <names>` were used to recover panic details that the parallel run truncated.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 10 |
| FN-REG-DIAGNOSTIC  | 3 |
| SCOPE-RECLAIM      | 101 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |
| **Total**          | **114** |

### SCOPE-RECLAIM sub-totals (by SURFACE family)

| Shape | Count | Dated pull-in | SURFACE family |
|---|---|---|---|
| A — Empty-literal `[]` un-resolvable element type | 38 | 2026-05-18 (W16.2-C empty-literal/spread/comprehension) + 2026-05-21 (Array<string>) | Semantic error: empty array `X` has an un-resolvable element type |
| B — V3-S5 ckpt-5 `op_new_array` / `op_new_typed_array` construction cascade | 39 | 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade) | Runtime error: Not implemented: op_new_array(N) / op_new_typed_array(N) |
| C — V3-S5 ckpt-2 consumer cascade (filter / map / flatMap / except / intersect / union / unique on TypedArray) | 16 | 2026-05-18 (V3-S5 ckpt-2..ckpt-6 cascade) | Runtime error: Not implemented: filter\|map\|flatMap\|except\|intersect\|union\|unique: SURFACE — V3-S5 ckpt-2 |
| D — V3-S5 ckpt-3 `joinStr` per-V2ElemType stringification primitive | 2 | 2026-05-18 (V3-S5 ckpt cascade; D4 2026-05-24 J.5f) | Runtime error: Not implemented: joinStr: SURFACE — per-V2ElemType element stringification primitive not yet landed |
| E — phase-2c kinded iterator API (iter_kind=Bool not supported) | 1 | 2026-05-22 (phase-2c host-tier marshal/snapshot rebuild) | Runtime error: Not implemented: op_iter_done SURFACE: iter_kind=Bool not supported as iterator |
| F — Pipeline / empty-array downstream "Cannot infer types" cascade | 5 | 2026-05-18 (W16.2-C cascade) + 2026-05-21 (Array<string>) | Semantic error: Cannot infer types for binary operation `Add`/`Mul`/`Equal` (often co-emitted with Shape A) |
| **SCOPE-RECLAIM total** | **101** | | |

## Per-test classification

The 114 failures cluster into 14 distinct evidence shapes (A–N). Per the TAXONOMY's "Run-verify binding" rule, every classification below is backed by a verbatim error excerpt from a `cargo test` invocation at HEAD 82f049dd; the excerpt is given once per shape (since the binary returns the same error verbatim for every member of a shape) and the per-test list is enumerated under each shape.

---

### Shape A — empty-literal un-resolvable element type (SCOPE-RECLAIM)

Verbatim error (binding to every Shape-A entry; only the binding name `X` differs):
```
Semantic error: empty array `X` has an un-resolvable element type. It is
created empty (`[]`) with no `Array<T>` annotation and is never pushed to, so
the compiler cannot prove what element type it holds. Strict typing requires a
known concrete element type: add an annotation (`let X: Array<T> = []`) or
remove the unused binding.
```

SCOPE-RECLAIM required fields (apply to every Shape-A entry):
- **Dated user disposition the work was pulled in by:** 2026-05-18 row of TAXONOMY — "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A typed-object-element + W16.2-B trait-object-element + **W16.2-C empty-literal/spread/comprehension**. **The annotation_targets + annotations_comptime cluster IS THIS WORK.**" Reinforced 2026-05-21 ("Array<string> must work. Len trait. Object destructuring must fully work."). Bare `[]` literal usage is *exactly* the W16.2-C surface.
- **Exact SURFACE message text:** as quoted above.
- **(Incorrect) v0.4 anchor cited by SURFACE:** none cited explicitly — the SURFACE asks the user to add an annotation as a workaround, implying user-fixable. This is the "documented as out-of-scope" rationalization shape that the TAXONOMY hard-discipline section calls out. The work IS in v0.3 user-pull-in scope per the 2026-05-18 row.
- **Why incorrect:** the 2026-05-18 row names W16.2-C verbatim as v0.3 scope.
- **Test asserts on user-facing semantics** (post-fix the test stays the same — the test program is plausibly-correct Shape that any user would expect to work).

Tests in Shape A (38):
- `creation::empty_array_creation`
- `creation::empty_array_print`
- `methods::array_length_empty`
- `stress_access_length::test_array_built_in_loop`
- `stress_access_length::test_array_built_in_loop_values`
- `stress_access_length::test_array_push_multiple`
- `stress_access_length::test_array_push_to_empty`
- `stress_chained::test_concat_with_empty`
- `stress_chained::test_first_empty`
- `stress_chained::test_flatten_empty`
- `stress_chained::test_join_empty_array`
- `stress_chained::test_last_empty`
- `stress_creation::test_array_literal_empty`
- `stress_map_filter::test_filter_empty_array`
- `stress_map_filter::test_map_empty_array`
- `stress_map_filter::test_sort_empty`
- `stress_map_filter::test_unique_empty`
- `stress_mutation::test_array_concat_then_flatten`
- `stress_mutation::test_array_large_build_with_loop`
- `stress_mutation::test_array_large_first_element`
- `stress_mutation::test_array_large_last_element`
- `stress_mutation::test_array_large_index_access`
- `stress_reduce_fold::test_avg_empty`
- `stress_reduce_fold::test_count_empty`
- `stress_reduce_fold::test_every_empty`
- `stress_reduce_fold::test_find_index_empty`
- `stress_reduce_fold::test_find_on_empty`
- `stress_reduce_fold::test_flatmap_empty_results`
- `stress_reduce_fold::test_flatmap_on_empty`
- `stress_reduce_fold::test_some_empty`
- `stress_sort_find::test_empty_through_map_filter`
- `stress_sort_find::test_empty_through_sort_unique`
- `stress_sort_find::test_large_array_filter`
- `stress_sort_find::test_large_array_map`
- `stress_sort_find::test_large_array_sort_reverse`
- `stress_sort_find::test_large_array_unique`
- `stress_sort_find::test_order_by_empty`
- `stress_sort_find::test_select_empty`

---

### Shape B — V3-S5 ckpt-5 op_new_array / op_new_typed_array construction cascade (SCOPE-RECLAIM)

Verbatim error (binding to every Shape-B entry; only the arity `N` differs):
```
Runtime error: Not implemented: op_new_array(<N>): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum + `Buf<T>`
/ aligned-typed-buf wrapper layer + outer `HeapValue::TypedArray(Arc<_>)` arm
+ `HeapKind::TypedArray=8` ordinal DELETED across V3-S5 ckpt-1..ckpt-4 per
W12-typed-array-data-deletion audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A
SUPERSEDED. Post-deletion target is per-T v2-raw `TypedArray<T>` flat-struct
monomorphization per audit §A.3 + §3.1 scalar recipe + §2.2 heap-element
variants. Construction-site rebuild lands at ckpt-6 STRICT close after
ckpt-5-prime (wire/marshal/json + 4-table lockstep) + ckpt-5-prime² (storage
migration + 10 intrinsics marshal-parameter migration). REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1).
```

(`op_new_typed_array(N)` is the same SURFACE with the verb-arity replaced; it triggers for nested arrays where the element kind is itself `Ptr(HeapKind::TypedArray)`.)

SCOPE-RECLAIM fields:
- **Dated user disposition:** 2026-05-18 row of TAXONOMY — "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade." The SURFACE literally uses the phrase "V3-S5 ckpt-5 consumer-cascade" matching the dated row.
- **Exact SURFACE text:** as quoted.
- **Cited v0.4 anchor:** "ckpt-6 STRICT close" — but ckpt-6 is in the 2026-05-18 v0.3 pull-in.
- **Why incorrect:** identical phrase "V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade" appears in both the TAXONOMY 2026-05-18 row and the SURFACE text.
- **Test asserts on user-facing semantics.**

Tests in Shape B (39):
- `creation::array_literal_mixed_types`
- `creation::nested_array_2d`
- `creation::nested_array_3d`
- `stress_access_length::test_array_concat_both_empty`
- `stress_access_length::test_array_drop_from_empty`
- `stress_access_length::test_array_flatten_already_flat`
- `stress_access_length::test_array_flatten_basic`
- `stress_access_length::test_array_flatten_empty_array`
- `stress_access_length::test_array_flatten_empty_inner`
- `stress_access_length::test_array_flatten_first_value`
- `stress_access_length::test_array_flatten_last_value`
- `stress_access_length::test_array_flatten_mixed_nested`
- `stress_access_length::test_array_flatten_three_nested`
- `stress_access_length::test_array_includes_empty`
- `stress_access_length::test_array_index_of_empty`
- `stress_access_length::test_array_take_from_empty`
- `stress_chained::test_flatten_nested`
- `stress_creation::test_array_concat_empty_left`
- `stress_creation::test_array_first_empty_returns_none`
- `stress_creation::test_array_from_expression`
- `stress_creation::test_array_last_empty_returns_none`
- `stress_creation::test_array_length_empty`
- `stress_creation::test_array_length_nested_array`
- `stress_creation::test_array_literal_deeply_nested`
- `stress_creation::test_array_literal_mixed_int_float`
- `stress_creation::test_array_literal_nested`
- `stress_creation::test_array_reverse_empty`
- `stress_creation::test_nested_array_index`
- `stress_creation::test_nested_array_index_deep`
- `stress_mutation::test_array_empty_chaining`
- `stress_mutation::test_array_flatten_preserves_order`
- `stress_mutation::test_array_flatten_single_nested`
- `stress_mutation::test_array_flatten_then_join`
- `stress_mutation::test_array_for_in_empty`
- `stress_mutation::test_nested_array_flatten_and_index`
- `stress_mutation::test_nested_array_inner_first`
- `stress_mutation::test_nested_array_inner_last`
- `stress_mutation::test_nested_array_lengths`
- `stress_reduce_fold::test_flatmap_identity_nested`

---

### Shape C — V3-S5 ckpt-2 consumer cascade on TypedArray methods (SCOPE-RECLAIM)

Verbatim error (representative — `filter` shown; `map`, `flatMap`, `except`, `intersect`, `union`, `unique` differ only in the method name):
```
Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2 consumer-cascade
tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per
W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED.
The previous `Arc<TypedArrayData>` receiver-recovery + per-variant match-arm
dispatch path (~206 references across 11 public handlers in this file)
cascade-broke at the enum deletion site
(`crates/shape-value/src/heap_value.rs:3944`). Post-deletion target is the
v2-raw `TypedArray<T>` flat-struct carrier per audit §1.2 + §A.3 + §3.1 scalar
recipe + §2.2 heap-element variants; per-T monomorphization landing across
ckpt-3..ckpt-6. Receiver kind: Ptr(TypedArray). UNREACHABLE until ckpt-6
STRICT close. REFUSED ON SIGHT: TypedArrayData resurrection under any rename.
```

SCOPE-RECLAIM fields:
- **Dated user disposition:** 2026-05-18 row of TAXONOMY (V3-S5 cascade ckpt-2..ckpt-6 are all named).
- **Exact SURFACE text:** as quoted.
- **Cited v0.4 anchor:** "UNREACHABLE until ckpt-6 STRICT close" — ckpt-6 is in the 2026-05-18 v0.3 pull-in.
- **Why incorrect:** same TAXONOMY 2026-05-18 row applies (the whole V3-S5 ckpt-2..ckpt-6 cascade was named).
- **Tests assert on user-facing semantics.**
- **Note per user 2026-05-27 ruling:** filter-on-TypedObject family is FN-REG-CORRECTNESS, citing `crates/shape-runtime/stdlib-src/core/vec.shape:46-54` (the pure-Shape `extend Vec<T>` filter implementation that should compile to the v2-raw path without needing the deleted-enum receiver-recovery). **No test in this binary qualifies** for that FN-REG-CORRECTNESS carve-out — every failing filter / map / flatMap / set-op test here uses `Array<int>` / `Array<string>` / `Array<bool>` receivers, never `Array<TypedObject>`. If/when a TypedObject-element test were to surface this same SURFACE, it would route to FN-REG-CORRECTNESS, citing `vec.shape:46-54`. The note is logged here per audit-coverage discipline.

Tests in Shape C (16):
- `stress_chained::test_except_basic`
- `stress_chained::test_except_all_excluded`
- `stress_chained::test_intersect_basic`
- `stress_chained::test_intersect_disjoint`
- `stress_chained::test_union_basic`
- `stress_chained::test_union_disjoint`
- `stress_chained::test_pipeline_flatmap_filter_sum`
- `stress_reduce_fold::test_distinct_alias` (`unique` set-op)
- `stress_reduce_fold::test_flatmap_expand`
- `stress_reduce_fold::test_flatmap_single_element_arrays`
- `stress_reduce_fold::test_flatmap_triple`
- `stress_sort_find::test_chain_map_then_filter`
- `stress_sort_find::test_chain_map_filter_sort`
- `transforms::array_flatmap_expand`
- `transforms::array_flatmap_values`
- `transforms::array_map_to_objects`

---

### Shape D — V3-S5 ckpt-3 joinStr per-V2ElemType primitive (SCOPE-RECLAIM)

Verbatim error:
```
Runtime error: Not implemented: joinStr: SURFACE — per-V2ElemType element
stringification primitive not yet landed (separate ckpt-3 sub-cluster, not
J.5f scope per supervisor D4 2026-05-24). Use `.map(|x| x.toString()).reduce(
"", |acc, s| acc + sep + s)` as a pure-Shape workaround until the
`v2_array_detect::element_to_string` primitive lands.
```

SCOPE-RECLAIM fields:
- **Dated user disposition:** 2026-05-18 row (V3-S5 cascade names ckpt-3 explicitly: "per-T monomorphization landing across ckpt-3 (array_ops/typed_array_methods/iterator_methods/array_sort/concat/property_access)").
- **Exact SURFACE text:** as quoted.
- **Cited v0.4 anchor:** none — cites "separate ckpt-3 sub-cluster"; workaround mentioned but the construction-cascade including ckpt-3 is in v0.3 scope per the 2026-05-18 row.
- **Why incorrect:** the TAXONOMY 2026-05-18 row explicitly names ckpt-3.
- **Test asserts on user-facing semantics.**

Tests in Shape D (2):
- `stress_mutation::test_array_drop_then_join`
- `stress_mutation::test_array_take_then_join`

---

### Shape E — phase-2c kinded iterator API: iter_kind=Bool unsupported (SCOPE-RECLAIM)

Verbatim error:
```
Runtime error: Not implemented: op_iter_done SURFACE: iter_kind=Bool not
supported as iterator in the kinded API (legacy Array/Range/Iterator
HeapValue variants deleted) — phase-2c, see ADR-006 §2.7.4
```

SCOPE-RECLAIM fields:
- **Dated user disposition:** 2026-05-22 row of TAXONOMY — "Scope expansion: W16.2-J PHF-retirement + W17.3-4 per-container FieldType + **phase-2c host-tier marshal/snapshot rebuild** + 6 Known Constraints + doc-truth round."
- **Exact SURFACE text:** as quoted.
- **Cited v0.4 anchor:** cites only ADR-006 §2.7.4 (phase-2c) — phase-2c is in the 2026-05-22 v0.3 pull-in.
- **Why incorrect:** phase-2c is in the 2026-05-22 row.
- **Test asserts on user-facing semantics.**

Tests in Shape E (1):
- `stress_map_filter::test_map_with_index` (`.map((x, i) => ...)` — the index iterator is Bool-kinded; phase-2c kinded-iterator API gap)

---

### Shape F — pipeline / empty-array downstream "Cannot infer types" cascade (SCOPE-RECLAIM)

Verbatim error (representative — the message is often *co-emitted* with a Shape-A "empty array" error preceding it; the second-paragraph "Cannot infer types" is the distinguishing signature):
```
Semantic error: Cannot infer types for binary operation `Mul`: operand types
are `unknown` and `unknown`. Strict typing requires both operands to have a
known concrete type at compile time. Add a type annotation to disambiguate.
```

This is either (a) a downstream cascade of the Shape-A empty-array gap (3 tests — the reduce/sort empty-array tests carry both messages), or (b) a standalone type-inference loss through chained stdlib array operations on non-empty arrays (`stress_chained::test_pipeline_top_3_squares` exercises `[5,2,8,1,9,3].sort().reverse().take(3).map(|x| x*x)[0]` where element type is lost across the pipeline). Either way the root cause is the V3-S5 cascade + W16.2-C strict-typing surface, both in v0.3 user-pull-in scope per 2026-05-18 / 2026-05-21 rows. Same SCOPE-RECLAIM fields as Shape A (dated pull-in 2026-05-18 + 2026-05-21; SURFACE quoted; incorrect cite reason: the row names this exact cascade as v0.3 scope; test asserts on user-facing semantics).

Tests in Shape F (5):
- `stress_chained::test_pipeline_top_3_squares` (standalone — chained Array<int> pipeline loses element type)
- `stress_creation::test_array_first_last_same_on_single` (Equal — `[42].first() == [42].last()`, element type lost through `first()`/`last()` PHF return)
- `stress_map_filter::test_reduce_empty_returns_initial` (Add — downstream of empty `arr`)
- `stress_sort_find::test_large_array_reduce` (Add — downstream of empty `arr`)
- `stress_sort_find::test_large_pipeline` (Mul — downstream of empty `arr`)

---

### Shape G — negative-array-indexing / out-of-bounds semantic change (FN-REG-CORRECTNESS)

Verbatim error (representative):
```
Runtime error: Index -1 out of bounds (length 4) (line 3)
```

Tests assert `arr[-1]` returns the last element (JS / Python-style negative indexing) and `arr[10]` returns `None` for out-of-bounds (the existing test fixtures `array_out_of_bounds_*` literally `.expect_output("None")`). Current VM treats negative indices as out-of-bounds and raises a runtime error rather than wrapping; positive out-of-bounds raises rather than returning `None`. This is plausibly-correct user-facing Shape (`let arr = [10, 20, 30, 40]; print(arr[-1])`) that previously executed; behavior regressed.

FN-REG-CORRECTNESS fields:
- **Minimal repro:**
  ```shape
  let arr = [10, 20, 30, 40]
  print(arr[-1])
  ```
- **Bisected regression commit:** NOT BISECTED (audit-only; the change likely landed alongside the V3-S5 ckpt-3 array-ops migration — candidate file: `crates/shape-vm/src/executor/objects/typed_array_*` and `op_index` handlers).
- **Affected stdlib symbol / compiler subsystem:** the `op_index` handler family on `HeapKind::TypedArray` (`crates/shape-vm/src/executor/objects/`). Both wrap-negative and `None`-on-OOB semantics regressed at the same site.

Tests in Shape G (5):
- `indexing::array_negative_index_first_element` — `arr[-4]` expected `10`, got `Runtime error: Index -4 out of bounds (length 4)`
- `indexing::array_negative_index_last` — `arr[-1]` expected `40`, got `Runtime error: Index -1 out of bounds (length 4)`
- `indexing::array_negative_index_second_last` — `arr[-2]` expected `30`, got `Runtime error: Index -2 out of bounds (length 4)`
- `indexing::array_out_of_bounds_negative` — `arr[-10]` expected `None`, got `Runtime error: Index -10 out of bounds (length 3)`
- `indexing::array_out_of_bounds_positive` — `arr[10]` expected `None`, got `Runtime error: Index 10 out of bounds (length 3)`

---

### Shape H — `array.pop()` returns scalar; test reads `.length` on it (FN-REG-CORRECTNESS)

Verbatim error:
```
Runtime error: TypeError: expected array, object, or string, got scalar (line 4)
```

Test source (`methods.rs:53-64`) reads:
```shape
let arr = [1, 2, 3]
let popped = arr.pop()
print(popped.length)
```
The test comment says "pop() returns the array without the last element, not the removed element". Current `vec.shape:32` declares `method pop() -> T { self.pop() }` — returns the element. The user-facing program previously ran (per the audit-trigger evidence that v0.3.x tags shipped this test as passing); something in the stdlib / dispatch regressed. Class is FN-REG-CORRECTNESS because the program is plausibly-correct Shape under the previous-semantics contract.

FN-REG-CORRECTNESS fields:
- **Minimal repro:**
  ```shape
  let arr = [1, 2, 3]
  let popped = arr.pop()
  print(popped.length)
  ```
- **Bisected regression commit:** NOT BISECTED.
- **Affected stdlib symbol:** `std::core::vec::pop` (`crates/shape-runtime/stdlib-src/core/vec.shape:32`) + the corresponding PHF dispatch handler (the stdlib comment at `vec.shape:11-13` notes that `pop` is currently Rust-PHF-only after R8 W4 J.5f).

Tests in Shape H (1):
- `methods::array_pop`

---

### Shape I — `stress_chained::test_for_each_returns_none` forEach return semantic (FN-REG-CORRECTNESS)

Verbatim error:
```
Expected None/null, got: Object {"Integer": Number(3)}
```

`vec.shape:78-80` declares `method forEach(f: (T) => void) -> void`, so returning `Integer(3)` is a semantic regression — return value should be Unit/None per the stdlib contract. Plausibly-correct user-facing Shape.

FN-REG-CORRECTNESS fields:
- **Minimal repro:** see `tools/shape-test/tests/arrays_vectors/stress_chained.rs::test_for_each_returns_none`.
- **Bisected regression commit:** NOT BISECTED.
- **Affected stdlib symbol:** `std::core::vec::forEach` (`vec.shape:78-80`).

Tests in Shape I (1):
- `stress_chained::test_for_each_returns_none`

---

### Shape J — `stress_map_filter::test_filter_with_index` filter produces empty + crash (FN-REG-CORRECTNESS)

Verbatim error (with V2 bytecode verifier warning):
```
V2 bytecode verification warning: 2 violation(s) found
  - V2 typed opcode NewTypedArrayI64 at offset 4 in function 'Vec.filter::i64_closure_0_bool_bbdfe089ee892995' has no FrameDescriptor
  - V2 typed opcode TypedArrayPushI64 at offset 32 in function 'Vec.filter::i64_closure_0_bool_bbdfe089ee892995' has no FrameDescriptor

Runtime error: Index 0 out of bounds (length 0) (line 3)
```

`Array<int>.filter(...)` produces a zero-length result where the predicate intent should match — and the test then indexes [0] into the result, hitting OOB. This is the Shape-J **filter-on-Array<int>** correctness regression, distinct from the Shape-C SURFACE family (which fails at the *receiver-recovery* boundary before the filter body runs).

FN-REG-CORRECTNESS fields:
- **Minimal repro:** see `tools/shape-test/tests/arrays_vectors/stress_map_filter.rs::test_filter_with_index`.
- **Bisected regression commit:** NOT BISECTED.
- **Affected stdlib symbol:** `std::core::vec::filter` (`crates/shape-runtime/stdlib-src/core/vec.shape:46-54`) — same anchor as the user 2026-05-27 filter-on-TypedObject FN-REG-CORRECTNESS ruling; here the receiver is `Array<int>` but the same `extend Vec<T>` body is at fault.

Tests in Shape J (1):
- `stress_map_filter::test_filter_with_index`

---

### Shape K — `stress_mutation::test_array_for_in_strings` TypeError: expected string, got string (FN-REG-CORRECTNESS)

Verbatim error:
```
Runtime error: TypeError: expected string, got string (line 4)
```

Self-contradicting message — VM checks for `string` and reports the actual value's type as `string`, yet the check fails. This is a kinded-API string-carrier boundary mismatch (StringObj `*const _` vs raw `String` per `NativeKind::String` / `StringV2`). The user-facing program `for x in ["a","b","c"]` is plausibly-correct Shape.

FN-REG-CORRECTNESS fields:
- **Minimal repro:** see `tools/shape-test/tests/arrays_vectors/stress_mutation.rs::test_array_for_in_strings`.
- **Bisected regression commit:** NOT BISECTED.
- **Affected subsystem:** kinded-API string carrier boundary (`NativeKind::String` vs `NativeKind::StringV2` per ADR-006 §2.7) in `for x in <Array<string>>` iteration.

Tests in Shape K (1):
- `stress_mutation::test_array_for_in_strings`

---

### Shape L — `stress_access_length::test_array_join_no_separator_uses_comma` TypeError: expected string, got null (FN-REG-CORRECTNESS)

Verbatim error (with V2 verifier warning):
```
V2 bytecode verification warning: 2 violation(s) found
  - V2 typed opcode StringConcatTyped at offset 24 in function 'Vec.join::i64' has no FrameDescriptor
  - V2 typed opcode StringConcatTyped at offset 35 in function 'Vec.join::i64' has no FrameDescriptor

Runtime error: TypeError: expected string, got null (line 2)
```

`Array<int>.join()` (no separator arg) should default to comma; current behavior receives a null where a string is expected. Public-API regression on `.join()` no-arg form.

FN-REG-CORRECTNESS fields:
- **Minimal repro:** `[1, 2, 3].join()`.
- **Bisected regression commit:** NOT BISECTED.
- **Affected stdlib symbol:** `std::core::vec::join` no-arg dispatch.

Tests in Shape L (1):
- `stress_access_length::test_array_join_no_separator_uses_comma`

---

### Shape M — `methods::array_contains_*` no-method diagnostic text (FN-REG-DIAGNOSTIC)

| field | value |
|---|---|
| Old expected text (`methods.rs:107` / `:119`): | `Unknown method 'contains'` |
| New actual text (from current run): | `Runtime error: no method 'contains' on receiver kind Ptr(TypedArray)` |
| One-line note: | Receiver-kind-aware no-method diagnostic was added when the runtime moved to typed `Ptr(HeapKind)` dispatch (ADR-006 §2.7). The substring `Unknown method` was replaced with `no method 'X' on receiver kind Y`. Same semantic outcome (method not found), updated text. |

Tests in Shape M (2):
- `methods::array_contains_found`
- `methods::array_contains_not_found`

---

### Shape N — `methods::array_index_of` number-format diagnostic (FN-REG-DIAGNOSTIC)

| field | value |
|---|---|
| Old expected text (`methods.rs:131`): | `2.0` |
| New actual text: | `2` |
| One-line note: | `Array<int>.indexOf` now returns an `int` (rendered `2`); previously rendered `2.0` (likely f64 carrier). The strict-typing typed-array migration narrowed the indexOf return path from f64-carrier to i64-carrier per the V3-S5 ckpt-3 array-ops monomorphization; the test fixture's expected string literal is stale. Same semantic outcome (index of `30` in `[10,20,30,40]` is 2). |

Tests in Shape N (1):
- `methods::array_index_of`

---

## UNKNOWN

(empty — every failing test in this binary is classified with verbatim-error backing per the run-verify binding rule.)
