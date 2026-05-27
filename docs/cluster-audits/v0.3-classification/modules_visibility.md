# modules_visibility classification

**HEAD:** 82f049dd
**Total tests in binary:** 136
**Passed:** 128 / Failed: 8 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test modules_visibility --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 7 |
| FN-REG-DIAGNOSTIC  | 1 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### scoped_contract::scoped_contract_hashmap_requires_explicit_import

Class: **FN-REG-CORRECTNESS**

```
panicked at crates/shape-runtime/src/wire_conversion.rs:201:5:
assertion `left == right` failed: slot kind HashMap does not match HeapValue::Char
  left: Char
 right: HashMap
```

- Minimal repro: an `import` of HashMap module followed by a HashMap operation surfaces a slot-kind/HeapValue mismatch at `wire_conversion.rs:201` (assertion failure → panic). Slot says `Char`, but HeapValue is `HashMap`.
- Bisect: not run.
- Affected subsystem: wire_conversion slot↔HeapValue tag pairing (`crates/shape-runtime/src/wire_conversion.rs:201`). Direct assertion-failure panic — not a clean SURFACE; release-blocker on a hot panic.

### inline_modules::test_mod_nested_access_runtime - should panic

Class: **FN-REG-CORRECTNESS**

```
note: test did not panic as expected
```

- Test was annotated `#[should_panic]` to assert that nested module member access aborts with a specific error. The program now runs to completion. Either the language behavior changed (nested access now silently succeeds with wrong value) or the panic-shape changed. Either is a behavior regression on plausibly-correct user code.

### complex::test_complex_module_then_enum_usage

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Equal`: operand types are `Concrete(Reference(TypePath { segments: [\"Color\"], qualified: \"Color\" }))` and `Concrete(Reference(TypePath { segments: [\"Color\"], qualified: \"Color\" }))`.
```

- Minimal repro: comparing two enum values of the *same* concrete enum type with `==` is rejected; inference reports both operands as the same enum but strict-typing still refuses. The enum-type equality plumbing isn't recognizing Reference(TypePath) on both sides as a known concrete type. Plausibly-correct user code; clear regression.

### inline_modules::test_mod_multiple_functions_runtime

Class: **FN-REG-CORRECTNESS**

```
Cannot infer types for binary operation `Add`: operand types are `unknown` and `unknown`.
```

- Module-scoped function call return type collapses to `unknown × unknown` when called and the result is `+`-combined. Strict-typing regression — module function return type isn't propagating across the module boundary.

### inline_modules::test_mod_triple_nested_access_runtime - should panic

Class: **FN-REG-CORRECTNESS**

```
note: test did not panic as expected
```

- Same family as `test_mod_nested_access_runtime`. Triple-nested module access should error but now succeeds/no-ops.

### visibility::test_vis_module_inner_fn_not_accessible_on_outer

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Invalid function call', got: Runtime error: call_value_immediate_nb: callee must be NativeKind::Ptr(HeapKind::Closure), NativeKind::Ptr(HeapKind::ModuleFn), or NativeKind::UInt64, got Null (line 7)
```

- Old expected: `Invalid function call`.
- New actual: `call_value_immediate_nb: callee must be ... got Null`.
- Language change: the error is correct (private function call resolves to `Null` at the call site, producing a runtime-error from `op_call_value`), but the diagnostic text changed from a higher-level "Invalid function call" to a low-level call-convention error. Behavior is correct; fixture text is stale.

### scoped_contract::scoped_contract_namespace_function_calls_use_double_colon

Class: **FN-REG-CORRECTNESS**

```
Semantic error: module namespace 's' is not typed. Missing module schema for export 'from_array'
Semantic error: module namespace 's' is not typed. Missing module schema for export 'size'
```

- Minimal repro: a `set::from_array(...)` (set module aliased as `s`) call across the namespace operator. Module schema lookup is missing for stdlib `set` exports. Plausibly-correct user code (named namespace import + `::` access); strict-typing regression.

### scoped_contract::scoped_contract_regular_named_import_alias_executes

Class: **FN-REG-CORRECTNESS**

```
Runtime error: Undefined function: set_size. Function names resolve from module scope, explicit imports, type-associated scope, and the implicit prelude.
```

- Minimal repro: aliased named-import of a function (`use set::size as set_size;`), then calling `set_size(...)` returns "Undefined function". Import-alias plumbing not registering the aliased name in the function-resolution scope. Plausibly-correct code; regression.
