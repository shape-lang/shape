# smoke_test — all-green at HEAD 82f049dd. No classification needed.

**Total tests in binary:** 8
**Passed:** 8 / Failed: 0 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test smoke_test --no-fail-fast 2>&1`

```
running 8 tests
test parse_error_detected ... ok
test output_contains_substring ... ok
test output_capture_multiline ... ok
test output_capture_single_line ... ok
test typed_object_property_assignment ... ok
test bool_result ... ok
test lsp_and_runtime_combined ... ok
test function_hover_and_execution ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 131.90s
```

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Zero failing tests to classify.
