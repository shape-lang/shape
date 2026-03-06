# Benchmark Tracking

This directory stores week-by-week JIT vs Node benchmark progress for the 10-week execution plan.

## Files
- `jit_weekly_history.tsv`: one row per weekly run (geomean, wins/losses, snapshot path).
- `jit_weekly_run_history.tsv`: one row per benchmark-suite repetition used to compute weekly medians.
- `jit_weekly_benchmark_history.tsv`: one row per benchmark per weekly run.
- `snapshots/*.tsv`: full per-run benchmark metrics emitted by `run_all.sh`.

## Record a weekly snapshot
```bash
shape/benchmarks/track_week.sh --week W1 --runs 5 --pin-core 3 --notes "baseline after module split"
```

## Re-run with custom flags
```bash
shape/benchmarks/track_week.sh --week W2 --runs 5 --notes "loop lowering v1" -- --fast --enforce --budget-file shape/benchmarks/v8_goal_budget.tsv
```

## Data source
`track_week.sh` calls:
```bash
shape/benchmarks/run_all.sh --write-ratios <tmp> --write-metrics <tmp>
```
multiple times (`--runs`, default 5), computes per-field medians, writes an aggregated weekly snapshot, and appends structured TSV history files.
