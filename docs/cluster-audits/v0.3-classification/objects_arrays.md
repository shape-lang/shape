# objects_arrays classification

**HEAD:** 82f049dd
**Total tests in binary:** 94
**Passed:** 63 / Failed: 30 / Ignored: 1
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test objects_arrays --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 19 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 10 |
| V0.4-DEFER         | 1 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Failure shapes:

1. **Negative indexing + OOB-returns-None regression (7 tests)** — VM
   `op_get_elem_i64` (`v2_handlers/typed_array_elem.rs:116`) now `return
   Err(IndexOutOfBounds)` for any `index < 0`. The
   `nums[-1]` / `nums[5]` semantics documented inline in the fixture
   ("Array out-of-bounds returns None in Shape") are deleted at the v2-raw
   typed-array element-access site. Not in any dated v0.3 pull-in →
   **FN-REG-CORRECTNESS** (~7 tests).
2. **V3-S5 ckpt-5/ckpt-6 op_new_array / op_new_typed_array SURFACEs
   (4 tests)** — bare array-literal / nested-array construction. 2026-05-18
   user disposition names this work explicitly → **SCOPE-RECLAIM**.
3. **map / flatMap V3-S5 ckpt-2 typed-array consumer-cascade SURFACE on
   `Array<{...object literal}>` (2 tests)** — W16.2-A typed-object-element
   per 2026-05-18 → **SCOPE-RECLAIM**.
4. **`array_of_objects_with_filter` (1 test)** — user's repro family per
   task brief → **FN-REG-CORRECTNESS** (vec.shape:46-54 filter body +
   R8 W4 J.5b commit family `91a6df21`).
5. **empty array element-type un-resolvable (2 tests)** — W16.2-C
   empty-literal per 2026-05-18 → **SCOPE-RECLAIM**.
6. **Destructuring in function params (2 tests)** — 2026-05-21 "Object
   destructuring must fully work" → **SCOPE-RECLAIM** (object form by
   name; array form analogically).
7. **HashMap fixture: `let m = HashMap()` chain rejected as "cannot
   assign to immutable binding" (8 tests)** — immutable `.set()` returning
   a new map is the documented stdlib API (`hashmap_methods.shape:9`,
   `method set(...) -> HashMap<K, V>`); rebuild misclassifies the call
   shape → **FN-REG-CORRECTNESS**.
8. **HashMap integer keys rejected (1 test)** — `HashMap key must be a
   string (got kind Int64)` → **FN-REG-CORRECTNESS**.
9. **Array<string> reduce TypeError "expected string, got string"
   (1 test)** — 2026-05-21 "Array<string> must work" + StringConcatTyped
   FrameDescriptor missing per V2 warning emitted in the same output →
   **SCOPE-RECLAIM**.
10. **`array_concatenation_with_plus`** — surface cites `v0.4 / planned`
    explicitly for `IntrinsicVecAddI64` (handle_vector_intrinsic) and
    surface-and-stops cleanly. No dated v0.3 pull-in covers array `+`
    operator → **V0.4-DEFER**.
11. **`object_property_assignment` + `array_destructuring_in_function_param`**
    — static-field assignment rejected; array-destructure binding kinds
    lost → **FN-REG-CORRECTNESS** (mirrors existing `objects.md`
    `object_computed_key` classification).

All tests assert on user-facing semantics (output values), so post-fix
the fixtures stay unchanged.

## Per-test classification

### arrays::array_negative_indexing_last

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index -1 out of bounds (length 4) (line 2)")
```

Minimal repro:
```shape
let nums = [1, 2, 3, 4]
print(nums[-1])
```
Expected `4`. VM `op_get_elem_i64` at
`crates/shape-vm/src/executor/v2_handlers/typed_array_elem.rs:116` now
unconditionally errors on `index < 0`. Subsystem: v2-raw typed-array
element-access (`op_get_elem_i64` / `_f64`). Bisect: `git log --oneline
-- crates/shape-vm/src/executor/v2_handlers/typed_array_elem.rs` —
W16.2-J.1 / V3-S5 ckpt-3 monomorphization era.

### arrays::array_negative_indexing_second_last

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index -2 out of bounds (length 4) (line 2)")
```
Same shape; `nums[-2]` expected `3`. Same site / subsystem as above.

