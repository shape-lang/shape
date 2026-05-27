# operators classification

**HEAD:** 82f049dd
**Total tests in binary:** 613
**Passed:** 597 / Failed: 16 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test operators --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 14 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 2 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### special::error_context_operator

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Less`: operand types are `unknown` and `int`.
```

- Closure body using `!!` (error-context) operator with comparison loses operand type to `unknown`. Strict-typing inference regression on closure body.

### special::pipe_operator_chained

Class: **FN-REG-CORRECTNESS**

```
Expected 12, got 2
```

- Minimal repro: `x |> f |> g |> h` chain. Only the first stage of the pipe is executed (or only the seed value reaches the result). Plausibly-correct user code; pipe operator regression on chains.

### special::pipe_operator_with_lambda

Class: **FN-REG-CORRECTNESS**

```
Expected 20, got 0.00000000000000000...001
```

- Pipe with a closure argument returns a denormal float instead of the expected int. Memory-garbage-class symptom (denormal at bottom of f64 range = uninitialized slot interpreted as f64). Plausibly-correct code; high-severity bug.

### special::pipe_operator_basic

Class: **FN-REG-CORRECTNESS**

```
Expected 10, got 0.0000...0005
```

- Same denormal symptom as pipe_operator_with_lambda. Pipe operator fundamentally broken on basic form. Release-blocker.

### stress_bitwise_and_or::test_and_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(0)})
```

- Bitwise `&` on a float value should statically reject (or runtime-error). Now silently runs and returns 0 (memory-garbage / reinterpret-cast). Type-safety regression — bitwise ops on non-int types should be rejected.

### stress_bitwise_and_or::test_and_on_string_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(0)})
```

- Same family: `&` on a string should error; now returns 0. Type-safety regression.

### stress_bitwise_and_or::test_or_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(4609434218613702659)})
```

- `|` on float now reinterprets float bits as int. The returned int (4609434218613702659) is exactly an f64 bit-pattern. Type-safety regression — silent-wrong-result class.

### stress_bitwise_and_or::test_or_on_string_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(125866037371043)})
```

- Same type-safety regression on string. Integer payload is reinterpret of string pointer bits.

### stress_bitwise_shift::test_shl_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(-9007199254740992)})
```

- Shift-left on float now reinterprets bits; same regression family.

### stress_bitwise_shift::test_shr_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(1152358554653425664)})
```

- Same family.

### stress_bitwise_xor_not::test_not_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(-4609434218613702657)})
```

- Same family. Unary bitwise-not on float reinterprets bits.

### stress_bitwise_xor_not::test_xor_on_float_fails

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Integer": Number(4609434218613702659)})
```

- Same family — silent-wrong-result on bitwise ops applied to floats. All 8 of these (test_and/or/shl/shr/not/xor _on_float / _on_string _fails) are the same regression: bitwise opcodes have lost their type guard and now reinterpret memory.

### stress_logical::demorgan_not_and_equiv

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Equal`: operand types are `unknown` and `unknown`.
```

- DeMorgan equivalence test: `!(a && b) == (!a || !b)`. Both sides land as `unknown`. Strict-typing inference regression on bool-typed comparisons; `==` between two `bool` expressions losing both types.

### stress_logical::demorgan_not_or_equiv

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Equal`: operand types are `unknown` and `unknown`.
```

- Same family as `demorgan_not_and_equiv`. Same root cause + disposition.

### stress_ordering::comparison_in_for_loop_with_break

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `GreaterEq`: operand types are `unknown` and `int`.
```

- Loop variable from `for` loop loses concrete type in a comparison. Same shape as control_flow / query_language regressions — bidirectional inference failure on loop bindings. Plausibly-correct code.

### stress_ordering::comparison_stability_loop

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) ... UNREACHABLE until ckpt-6 STRICT close.
```

- Dated user pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array.
- SURFACE cites "range: SURFACE — V3-S5 ckpt-3 consumer-cascade ... UNREACHABLE until ckpt-6 STRICT close" — directly in-scope per the dated pull-in.
- Test asserts on user-facing semantics (range stability in a loop); stays the same after fix.
