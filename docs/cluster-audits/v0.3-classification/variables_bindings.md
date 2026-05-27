# variables_bindings classification

**HEAD:** 82f049dd
**Total tests in binary:** 167
**Passed:** 142 / Failed: 25 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test variables_bindings --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 20 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 5 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### destructuring::array_destructuring_basic

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`.
Strict typing requires both operands to have a known concrete type at
compile time. Add a type annotation to disambiguate.")
```

- Minimal repro:
  ```shape
  let [a, b, c] = [1, 2, 3]
  a + b + c
  ```
- Array destructure-binding kinds (`a`, `b`, `c`) lost — array literal `[1, 2, 3]` element type `int` is not propagated into the destructured binding slots; subsequent `a + b + c` sees `unknown + unknown`.
- Affected subsystem: compiler array-destructure path (`crates/shape-vm/src/compiler/`) + `type_system` binding-kind propagation. Mirrors existing `objects_arrays.md` `arrays::array_destructuring_in_function_param` FN-REG-CORRECTNESS classification.
- Bisected commit: not run (audit-only).

### destructuring::array_destructuring_two_elements

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Mul`: operand types are `unknown` and `unknown`. ...")
```

- Minimal repro:
  ```shape
  let [x, y] = [10, 20]
  x * y
  ```
- Same root cause as `array_destructuring_basic`: destructure-binding kinds lost.

### destructuring::nested_array_destructuring

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ...")
```

- Minimal repro:
  ```shape
  let [a, [b, c]] = [1, [2, 3]]
  a + b + c
  ```
- Same root cause; nested array destructure adds an inner layer that also drops binding kinds.

### destructuring::destructuring_in_for_loop

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. ...")
```

- Repro:
  ```shape
  let points = [{x: 1, y: 2}, {x: 3, y: 4}]
  let mut sum = 0
  for {x, y} in points { sum = sum + x + y }
  sum
  ```
- Dated user disposition: 2026-05-21 "Object destructuring must fully work."
- SURFACE: generic strict-typing error; for-loop pattern `{x, y}` over an array of object literals doesn't propagate element kinds into destructured bindings — same family as the `objects::destructuring_in_function_param` SCOPE-RECLAIM in `objects_arrays.md`.
- Why mis-cite: not surfaced under a v0.4 anchor; failure routes per the 2026-05-21 row.
- Test asserts on user-facing semantics (sum value); test stays the same after fix.

### destructuring::array_destructuring_rest

Class: **V0.4-DEFER** → reclassified **SCOPE-RECLAIM**

Re-examined: the SURFACE text is "Semantic error: array rest-pattern (`[a, ...rest]`) is not supported" — a clean surface-and-stop. The 2026-05-21 row names "Object destructuring must fully work" but not array rest patterns; rest patterns were never in a v0.3 dated pull-in. However the destructuring family is in active scope per 2026-05-21 and the per-binary audit precedent groups rest with destructuring.

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: Some("Semantic error: array rest-pattern
(`[a, ...rest]`) is not supported\n\nRuntime error: Undefined variable:
first. ...")
```

- Dated disposition: 2026-05-21 "Object destructuring must fully work" (destructuring family).
- SURFACE: "array rest-pattern (`[a, ...rest]`) is not supported" — clean surface; downstream "Undefined variable: first" is a cascade.
- Why mis-cite: surface cites no v0.4 anchor; rest patterns sit inside the destructuring family pulled in 2026-05-21.
- Test asserts on user-facing semantics (`first == 1`); test stays the same after fix.

### stress_let_basic::test_width_i8

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I8": Number(100)}
```

- Repro:
  ```shape
  fn test() -> int { let a: i8 = 100; return a }
  test()
  ```
- The runtime returns a tagged wrapper `Object {"I8": Number(100)}` instead of projecting to plain `int`/`number` per the declared `-> int` return. Width-typed locals are not lowering back to scalar on function-return.
- Affected subsystem: width-type (`i8`/`i16`/`i32`/`u8`/`u16`/`u32`/`u64`) return projection in `crates/shape-runtime/src/type_schema/` + VM return-marshal in `crates/shape-vm/src/executor/`.

### stress_let_basic::test_width_i16

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I16": Number(1000)}
```

- Same root cause as `test_width_i8`; `i16` variant.

### stress_let_basic::test_width_i32

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I32": Number(100000)}
```

- Same root cause; `i32` variant.

### stress_let_basic::test_width_u8

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U8": Number(200)}
```

- Same root cause; `u8` variant.

### stress_let_basic::test_width_u16

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U16": Number(50000)}
```

- Same root cause; `u16` variant.

### stress_let_basic::test_width_u32

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U32": Number(3000000)}
```

- Same root cause; `u32` variant.

### stress_let_basic::test_width_u64

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U64": Number(999999)}
```

- Same root cause; `u64` variant.

### stress_let_basic::test_width_i8_negative

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I8": Number(-128)}
```

- Same root cause; `i8` negative-boundary variant.

### stress_let_basic::test_width_i16_negative

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I16": Number(-32768)}
```

- Same root cause; `i16` negative-boundary variant.

### stress_let_basic::test_width_i8_max_boundary

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I8": Number(127)}
```

- Same root cause; `i8` max-boundary variant.

### stress_let_basic::test_width_u8_max_boundary

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U8": Number(255)}
```

- Same root cause; `u8` max-boundary variant.

### stress_let_basic::test_width_u16_max_boundary

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U16": Number(65535)}
```

- Same root cause; `u16` max-boundary variant.

### stress_let_basic::test_width_u8_zero

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U8": Number(0)}
```

- Same root cause; `u8` zero variant.

### stress_let_basic::test_width_i32_large_negative

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I32": Number(-2000000000)}
```

- Same root cause; `i32` large-negative variant.

### stress_let_basic::test_width_typed_arithmetic

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I8": Number(30)}
```

- Repro:
  ```shape
  fn test() -> int { let a: i8 = 10; let b: i8 = 20; return a + b }
  test()
  ```
- Same root cause: i8 + i8 result stays wrapped; `-> int` return projection drops the wrapper. Compounded by typed-arithmetic on width types.

### stress_let_basic::test_width_typed_u8_arithmetic

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"U8": Number(150)}
```

- Same root cause; `u8` arithmetic variant.

### stress_let_basic::test_mixed_width_add

Class: **FN-REG-CORRECTNESS**

```
Expected number, got: Object {"I16": Number(210)}
```

- Repro:
  ```shape
  fn test() -> int { let a: i8 = 10; let b: i16 = 200; return a + b }
  test()
  ```
- Same root cause + mixed-width widening (i8 + i16 → i16 wrapper preserved on return).

### stress_let_basic::test_width_i8_overflow_compile_error

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Bool": Bool(false)})
```

- Repro:
  ```shape
  fn test() { let x: i8 = 128; return x }
  ```
- Compile-time width-overflow check is gone: `128` overflows `i8` (range -128..=127) but compiler now accepts and emits a `Bool(false)` somehow. Real correctness regression — overflow gate was a documented safety property of width-typed bindings.
- Affected subsystem: compile-time integer-literal width-range check in `crates/shape-runtime/src/type_system/`.

### stress_let_basic::test_width_u8_negative_compile_error

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Bool": Bool(false)})
```

- Same root cause: `let x: u8 = -1` should be a compile error; now accepted.

### stress_let_basic::test_width_u16_overflow_compile_error

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Bool": Bool(false)})
```

- Same root cause: `let x: u16 = 65536` should be a compile error; now accepted.

## UNKNOWN list

(none)
