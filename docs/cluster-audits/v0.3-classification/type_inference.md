# type_inference classification

**HEAD:** 82f049dd
**Total tests in binary:** 295
**Passed:** 260 / Failed: 35 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test type_inference --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 17 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 18 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Strict-typing inference is the core v0.3 promise. Most failures here are
plausibly-correct user-facing Shape that previously worked: nested
function inference returning 0 instead of 30, `typeof` returning a
`Discriminant(15)` runtime crash, `String.substring` ignoring its `end`
arg, HashMap value-kind contradiction, and `Cannot infer` errors on
chained method calls. Those route to **FN-REG-CORRECTNESS** — they are
not v0.4 territory, and no SURFACE message defers them. Failures whose
SURFACE text cites V3-S5 ckpt-2..ckpt-5 / W12 TypedArrayData deletion /
`op_new_array(N)` / `op_new_typed_array(N)` / `range` consumer-cascade /
`map`/`filter` consumer-cascade / `String.split` carrier rebuild all
route to **SCOPE-RECLAIM** per the 2026-05-18 dated user disposition
(V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade + W16.2-A/B/C);
2026-05-21 ("Array<string> must work"); 2026-05-22 (W17.3-4 per-container
FieldType + phase-2c host-tier marshal/snapshot rebuild). None of the
SURFACE-tagged failures cite §5.16 — no SURFACE-cite mis-routings to
v0.4 here.

## Per-test classification

### basic::test_infer_nested_function_calls

Class: **FN-REG-CORRECTNESS**

Repro:
```
fn add(a, b) { a + b }
fn double(x) { x * 2 }
double(add(5, 10))
```

Expected `30`, got `0.000…015` (denormal noise). Silent-wrong-output on
two-arg untyped function inference. No SURFACE, no error — the program
compiles and runs and returns garbage. This is the highest-severity
class of regression (correctness, not crash). Affected subsystem:
function-parameter type inference + `AddInt`/`MulInt` typed-opcode
selection on untyped params. Bisect TBD.

### collections::test_array_empty_length

Class: **FN-REG-CORRECTNESS**

Repro:
```
let a = []
a.length
```

Expected `0`. Actual:
```
Semantic error: empty array `a` has an un-resolvable element type. It
is created empty (`[]`) with no `Array<T>` annotation and is never
pushed to, so the compiler cannot prove what element type it holds.
Strict typing requires a known concrete element type: add an annotation
(`let a: Array<T> = []`) or remove the unused binding.
```

Plausibly-correct user-facing Shape: `[].length == 0` regardless of
element type. Strict typing should not refuse `.length` on an
un-pushed empty array — `length` is element-type-agnostic. Subsystem:
empty-array element-type inference (`compiler/array_literal_empty.rs`
or equivalent). Note: 2026-05-18 W16.2-C explicitly named "empty-literal";
this could plausibly route to SCOPE-RECLAIM, but the SURFACE here is a
plain `Semantic error`, not a V3-S5 SURFACE. Classified as FN-REG-CORRECTNESS
because (a) the SURFACE doesn't self-identify as ckpt-5/ckpt-6 work and
(b) `.length` on an empty literal is a strictly weaker requirement than
W16.2-C element-type resolution — it should succeed without knowing T.

