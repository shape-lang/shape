# traits classification

**HEAD:** 82f049dd
**Total tests in binary:** 195
**Passed:** 159 / Failed: 36 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test traits --no-fail-fast 2>&1`
**Wall time:** 1358.17s

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 34 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 2 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Failure-shape grouping (read once, applies to many entries below)

The 34 FN-REG-CORRECTNESS failures cluster into three regression-shapes,
all squarely inside dated v0.3 user-pull-in scope.

- **Shape A — operator-overload trait dispatch broken** (16 tests). Source
  is `let c = a + b` (or `-` / `*` / `/`) where `a, b: SomeType` and the
  user defined `impl Add for SomeType { method add(other) -> SomeType {
  ... } }`. Compiler emits: `Semantic error: Cannot infer types for
  binary operation 'Add': operand types are 'unknown' and 'unknown'.`
  The operator trait `+` is not resolving to the user `impl`. Operator
  trait coverage was the **v0.3 W1 headline** (per task brief);
  regression in W1-scope work is FN-REG-CORRECTNESS.

- **Shape B — Display trait `.to_string()` not dispatching to user
  `display()`** (8 tests). Source: `impl Display for T { method
  display() { "User:" + self.name } }; let u = T{...}; u.to_string()`.
  Output is the default debug repr `{name: "Alice"}`, not `User:Alice`.
  Bytecode verifier also emits `V2 typed opcode StringConcatTyped at
  offset N in function 'T::display' has no FrameDescriptor` warning
  — display() body IS being compiled, but to_string() is not routed
  to it. Display-trait dispatch was W1 scope; FN-REG-CORRECTNESS.

- **Shape C — trait method return type not flowed into caller** (6
  tests, stress_default + a few stress_dispatch_advanced). Source: a
  trait default method body does `"Object: " + self.name()` where
  `name() -> string` is declared in the trait. Compiler reports
  `string + unknown` (or `unknown + string`). Trait method return-type
  not threaded through the type inference at the binop. W1 scope;
  FN-REG-CORRECTNESS.

The 2 SCOPE-RECLAIM failures route through a SURFACE message that
explicitly cites V3-S5 ckpt-2/ckpt-3 consumer-cascade (per 2026-05-18
user pull-in).

## Per-test classification

### impl_blocks::impl_method_with_extra_param

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: no method 'add' on receiver kind Int64 (line 7)")
```

Test source: `type Counter { count: int }; trait Addable { method add(n: int) -> int }; impl Addable for Counter { method add(n) { self.count + n } }; c.add(5)`. The trait method named `add` collides with the operator-overload `Add::add` dispatch — runtime tries to route `self.count + n` (Int64 + Int) to a method called `add` on Int64 receiver kind and fails. Affects compiler operator-dispatch resolution inside trait method bodies. v0.3 W1 trait-operator-coverage scope.

### stress_default::trait_default_method_used_when_not_overridden

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `string` and `unknown`.
```

Shape C. Trait default body `"Object: " + self.name()` — `name(): string` declared in trait but the call's return type isn't flowed back into the inference solver at the binop. v0.3 W1 trait-default-method scope.

### stress_default::trait_default_method_overridden

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `string` and `unknown`.
```

Shape C. Identical failure shape to `trait_default_method_used_when_not_overridden`; this fixture additionally overrides `describe()`.

### stress_default::trait_default_method_calls_required_method

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `string`.
```

Shape C. `self.first_name() + " " + self.last_name()` — both trait method return types not threaded.

### stress_default::mixed_override_and_default

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `string` and `unknown`.
```

Shape C.

### stress_default::mixed_override_and_default_category

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `string` and `unknown`.
```

Shape C.

### stress_default::partial_override_of_defaults

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `string`.
```

Shape C.

### stress_default::trait_multiple_defaults

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape C (degenerate — both operands lost).