### arrays::array_negative_index_first_element

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index -4 out of bounds (length 4) (line 2)")
```
`nums[-4]` on 4-element array should give `1`. Same site / subsystem.

### arrays::array_of_strings_negative_index

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index -1 out of bounds (length 3) (line 2)")
```
Same family on `TypedArray<*const Str>` (string element variant). Same
root cause.

### arrays::array_out_of_bounds_positive_returns_none

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index 5 out of bounds (length 3) (line 2)")
```
Test asserts (and fixture comment documents) "Array out-of-bounds returns
None in Shape (runtime prints \"None\")". The v2-raw element-access path
now hard-errors instead of returning `Option::None`. Same site
(typed_array_elem.rs:130 / 148 / 161).

### arrays::array_out_of_bounds_negative_returns_none

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: Index -5 out of bounds (length 3) (line 2)")
```
Same family. Negative-OOB also expected to return `None` per the
fixture's own comment. Same site / subsystem.

### arrays::array_of_mixed_types

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(3): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. ... Construction-site rebuild lands at
ckpt-6 STRICT close after ckpt-5-prime ... REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1). (line 1)
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade).
- SURFACE: `op_new_array(3): SURFACE — V3-S5 ckpt-5 consumer-cascade
  tier 3 surface`.
- v0.4 anchor cited: none (correctly pins itself to ckpt-6 STRICT close).
- Asserts on: user-facing semantics (`print(mixed)` runs). Stays the same
  after fix.

### arrays::nested_arrays

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_typed_array(2): SURFACE — V3-S5
ckpt-5 consumer-cascade tier 3 surface. ... ckpt-6 STRICT close after
ckpt-5-prime ... (line 1)
```
Same V3-S5 ckpt-5/ckpt-6 row (2026-05-18). Asserts on user-facing
semantics.

### arrays::nested_array_access

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_typed_array(2): SURFACE — V3-S5
ckpt-5 consumer-cascade tier 3 surface. ... (line 1)
```
Same family. Asserts on user-facing semantics.

### arrays::nested_array_second_row_access

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_typed_array(2): SURFACE — V3-S5
ckpt-5 consumer-cascade tier 3 surface. ... (line 1)
```
Same family. Asserts on user-facing semantics.

### arrays::array_of_objects_with_map

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2
consumer-cascade tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1
(2026-05-15) per W12-typed-array-data-deletion audit §3.5 + ADR-006
§2.7.24 Q25.A SUPERSEDED. ... Receiver kind: Ptr(TypedArray).
UNREACHABLE until ckpt-6 STRICT close. ... (line 5)
```

- Dated pull-in: 2026-05-18 (V3-S5 ckpt-5/ckpt-6; W16.2-A
  typed-object-element receiver).
- SURFACE: `map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface`.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (`names.map(|u| u.name)` returns list
  of names).

