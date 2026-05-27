# e2e_gated — all-green at HEAD 82f049dd. No classification needed.

**HEAD:** 82f049dd
**Total tests in binary:** 0
**Passed:** 0 / Failed: 0 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test e2e_gated --no-fail-fast 2>&1`

Binary `tools/shape-test/tests/e2e_gated/main.rs` gates all submodules behind
Cargo features `e2e-python` / `e2e-typescript`. With neither feature enabled
(the default `--no-fail-fast` invocation above), the binary compiles but
contains zero test functions:

```
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

No failing tests → no per-test classification rows. Per-class counts: all
zero (FN-REG-CORRECTNESS 0, FN-REG-DIAGNOSTIC 0, SCOPE-RECLAIM 0,
V0.4-DEFER 0, INFRA-FLAKY 0, UNKNOWN 0).