### stress_dispatch_advanced::call_same_trait_method_on_two_different_types

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `string`.
```

Shape C.

### stress_dispatch_advanced::display_on_multiple_types

Class: **FN-REG-CORRECTNESS**

```
V2 bytecode verification warning: ... StringConcatTyped at offset 607 in function 'Cat::display' has no FrameDescriptor
assertion `left == right` failed: Expected 'Dog:Rex and Cat:Whiskers', got '{name: "Rex"} and {name: "Whiskers"}'
```

Shape B. `.to_string()` returns debug repr instead of dispatching to user `Display::display()`.

### stress_dispatch_advanced::multiple_named_display_impls_default_used

Class: **FN-REG-CORRECTNESS**

```
assertion `left == right` failed: Expected 'plain:hi', got '{text: "hi"}'
```

Shape B + named-impl resolution (W1 scope).

### stress_dispatch_advanced::nested_trait_method_in_expression

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Mul`: operand types are `unknown` and `unknown`.
```

Shape A (Mul variant).

### stress_dispatch_advanced::same_field_names_different_types_same_trait

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape A / C.

### stress_dispatch_advanced::trait_method_calling_another_trait_method

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape C.

### stress_dispatch_advanced::trait_method_on_different_instances

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape C.

### stress_dispatch_advanced::trait_method_returns_array_length

Class: **FN-REG-CORRECTNESS**

```
Runtime error: datatable.len: expected DataTable/TableView receiver, got Ptr(TypedObject) (line 5)
```

Test source: `type Bag { items: any }; trait Countable { method count() -> int }; impl Countable for Bag { method count() { self.items.length() } }; b.count()`. `items: any` plus an array literal `[1,2,3,4,5]` is mis-routing `.length()` to the `DataTable.len` builtin (probably via `any`-typed receiver dispatch). Plausibly-correct user-facing program; previously worked. Affected: stdlib method-dispatch + `any`-typed field handling. Not a SURFACE message; routes here, not SCOPE-RECLAIM.

### stress_dispatch_advanced::trait_method_using_closure_variable

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: map: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. ... UNREACHABLE until ckpt-6 STRICT close. ...
```

- Pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. The SURFACE explicitly cites the V3-S5 ckpt-2/ckpt-3 cascade work that the 2026-05-18 disposition pulled into v0.3.
- SURFACE text: as above (V3-S5 ckpt-2 consumer-cascade tier 1, `TypedArrayData` enum deletion, "UNREACHABLE until ckpt-6 STRICT close").
- (Incorrect) v0.4 anchor cited: SURFACE says "UNREACHABLE until ckpt-6 STRICT close" but does NOT cite v0.4 — yet the corresponding work is gating live v0.3 user code (`.map(|x| ...)` on array literal — most fundamental Shape iteration). Per TAXONOMY: 2026-05-18 row pulls in V3-S5 ckpt-5/ckpt-6 — `.map` cascade IS that work.
- Test asserts on user-facing semantics (`.length() == 3`); test stays the same after fix.

### stress_dispatch_advanced::trait_number_method_call_chain

Class: **FN-REG-CORRECTNESS**

```
Runtime error: no method 'div' on receiver kind Float64 (line 5)
```

Test source: `method avg() { self.sum / self.count.to_number() }` where `sum: number, count: int`. Operator `/` is being routed through method dispatch (`div`) on Float64 instead of using native fp-div. Operator-overload regression spilling into native ops. W1 trait operator coverage scope.

### stress_impl::four_traits_on_one_type

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape A / C.

