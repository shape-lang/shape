# features classification

**HEAD:** 82f049dd
**Total tests in binary:** 7
**Passed:** 5 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test features --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### snapshots::snapshot_preserves_variables

Class: **FN-REG-CORRECTNESS**

```
Error should contain 'queryable', got: Runtime error: Suspended on future 18446744073709551615
```

- Minimal repro: snapshot()-bearing program that previously emitted a "queryable" diagnostic now emits `Runtime error: Suspended on future 18446744073709551615` instead — async-suspension is leaking past the snapshot test driver. (Snapshot tests live in `tools/shape-test/tests/features/snapshots.rs`.)
- Bisect: not run (audit-only).
- Affected subsystem: VM snapshot resume + async scheduler — suspended-future sentinel `u64::MAX` is bubbling up as a runtime error instead of being captured by the snapshot machinery. Plausibly a regression in async/snapshot interaction.

### snapshots::snapshot_returns_hash_on_first_run

Class: **FN-REG-CORRECTNESS**

```
Error should contain 'queryable', got: Runtime error: Suspended on future 18446744073709551615
```

- Same root cause as `snapshot_preserves_variables` above — twin test, same diagnostic shape, same subsystem.
- Affected subsystem: VM snapshot machinery + async suspension sentinel.
