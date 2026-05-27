# typesystem classification

**HEAD:** 82f049dd
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test typesystem --no-fail-fast 2>&1`

## Status: NOT A TEST BINARY

`cargo test -p shape-test --test typesystem` fails with:

```
error: no test target named `typesystem` in `shape-test` package
```

No `typesystem` entry exists in `shape-test`'s test-target list (verified
against the `available test targets:` enumeration printed by cargo — 60
targets, none named `typesystem`).

The directory `tools/shape-test/tests/typesystem/` exists on disk and
contains:

- `mod.rs` — `mod hoisting;`
- `hoisting.rs` — 1 test: `lsp_and_runtime_combined`

It is **orphaned**: no top-level integration `*.rs` file in
`tools/shape-test/tests/` declares `mod typesystem;`, so cargo does not
compile or run any of its tests. `grep -rn "mod typesystem" tools/shape-test/tests/`
returns zero hits. `grep -rn "typesystem" tools/shape-test/` returns zero hits.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

Zero failing tests to classify — binary does not exist, and the
on-disk `typesystem/` subdirectory contributes zero tests to any
running binary.

## Note for team-lead

The orphan `typesystem/` subdirectory is a doc/infra-hygiene finding,
not a test failure. Either:

- Wire `mod typesystem;` into an existing integration entry-point so
  `lsp_and_runtime_combined` runs, or
- Delete `tools/shape-test/tests/typesystem/` if the test was
  superseded.

Out of scope for this audit (audit-only, no source changes).