### stress_impl::trait_method_calls_builtin_range

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. ... UNREACHABLE until ckpt-6 STRICT close. ...
```

- Pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6.
- SURFACE text: as above (V3-S5 ckpt-3 cascade, `range` builtin).
- (Incorrect) v0.4 anchor: SURFACE again says "UNREACHABLE until ckpt-6 STRICT close" without an explicit v0.4 cite, but the underlying construction-cascade IS in 2026-05-18 v0.3 pull-in scope.
- Test asserts user-facing `range(0, 5).length() == 5` semantics; test stays the same after fix.

### stress_operators::all_four_arithmetic_operators

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::display_trait_basic

Class: **FN-REG-CORRECTNESS**

```
V2 bytecode verification warning: StringConcatTyped at offset 576 in function 'User::display' has no FrameDescriptor
assertion `left == right` failed: Expected 'User:Alice', got '{name: "Alice"}'
```

Shape B. Display impl compiles (verifier sees it) but `.to_string()` returns debug repr instead of routing to `display()`. Display trait dispatch is W1 trait-coverage scope.

### stress_operators::display_trait_to_string_numeric

Class: **FN-REG-CORRECTNESS**. Shape B.

### stress_operators::display_trait_with_formatting

Class: **FN-REG-CORRECTNESS**. Shape B.

### stress_operators::display_trait_with_multiple_fields

Class: **FN-REG-CORRECTNESS**. Shape B.

### stress_operators::impl_add_for_custom_type

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

Shape A canonical: `impl Add for Vec2 { method add(other) { ... } }; let c = a + b`. Operator-overload `+` not resolved to user `impl Add`. W1 headline regression.

### stress_operators::impl_div_for_custom_type

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::impl_mul_for_custom_type

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::impl_neg_for_custom_type

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::impl_sub_for_custom_type

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::multiple_operator_traits_on_one_type

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::named_impl_basic

Class: **FN-REG-CORRECTNESS**

```
V2 bytecode verification warning: StringConcatTyped ... in function 'User::display' / 'Display::User::JsonDisplay::display'
assertion `left == right` failed: Expected 'default:Alice', got '{name: "Alice"}'
```

Shape B + named-impl. Both default and named Display impls compile (verifier sees both) but `.to_string()` returns debug repr.

### stress_operators::operator_and_display_on_same_type

Class: **FN-REG-CORRECTNESS**. Shape B.

### stress_operators::operator_overload_chained_operations

Class: **FN-REG-CORRECTNESS**. Shape A.

### stress_operators::operator_result_in_further_computation

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `Mul`: operand types are `unknown` and `unknown`.
```

Shape A (Mul variant).

### stress_operators::type_with_display_and_operator

Class: **FN-REG-CORRECTNESS**. Shape B.

## UNKNOWN list

(empty)

## Notes for team-lead aggregation

- **Single regression locus drives the bulk.** Shapes A + B + C share an
  underlying common cause: trait-method dispatch / trait-method
  return-type signatures are not being threaded through inference
  (operator overload, Display::to_string, and trait default bodies are
  three sub-cases of the same compiler subsystem). A single bisect on
  the W1 trait-coverage code (`crates/shape-vm/src/compiler/` trait
  resolution + `crates/shape-runtime/src/type_system/` method-call
  inference) plausibly closes 30 of 36 failures.
- **Suggested bisect anchors:** `git log --oneline -- crates/shape-vm/src/compiler/ crates/shape-runtime/src/type_system/` filtered to the v0.3 W1 window. Particularly the trait operator-overload landing commit and any post-W1 type-tracker / typed-opcode changes (the `StringConcatTyped ... has no FrameDescriptor` verifier warning suggests the compile path for Display::display() bodies is now emitting a typed-string-concat opcode without a frame descriptor — a typed-opcodes-tightening change after Display impls were wired).
- **Two SCOPE-RECLAIM rows** are not blocked on a "v0.4" mis-cite — the
  SURFACE says "UNREACHABLE until ckpt-6 STRICT close" with no v0.4
  citation. But `.map(|x| ...)` and `range(...)` ARE in 2026-05-18
  V3-S5 ckpt-5/ckpt-6 pull-in scope; the SURFACE describes work that
  is in-scope-but-incomplete, which is the SCOPE-RECLAIM signature.