### collections::test_array_nested_access

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_typed_array(2): SURFACE — V3-S5
ckpt-5 consumer-cascade tier 3 surface. … REFUSED ON SIGHT:
TypedArrayData resurrection under any rename (Refusal #1). (line 2)
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade.
- v0.4 anchor cited: none.
- Why mis-cite would be incorrect: SURFACE correctly names ckpt-5.
- Test asserts on user-facing semantics (`nested[1][0] == 3`); test
  stays the same after fix.

### collections::test_hashmap_delete_key

Class: **FN-REG-CORRECTNESS**

Repro:
```
let m = HashMap().set("a", 1).set("b", 2)
let m2 = m.delete("a")
m2.has("a")
```

Expected `false`. Actual:
```
Semantic error: Cannot reassign immutable variable 'm'. Use `let mut`
or `var` for mutable bindings
```

`m` is not reassigned anywhere in the program — `m2 = m.delete("a")`
binds a NEW variable. The compiler is mis-classifying a chained method
call `.delete()` as a reassignment of the receiver. Subsystem: borrow
solver / `BindingStorageClass` handling of `HashMap.delete()` (likely
mis-tagged as `&mut self` receiver and confusing the immutability gate).

### collections::test_hashmap_delete_preserves_other_keys

Class: **FN-REG-CORRECTNESS**

Repro:
```
let m = HashMap().set("a", 1).set("b", 2)
let m2 = m.delete("a")
m2.get("b")
```

Same diagnostic as `test_hashmap_delete_key`:
`"Cannot reassign immutable variable 'm'"` though `m` is never
reassigned. Same subsystem.

### collections::test_hashmap_in_function

Class: **FN-REG-CORRECTNESS**

Repro:
```
fn make_config() {
    HashMap().set("host", "localhost").set("port", 8080)
}
let cfg = make_config()
cfg.get("host")
```

Expected `"localhost"`. Actual:
```
Runtime error: HashMap.set(): value kind Int64 incompatible with
HashMap<string, string> (line 3)
```

The compiler inferred `HashMap<string, string>` from the first `.set`
call's literal `"localhost"`, then refused the second `.set("port", 8080)`
because `8080: int`. This is plausibly-correct user-facing Shape:
`HashMap` should default to a heterogeneous value type (`Any` /
union) OR the inference rule should treat the chain as one
constructor-shape group. Subsystem: HashMap value-type inference in
chained-builder pattern.

### complex::test_complex_accumulate_with_hashmap

Class: **FN-REG-CORRECTNESS**

Repro:
```
let m = HashMap()
    .set("apples", 3)
    .set("bananas", 5)
    .set("oranges", 2)
let total = m.get("apples") + m.get("bananas") + m.get("oranges")
total
```

Expected `10`. Actual:
```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `unknown`. Strict typing requires both operands
to have a known concrete type at compile time.
```

`HashMap.get()` return type is not propagated to the binary-op operand
position even when the receiver has a known `HashMap<string, int>`
type. Subsystem: bidirectional inference from method return-type to
binary-op operand.

### complex::test_complex_bubble_sort

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. … REFUSED ON SIGHT: TypedArrayData
resurrection under any rename (Refusal #1). (line 11)
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6.
- Test uses `.slice` / `.concat` array-rebuild ops — squarely
  construction-cascade.
- Test asserts on user-facing semantics.

### complex::test_complex_data_processing_mixed_types

Class: **FN-REG-CORRECTNESS**

Repro (4-element string array → filter → map → join):
```
let names = ["Alice", "Bob", "Charlie", "Diana"]
let greeting = names
    .filter(|n| n.length > 3)
    .map(|n| "Hello " + n)
    .join("; ")
greeting
```

Expected `"Hello Alice; Hello Charlie; Hello Diana"`. Actual:
```
Runtime error: TypeError: expected string, got string (line 8)
```

The diagnostic is self-contradictory ("expected string, got string").
2026-05-21 explicitly pulled `Array<string>` into v0.3, so the user
expectation that this works is dated. The V2-bytecode-verification
warnings on `NewTypedArrayString` / `TypedArrayPushString` /
`StringConcatTyped` "has no FrameDescriptor" suggest the string-typed
array path mis-tags one of the string values mid-pipeline.

### complex::test_complex_nested_array_flatten

Class: **FN-REG-CORRECTNESS**

Repro:
```
let nested = [[1, 2], [3, 4], [5, 6]]
let flat = nested.flatten()
flat.reduce(|acc, x| acc + x, 0)
```

Expected `21`. Actual:
```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `unknown`.
```

Closure params `|acc, x|` to `.reduce` should infer from the array
element type and the seed `0`. Subsystem: bidirectional closure-param
inference for `.reduce` (compare the SAME pattern works for `.map`
according to other passing tests). Note: `nested.flatten()` is not in
the dated pull-in list; classified as a correctness regression on
bidirectional inference, not as SCOPE-RECLAIM (it could also be a
missing `.flatten` stdlib, but the diagnostic is about Add, not about
missing-method).

### complex::test_complex_string_processing_pipeline

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: String.split: SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data String
`Arc<Buf<Arc<String>>>` result carrier DELETED at V3-S5 ckpt-1..ckpt-4
… REFUSED ON SIGHT: TypedArrayData resurrection under any rename
(Refusal #1).
```

- Dated pull-in: 2026-05-18 (ckpt-5 / ckpt-6) + 2026-05-21
  (Array<string> must work — `String.split` returns
  `Array<string>`) + 2026-05-22 (W17.3-4 per-container FieldType).
- v0.4 anchor cited: none (correctly routes through ckpt-6 STRICT close).
- Test asserts on user-facing semantics.

### stress_annotations::fn_mixed_param_types

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3
consumer-cascade tier 2 surface. `TypedArrayData` enum DELETED at
ckpt-1 (2026-05-15) per W12-typed-array-data-deletion audit §3.5 +
ADR-006 §2.7.24 Q25.A SUPERSEDED. … UNREACHABLE until ckpt-6 STRICT
close. REFUSED ON SIGHT: TypedArrayData resurrection under any rename.
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 construction-cascade
  (`range` is in the ~105-reference consumer cascade) + 2026-05-22 W17.3-4
  per-container FieldType.
- v0.4 anchor cited: none.
- Test asserts on user-facing semantics (`"ababab"`).

### stress_annotations::typed_for_loop_accumulator

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3
consumer-cascade tier 2 surface. … UNREACHABLE until ckpt-6 STRICT close.
```

Same SURFACE as `fn_mixed_param_types`; same dated pull-ins. Test asserts
on user-facing semantics (`sum == 45`).

### stress_generics::generic_fn_with_array_return

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: op_new_array(1): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. … REFUSED ON SIGHT: TypedArrayData
resurrection under any rename.
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array
  construction-cascade (literal `[x]` construction inside a generic
  fn).
- Test asserts on user-facing semantics (`arr[0] == 42`).

### stress_generics::generic_identity_with_null

Class: **FN-REG-CORRECTNESS**

Repro:
```
fn id<T>(x: T) -> T { return x }
fn test() {
    let x = id(None)
    return x == None
}
test()
```

Expected `true`. Actual:
```
Semantic error: cannot infer type argument(s) for generic function
'id' from the call-site arguments — annotate the arguments or call with
values whose types are statically known
```

`None` should infer to `Option<T>` for some `T`, and `id(None)` should
return `Option<T>`. The generic inference engine refuses `None` as a
type-argument source. Plausibly-correct user-facing Shape; subsystem:
generic inference of `Option<T>` from `None` literal.

### stress_generics::generic_runtime_type_via_type_method

Class: **FN-REG-CORRECTNESS**

Repro:
```
fn inner<T>(x: T) { return x.type().to_string() }
fn test() { return inner(2.1) }
test()
```

Expected `"number"`. Actual:
```
Runtime error: unsupported constant variant in PushConst (Wave 6
follow-up): Discriminant(15) (line 3)
```

`Discriminant(15)` is a VM-level crash, not a SURFACE-and-stop. The
SURFACE text literally cites "Wave 6 follow-up" but per TAXONOMY a
"Wave 6 follow-up" rationalization without a dated user re-disposition
to v0.4 is a defection-attractor framing. `.type()` is a fundamental
introspection builtin — not in the V3-S5 carrier cascade, not in any
dated v0.4 pull-out, and the failure shape is a runtime crash on a
constant the compiler emitted. Routes to FN-REG-CORRECTNESS.

### stress_generics::generic_runtime_type_via_type_method_int

Class: **FN-REG-CORRECTNESS**

Same crash as above (`Discriminant(15)`), expected `"int"` from
`inner(42)`. Same classification rationale.

### stress_generics::generic_struct_type_name_with_default

Class: **FN-REG-CORRECTNESS**

Repro:
```
type MyType<T = int> { x: T }
fn test() {
    let a = MyType { x: 1 }
    return a.type().to_string()
}
test()
```

Expected `"MyType"`. Actual: `Discriminant(15)` runtime crash. Same
class as the two preceding `.type()` crashes.

### stress_generics::generic_struct_type_name_with_non_default

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash; expected `"MyType<number>"` from
`MyType { x: 1.0 }.type().to_string()`. Same class.

### stress_inference::type_preserved_through_loop

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3
consumer-cascade tier 2 surface. … UNREACHABLE until ckpt-6 STRICT close.
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-3/ckpt-5/ckpt-6 (`range`
  consumer-cascade).
- Test asserts on user-facing semantics (`sum == 10`).

### stress_inference::typeof_int_via_type_method

Class: **FN-REG-CORRECTNESS**

```
Runtime error: unsupported constant variant in PushConst (Wave 6
follow-up): Discriminant(15) (line 2)
```

Same `Discriminant(15)` runtime crash on `.type()`. Same class as the
`stress_generics::generic_runtime_type_via_type_method` family.

### stress_inference::typeof_number_via_type_method

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`. Same class.

### stress_inference::typeof_array_via_type_method

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`. Same class. V2-bytecode
verification warning emitted about `NewTypedArrayI64` missing
`FrameDescriptor` — a parallel-cascade artifact but the underlying
failure is the same `.type()` crash.

### stress_inference::typeof_bool_via_type_method

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`.

### stress_inference::typeof_string_via_type_method

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`.

### stress_inference::typeof_struct_via_type_method

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`.

### stress_inference::typeof_struct_on_type_symbol

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`.

### stress_inference_complex::struct_type_name_via_instance

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()`.

### stress_inference_complex::struct_type_name_via_symbol

Class: **FN-REG-CORRECTNESS**

Same `Discriminant(15)` crash on `.type()` (via `Bar.type()` —
type-symbol form rather than instance form, same crash).

### stress_inference_complex::typed_closure_in_array_filter

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2
consumer-cascade tier 1 surface. `TypedArrayData` enum DELETED at
ckpt-1 (2026-05-15) per W12-typed-array-data-deletion audit §3.5 +
ADR-006 §2.7.24 Q25.A SUPERSEDED. … UNREACHABLE until ckpt-6 STRICT
close. REFUSED ON SIGHT: TypedArrayData resurrection under any rename.
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-2/ckpt-5/ckpt-6 (`filter`
  consumer-cascade — exactly the "filter on Array<User> rejected"
  example named in the TAXONOMY preamble).
- v0.4 anchor cited: none.
- Test asserts on user-facing semantics (`evens.length == 2`).

### stress_inference_complex::typed_closure_in_array_map

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2
consumer-cascade tier 1 surface. … UNREACHABLE until ckpt-6 STRICT close.
```

- Dated pull-in: 2026-05-18 V3-S5 ckpt-2/ckpt-5/ckpt-6 (`map`
  consumer-cascade).
- Test asserts on user-facing semantics (`doubled[2] == 6`).

### strings::test_string_split_basic

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: String.split: SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data String
`Arc<Buf<Arc<String>>>` result carrier DELETED at V3-S5 ckpt-1..ckpt-4
… REFUSED ON SIGHT: TypedArrayData resurrection under any rename.
```

- Dated pull-in: 2026-05-18 (ckpt-5 / ckpt-6) + 2026-05-21
  (Array<string> must work) + 2026-05-22 (W17.3-4 per-container
  FieldType).
- Test asserts on user-facing semantics.

### strings::test_string_split_first_element

Class: **SCOPE-RECLAIM**

Same `String.split` SURFACE as `test_string_split_basic`. Same dated
pull-ins. Test asserts on user-facing semantics.

### strings::test_string_substring_basic

Class: **FN-REG-CORRECTNESS**

Repro:
```
"hello".substring(1, 3)
```

Expected `"el"`, got `"ello"`. `String.substring(start, end)` is
ignoring the `end` argument (or treating second arg as `count`, which
would also be wrong since count=3 from start=1 yields "ell" not "ello").
Actual behavior looks like the second arg is being dropped entirely.
Silent-wrong-output on a fundamental string builtin. Subsystem:
`String.substring` PHF handler argument binding.

### strings::test_string_substring_from_start

Class: **FN-REG-CORRECTNESS**

Repro:
```
"hello world".substring(0, 5)
```

Expected `"hello"`, got `"hello world"`. Same `String.substring`
end-arg dropping bug as above; classified the same.

## UNKNOWN list

(none)
