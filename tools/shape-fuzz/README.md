# shape-fuzz — Shape differential-fuzz harness (W13)

Subprocess-level differential execution of `.shape` programs comparing
`shape run --mode vm` against `shape run --mode jit`. Implements the
W13 audit recommendation (`docs/cluster-audits/v0.3-w13-differential-fuzz-audit.md`,
§1.3 custom harness; cargo-fuzz rejected).

## Public API (`shape_fuzz` library)

- `compare_outputs(snippet, cfg) -> Result<CompareResult, HarnessError>` —
  spawn `shape run --mode {vm,jit} <snippet>` and capture
  `(stdout_tail, exit_code)` per mode.
- `classify_divergence(vm, jit) -> Divergence` — pure §2 8-class table.
- `record_finding(cmp, divergence, dir) -> Result<PathBuf, HarnessError>` —
  write a self-contained record (source + both outcomes) for triage.
- `minimize_reproducer(...)` — placeholder; AST-subset-bisect lands in W13.3.

## CLI

```
shape-fuzz run --corpus=<dir> [--shape-bin=<path>] [--timeout-secs=30] \
                [--findings-dir=<dir>] [--seed=<u64>] [--allow-low-signal]
```

Exit codes:

- `0` — every seed classified `Convergent` (or NOISE / allowed LOW).
- `1` — at least one HIGH or MEDIUM signal divergence fired.
- `2` — harness driver failure (binary missing, snippet unreadable, ...).

## Adding corpus seeds (W13.3 territory)

W13.3 lands the per-domain corpus at `tools/shape-fuzz/corpus/<domain>/*.shape`
following the 50-seed inventory in the W13 audit §3 (arithmetic 10 +
collections 10 + closures 7 + patterns 8 + async 5 + generics 8 +
fallthrough 2). Until then, the only seed in this crate is the smoke
self-test at `tests/smoke-self-test/s1.shape` (the same program as
`/tmp/smokes/s1.shape`), exercised by `cargo test -p shape-fuzz`.

When W13.3 lands corpus seeds, each `(g)`-class entry runs end-to-end and
must classify `Convergent`; each `(n)`-class entry is named after its
`docs/v0.3-close-summary.md` §5.1 residual class and is the harness's own
regression sentinel against unintended convergence flips.

## CI cadence (W13.4 territory)

The harness runs **NIGHTLY ONLY** per the Phase 4 test execution policy
in `docs/cluster-audits/phase-3-team-lead-handover.md:42-47`. It is not a
per-commit gate, not a merge-ceremony gate (except at audit-day batch
merges per the coverage-gate convention), and not the v0.3.0 release
gate. W13.4 lands `.github/workflows/nightly-fuzz.yml` per the audit
§6.2 shape (cron `0 4 * * *`, 180-minute job cap, per-domain 20-minute
sub-timeout, findings uploaded as GitHub Actions artifacts with 30-day
retention).

## Discipline

- Custom harness only. No `cargo-fuzz` / libFuzzer / AFL dependency, no
  nightly-toolchain bump (per audit §1.3 reasoning).
- Bounded mutation only. W13.3 lands a fixed mutation set per audit §4.2;
  coverage-guided random-byte mutation is out of scope (random bytes
  almost never form a syntactically-valid Shape program).
- `[jit-fallback]` stderr emission is **NOT** a divergence per audit §2.1
  — stderr is piped to `/dev/null` and the stdout match drops the case
  into `Convergent`.
- All CLAUDE.md Forbidden Patterns + Renames-to-refuse-on-sight apply to
  any code in this crate.
