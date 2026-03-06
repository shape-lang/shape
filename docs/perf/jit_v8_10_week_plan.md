# JIT vs V8 Honest 10-Week Execution Plan

## Objective
Beat Node/V8 with general compiler/runtime architecture improvements only.

Hard constraints:
- No benchmark-name dispatch, no benchmark-only kernels.
- Keep maintainability guardrails active (module splits, file line budgets).
- Track every week with repeated benchmark medians and raw run audit logs.

## Baseline (2026-02-22, tracked W1 snapshot, runs=5, pinned core)
- Geomean JIT/Node: `1.88x` (range `1.88-1.91`)
- Warm geomean JIT(exec)/Node: `1.58x` (range `1.57-1.61`)
- Wins/losses: `2/8`
- Largest gaps:
  - `02_fib_iter`: ~`4.3-4.4x`
  - `03_sieve`: ~`3.6-3.9x`
  - `09_matrix_mul`: ~`3.6-5.9x`
  - `04_mandelbrot`: ~`2.7x`
  - `05_spectral`: ~`2.35x`

## KPI Definitions
- `Cold geomean`: `shape/benchmarks/run_all.sh --fast` JIT/Node geometric mean.
- `Warm geomean`: JIT `exec_ms` / Node geometric mean from phase metrics.
- `No-cheat guard`: `cargo xtask benchmark-specialization check`.
- `Maintainability guard`: `cargo xtask line-budget check`.

## Weekly Plan

| Week | Focus | Required Code Artifacts | KPI Gate (cold geomean) |
|---|---|---|---|
| W1 | Measurement hardening + anti-cheating | repeated median tracking, run-level history, no-cheat guard wired into smoke | Stable baseline with run-range reported |
| W2 | Tight loop lowering v1 | loop-carried scalars stay unboxed in SSA/registers | `<= 1.75x` |
| W3 | Tight loop lowering v2 | stronger IV canonicalization, safe unroll policy for unboxed loops | `<= 1.55x` |
| W4 | Typed contiguous numeric arrays | unified typed numeric array fast layout and access path | `<= 1.35x` |
| W5 | Bounds + memory pipeline | bounds-check hoist and noalias-aligned memory lowering | `<= 1.20x` |
| W6 | SIMD infrastructure | vectorization plan lowered to emitted vector loop + scalar tail | `<= 1.10x` |
| W7 | SIMD numeric kernels | reduction/strip-mining improvements for nested numeric loops | `<= 1.02x` |
| W8 | Call-path redesign | direct-call ABI + typed inlining heuristics | `<= 0.96x` |
| W9 | Table<T>/Queryable<T> | typed query pipeline fusion and columnar fast operators | `<= 0.92x` |
| W10 | Correctness + stabilization | differential tests, translation validation, perf regression gates | `<= 0.90x` stretch |

## Tracking Workflow
1. Record a week using repeated runs:
   - `shape/benchmarks/track_week.sh --week WN --runs 5 --pin-core <cpu> --notes "..."`
2. Inspect outputs:
   - `shape/benchmarks/tracking/jit_weekly_history.tsv`
   - `shape/benchmarks/tracking/jit_weekly_run_history.tsv`
   - `shape/benchmarks/tracking/jit_weekly_benchmark_history.tsv`
3. Run guardrails before week closeout:
   - `cargo xtask benchmark-specialization check`
   - `cargo xtask line-budget check`
   - `cargo test -p shape-jit --lib`
   - `shape/benchmarks/run_all.sh --fast`

## Definition Of Done (Week Complete)
A week closes only when all are true:
- Code artifacts for that week are landed.
- Weekly KPI gate is met (or explicitly marked as missed with root-cause notes).
- No-cheat + maintainability guards are green.
- A weekly snapshot is recorded with run-range metadata.
