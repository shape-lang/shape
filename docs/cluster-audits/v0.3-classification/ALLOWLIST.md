# Pre-tag-gate allowlist (proposal)

**HEAD:** `82f049dd`.
**Purpose:** define the SET of test failures that MAY pass through the
`cargo test -p shape-test --no-fail-fast` pre-tag gate without blocking
the release. Built per supervisor 2026-05-26 Step 3(a) ratify:

> Allowlist with **ZERO FN-REG-CORRECTNESS** and **ZERO SCOPE-RECLAIM**
> entries by design (both classes RELEASE-BLOCKING).

## Allowlisted classes (release-safe; refined post error_handling + regression re-class)

| Class | Count | Disposition |
|---|---:|---|
| V0.4-DEFER | 40 | Legitimate v0.4 (§5.16 named scope). Each entry has an issue link or in-doc justification. |
| FN-REG-DIAGNOSTIC | 57 | Per-test fixture text updates; the language behavior is correct. |
| INFRA-FLAKY | 1 | Test-isolation defect; investigate but not release-blocking. |
| **TOTAL ALLOWLISTED** | **98** | |

## Per-binary allowlist

```
binary                  V4   D   F   Allowlist total
─────────────────────────────────────────────────────
arrays_vectors           0   3   0   3
comptime                 0   1   0   1
complex_integration      0   0   1   1
control_flow             0   5   0   5
enums                   27   0   0  27
literals                 0   1   0   1
lsp                      0   4   0   4
modules_visibility       0   1   0   1
objects_arrays           1   0   0   1
query_language           0   2   0   2
regression               0  36   0  36
stdlib_http              0   5   0   5
window_functions         0   7   0   7
─────────────────────────────────────────────────────
TOTALS                  28  65   1  94
```

## V0.4-DEFER per-test issue links (28)

| Test | Issue / justification |
|---|---|
| enums::* (27 match-arm payload tests, e.g. `match_some_then_arithmetic_on_x`, `match_ok_then_arithmetic_on_v`, etc.) | §5.16 B2 EnumPayload preflight (supervisor 2026-05-25 named scope). Match-arm payload binding has type `unknown` in arithmetic context. Surface-and-stops with structured semantic error. Issue: TBD-v0.4-b2-enum-payload-preflight. |
| objects_arrays::array_concatenation_with_plus | SURFACE explicitly cites `v0.4 / planned` for `IntrinsicVecAddI64`. Vec append via `+` operator. Issue: TBD-v0.4-vec-plus-op. |

## FN-REG-DIAGNOSTIC per-binary

These need per-test fixture text updates (NOT bulk-update; per-test
discipline per the binding). Update fixture in lockstep with the
language change that drove the new diagnostic text.

### regression (36)
JIT integration tests assert `WireValue::Number` but `10 + 5` now
returns `WireValue::Integer` (int/number split is a v0.3-promised
language change). Fix path: each fixture asserts the correct WireValue
variant for its expression.

### window_functions (7)
Method-not-found diagnostic shape changed: was `"Unknown method 'X'"`,
now `"no method 'X' on receiver kind Ptr(TypedArray)"`. Fix: update
expected-text in 7 tests.

### stdlib_http (5)
http API arity + module-export shape changed in R8 W6 G.3 commit
`94dc8fa9`. 8 fns now require options arg; post/put split into
typed variants. Fix: update test fixtures to new API shape OR delete
the stale tests (per LSP-E precedent).

### control_flow (5)
Various stale diagnostic text. Per-test investigation per binding.

### lsp (4)
LSP diagnostic-message text drift. Per-test fixture updates.

### arrays_vectors (3)
`contains` no-method diagnostic text; `indexOf` "2.0" vs "2" formatting
drift.

### query_language (2)
Stale text per per-binary doc.

### comptime / literals / modules_visibility (1 each)
Various per per-binary docs.

## INFRA-FLAKY (1)

`complex_integration` — full `cargo test --test complex_integration`
SIGSEGVs under parallel-worker contention. Individual tests pass when
run with `--test-threads=1`. Investigate test-isolation or memory-
pressure; not release-blocking if rerun-with-isolation passes.

## Gate enforcement

The pre-tag gate (added to `~/.claude/skills/shape-release/SKILL.md`)
should:

```bash
direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --no-fail-fast 2>&1 \
  > /tmp/shape-test-run.log

actual_fails=$(grep -E "^test result.*FAILED" /tmp/shape-test-run.log \
  | awk 'match($0, /([0-9]+) failed/, a) { sum += a[1] } END { print sum }')

allowed=94  # from this allowlist; update when allowlist changes

if [ "$actual_fails" -gt "$allowed" ]; then
  echo "ERROR: shape-test failures ($actual_fails) exceed allowlist ($allowed)"
  echo "ERROR: NEW regressions present. Classify per TAXONOMY.md before tag."
  exit 1
fi
```

(A tighter check would compare per-binary fail counts against this
allowlist's per-binary table; the simple sum-check is a valid first cut.)

## How to grow the allowlist

A NEW failure may be added to the allowlist ONLY if:

1. Its taxonomy class is V0.4-DEFER, FN-REG-DIAGNOSTIC, or INFRA-FLAKY.
2. For V0.4-DEFER: dated user authorization re-dispositions the work to
   v0.4 (added to TAXONOMY.md pull-in table by team-lead) AND the
   SURFACE is clean per ADR-006 §2.7.14.
3. For FN-REG-DIAGNOSTIC: the per-test fixture update is queued; the
   allowlist entry is a temporary parking spot, not a permanent waiver.
4. For INFRA-FLAKY: investigation issue is filed.

A failure CANNOT be added to the allowlist if its class is
FN-REG-CORRECTNESS or SCOPE-RECLAIM.

## How to remove from allowlist

Remove when:
- V0.4-DEFER → the v0.4 work lands.
- FN-REG-DIAGNOSTIC → the fixture is updated.
- INFRA-FLAKY → the isolation defect is fixed.

The pre-tag gate's `allowed=N` constant decreases accordingly.

## Current state

This allowlist defines the **MAXIMUM** acceptable shape-test fail count
for the next release. Today's 1065 raw fails (367 + 756 + 65 + 28 + 1 +
... = 1218 with re-classification of error_handling pending) must be
reduced to **≤94** before the next tag.

Gap: **~1124 fails** to either fix (FN-REG-CORRECTNESS + SCOPE-RECLAIM)
or update-fixture-in-lockstep (FN-REG-DIAGNOSTIC re-classification if
any move into that bucket).

## Related artifacts

- `TAXONOMY.md` — full classification rules + dated pull-ins.
- `TRUTH-SET.md` — corpus-wide aggregate with per-binary table.
- `SCOPE-RECLAIM.md` — mis-cite enumeration sorted by pull-in date.
- Per-binary docs in this dir.
- `~/.claude/skills/shape-release/SKILL.md` — release skill (revised per
  Step 3 to add this gate).
