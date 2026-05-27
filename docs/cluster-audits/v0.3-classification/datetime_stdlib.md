# datetime_stdlib classification

**HEAD:** 82f049dd
**Total tests in binary:** 0
**Passed:** 0 / Failed: 0 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test datetime_stdlib --no-fail-fast 2>&1`

## Run result

```
     Running tests/datetime_stdlib/main.rs (target/debug/deps/datetime_stdlib-4c52077c95cc6502)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
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

## Notes

All three submodules (`datetime`, `time_module`, `io_module`) are gated out
via `#[cfg(any())]` in `tools/shape-test/tests/datetime_stdlib/main.rs`.
Per the file's own header comment (ADR-006 §2.7.4 / W11-tail, 2026-05-10),
the submodules were written against the deleted `ValueWord` / `ValueWordExt`
/ `vmarray_from_vec` API plus the pre-§2.7.10 `ModuleExports::invoke_export`
shim and deleted `file_ops::io_*` / `path_ops::io_*` exports. Gated pending
wholesale rewrite onto §2.7.10/Q11 `KindedSlot` dispatch + current
`time`/`io` module surfaces.

Binary compiles with 0 tests; nothing to classify.
