# window_functions classification

**HEAD:** 82f049dd
**Total tests in binary:** 8
**Passed:** 1 / Failed: 7 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test window_functions --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 7 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All failing tests are TDD-style fixtures asserting on stale "Unknown method
'X'" diagnostic text. The runtime now emits a different (also-correct)
diagnostic shape: `no method 'X' on receiver kind Ptr(TypedArray)`. Language
behavior is correct — the array methods (`row_number`, `rank`, `lag`,
`lead`, `ntile`, `rolling`, `scan`) are legitimately absent and the VM
cleanly surfaces a method-missing error. Only the expected-text strings
in the fixtures are stale.

Fixture path: `tools/shape-test/tests/window_functions/basic.rs`.

## Per-test classification

### window_row_number_basic

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'row_number'',
got: Runtime error: no method 'row_number' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'row_number'`
- New actual text: `no method 'row_number' on receiver kind Ptr(TypedArray)`
- Driver: post-strict-typing method-dispatch error path now reports the
  receiver's `NativeKind::Ptr(HeapKind)` instead of the legacy
  `Unknown method '<name>'` wording.

### window_rank_basic

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'rank'',
got: Runtime error: no method 'rank' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'rank'`
- New actual text: `no method 'rank' on receiver kind Ptr(TypedArray)`
- Driver: same as above — diagnostic shape change in method-dispatch
  error path.

### window_lag_offset_1

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'lag'',
got: Runtime error: no method 'lag' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'lag'`
- New actual text: `no method 'lag' on receiver kind Ptr(TypedArray)`
- Driver: same diagnostic shape change.

### window_lead_offset_1

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'lead'',
got: Runtime error: no method 'lead' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'lead'`
- New actual text: `no method 'lead' on receiver kind Ptr(TypedArray)`
- Driver: same diagnostic shape change.

### window_ntile_quartiles

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'ntile'',
got: Runtime error: no method 'ntile' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'ntile'`
- New actual text: `no method 'ntile' on receiver kind Ptr(TypedArray)`
- Driver: same diagnostic shape change.

### window_rolling_sum

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'rolling'',
got: Runtime error: no method 'rolling' on receiver kind Ptr(TypedArray) (line 3)
```

- Old expected text: `Unknown method 'rolling'`
- New actual text: `no method 'rolling' on receiver kind Ptr(TypedArray)`
- Driver: same diagnostic shape change.

### window_cumulative_sum

Class: **FN-REG-DIAGNOSTIC**

```
Error should contain 'Unknown method 'scan'',
got: Semantic error: Cannot infer types for binary operation `Add`:
operand types are `unknown` and `unknown`. Strict typing requires both
operands to have a known concrete type at compile time. Add a type
annotation to disambiguate.
```

- Old expected text: `Unknown method 'scan'`
- New actual text: `Cannot infer types for binary operation 'Add'` (compile
  error from inside the closure body `|acc, x| acc + x`).
- Driver: bidirectional closure inference relies on the callee method
  signature to seed the closure param types. With `scan` absent from
  Array's PHF, the compiler errors on the `acc + x` operand inference
  before reaching the method-missing error path. The test still belongs
  to FN-REG-DIAGNOSTIC: language behavior (rejecting `scan` + unannotated
  closure under strict typing) is correct; only the test's expected-text
  string is stale and would need to change to match the strict-typing
  diagnostic.

## Per-binary notes

- `window_over_partition_by` PASSES — D-γ close (KC #6(e), 2026-05-22)
  routed the generic-no-body callee through `CallMethod` dispatch,
  surface-and-stopping at `handle_map_v2`'s V3-S5 ckpt-2 NotImplemented
  stub. The fixture's `expect_run_err_contains("map: SURFACE")` matches.
- No SCOPE-RECLAIM entries: none of the failures cite v0.4 anchors. The
  underlying window-function methods (`row_number`, `rank`, `lag`,
  `lead`, `ntile`, `rolling`, `scan`) are not in any of the dated v0.3
  user pull-ins; the language correctly surfaces "no method" errors for
  them. The fixtures intentionally TDD-assert against the missing-method
  diagnostic, so once the diagnostic-text update lands, these tests stay
  TDD pinning for future v0.4 implementation.
- No UNKNOWNs.