### arrays::array_flatmap

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: flatMap: SURFACE — V3-S5 ckpt-2
consumer-cascade tier 1 surface. ... UNREACHABLE until ckpt-6 STRICT
close. ... (line 2)
```
Same V3-S5 ckpt-2/ckpt-6 family. Asserts on user-facing semantics.

### arrays::array_of_objects_with_filter

Class: **FN-REG-CORRECTNESS** — **user's repro family per task brief.**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Greater`: operand types are `unknown` and `int`. Strict
typing requires both operands to have a known concrete type at compile
time. Add a type annotation to disambiguate.")
```

Minimal repro:
```shape
let users = [
  { name: "Ada", score: 90 },
  { name: "Bob", score: 80 },
  { name: "Charlie", score: 95 }
]
let top = users.filter(|u| u.score > 85)
print(top.len())
```
Expected `2`. The closure body `u.score > 85` fails type inference
because `u: Object{name: string, score: int}` element-type is not
propagated into the `filter` predicate's `T` binding via the producer-
side stamp. The user-visible filter implementation
(`crates/shape-runtime/stdlib-src/core/vec.shape:46-54`,
`method filter(predicate: (T) => bool) -> Vec<T> { let mut result = []
for item in self { if predicate(item) { result.push(item) } } result }`)
relies on `T` being substituted with the receiver's stamped element type
at the call site — which it is for `Array<int>` / `Array<string>` but
not for object-literal element types.
Affected subsystem: bidirectional closure-param inference for
HOF receivers carrying object-typed elements
(`crates/shape-runtime/src/type_system/`).
Bisect: R8 W4 J.5b commit family — `91a6df21` (HOF builders two-pass
scan-then-allocate + closure-return-kind structured Err) +
`166f9d3e` (merge). Same `Vec.filter` body that J.5b rebuilds works
for scalar element types but not for inline-object element types
post-rebuild. **This is the v0.3.2 playground crash class.**

### arrays::array_concatenation_with_plus

Class: **V0.4-DEFER**

```
Expected run ok, got error: Some("Runtime error: Not implemented:
phase-1b-vm-wave-5d-vec-intrinsic: IntrinsicVecAddI64 body migration
to kinded carrier (handle_vector_intrinsic) pending (v0.4 / planned)
(line 3)")
```

- Brief v0.4 reason: array `+` operator delegates to
  `IntrinsicVecAddI64` intrinsic whose body migration to the kinded
  carrier per phase-1b-vm-wave-5d is explicitly tagged `v0.4 / planned`.
- Surface-and-stop is clean: `Not implemented:` structured runtime error
  naming the feature + v0.4 annotation (no panic, no SEGFAULT, no
  silent-wrong-output).
- No dated 2026-05-18..2026-05-26 user disposition names array `+`
  operator concat as v0.3-gating.
- Recommended issue: `TBD-v0.4-array-plus-intrinsic-migration`.

### arrays::empty_array

Class: **SCOPE-RECLAIM**

```
Semantic error: empty array `a` has an un-resolvable element type. It is
created empty (`[]`) with no `Array<T>` annotation and is never pushed
to, so the compiler cannot prove what element type it holds. ...
```

- Dated pull-in: 2026-05-18 (W16.2-C empty-literal/spread/comprehension).
- SURFACE: empty-array element-type un-resolvable; no v0.4 anchor.
- Asserts on: user-facing semantics (`print(a)` runs on empty array).

### arrays::empty_array_len

Class: **SCOPE-RECLAIM**

```
Semantic error: empty array `a` has an un-resolvable element type. ...
```
Same W16.2-C empty-literal pull-in. Same surface.

### arrays::array_destructuring_in_function_param

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ...
```

Repro:
```shape
fn sum_pair([a, b]) { return a + b }
print(sum_pair([10, 20]))
```
Expected `30`. The array-destructure binding kinds for `a` and `b` are
not propagated from the call-site element type into the function param
binding — symmetrical to the `objects.rs` `destructuring_in_function`
SCOPE-RECLAIM (2026-05-21 "Object destructuring must fully work"), but
the 2026-05-21 row names objects specifically. Array-destructure-in-
fn-param is not in any dated pull-in; classifying as correctness
regression. Subsystem: compiler param-destructure
inference (`crates/shape-vm/src/compiler/` destructure path +
type_system param binding kind propagation).

### arrays::array_reduce_string_concat

Class: **SCOPE-RECLAIM**

```
V2 bytecode verification warning: 1 violation(s) found
  - V2 typed opcode StringConcatTyped at offset 34 in function
    'Vec.reduce::string_string_closure_0_string_b98b800379e618868' has
    no FrameDescriptor
... Expected run ok, got error: Some("Runtime error: TypeError: expected
string, got string (line 2)")
```

- Dated pull-in: 2026-05-21 ("Array<string> must work") covers the
  Vec.reduce-over-Array<string> + StringConcatTyped FrameDescriptor
  emission path.
- SURFACE-ish: V2 bytecode verifier warns missing FrameDescriptor;
  runtime then emits a `TypeError: expected string, got string` (kind
  metadata lost — string-vs-string fails because the slot's NativeKind
  didn't propagate). Not a panic, not a SEGFAULT.
- Asserts on: user-facing semantics (`words.reduce(|acc, w| acc + w, "")`
  → `"hello world"`).

### objects::object_property_assignment

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Assignment to
'user.score' requires compile-time field resolution. Generic runtime
property lookup is disabled.")
```

Repro:
```shape
let mut user = { id: 1, name: "Ada" }
user.score = 99
print(user.score)
```
Expected `99`. Adding a previously-unknown field to a `let mut` object
literal is rejected. Same class as `objects.md` `operations::object_computed_key`
finding — bytecode-compiler SetProp lowering on object-literal-typed
receivers no longer accepts new-field assignment. Subsystem:
`crates/shape-vm/src/compiler/` SetProp / TypedObject schema-extension
path (post-W16.2 PHF-retirement era).

### objects::destructuring_in_function_param

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ...
```

- Dated pull-in: 2026-05-21 "Object destructuring must fully work."
- SURFACE: generic strict-typing error because destructured-param path
  doesn't propagate call-site `{x: int, y: int}` into the `{x, y}`
  binding kinds.
- v0.4 anchor cited: none.
- Asserts on: user-facing semantics (`distance({x:3,y:4})` → `7`).

### objects::hashmap_basic_creation_and_get

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: cannot assign to
immutable binding 'm'\n\nSemantic error: Cannot reassign immutable
variable 'm'. Use `let mut` or `var` for mutable bindings")
```

Repro:
```shape
let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.get("b"))
```
Expected `2`. The fixture does not reassign `m`. `HashMap.set` returns
a *new* `HashMap<K, V>` per documented stdlib
(`crates/shape-runtime/stdlib-src/core/hashmap_methods.shape:9`,
`method set(key: K, value: V) -> HashMap<K, V>`). The compiler is
treating the chained method call as an in-place mutation requiring
`let mut`. Subsystem: compiler immutable-binding analysis vs.
HashMap-method-return-type plumbing (the post-bee4f137 C2-joint ckpt-3
HashMap per-V mutation API rebuild likely flipped the call-shape
classification for self-receiver method calls returning `Self`).
Bisect: `git log --oneline -- crates/shape-vm/src/compiler/` around
HashMap method registration.

### objects::hashmap_has_key

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same shape as `hashmap_basic_creation_and_get`. Same root cause.

### objects::hashmap_has_missing_key

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause.

### objects::hashmap_delete

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot reassign immutable variable 'm'. ...
```
Same root cause. `delete` returns new `HashMap<K, V>`.

### objects::hashmap_entries

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause.

### objects::hashmap_immutability

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause. (Note the fixture is *literally testing* HashMap
immutability semantics — the regression rejects the exact pattern
the test pins.)

### objects::hashmap_keys

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause.

### objects::hashmap_values

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause.

### objects::hashmap_len

Class: **FN-REG-CORRECTNESS**

```
Semantic error: cannot assign to immutable binding 'm' ...
```
Same root cause.

### objects::hashmap_integer_keys

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: HashMap key must be a
string (got kind Int64) (line 2)")
```

Repro:
```shape
let scores = HashMap()
    .set(1, "gold")
    .set(2, "silver")
    .set(3, "bronze")
print(scores.get(1))
```
Expected `gold`. Stdlib signature is `method set(key: K, value: V)` —
generic over `K`. Runtime rejects non-string keys. The `K = int`
specialization isn't being emitted for HashMap. Subsystem: HashMap PHF
dispatch + per-K monomorphization (post-W16.2-J.1 era — per-kind PHF
registry deletion likely collapsed the K-monomorphization for HashMap
keys to string-only).

## UNKNOWN list

None.
