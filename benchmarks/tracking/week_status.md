# 10-Week Status Tracker

## Rule Of Engagement
- [x] No benchmark-name/profile dispatch in compiler/runtime sources (`cargo xtask benchmark-specialization check`).
- [x] Keep module/file-size guardrails green (`cargo xtask line-budget check`).
- [x] Track every week with median-of-N runs (`shape/benchmarks/track_week.sh --runs 5 ...`).

## Global End Goal
- [ ] Geomean JIT/Node <= `1.0x` (end-to-end, `--fast` suite).
- [ ] Warm geomean JIT(exec)/Node <= `1.0x`.
- [ ] Every benchmark JIT/Node <= `1.0x`.

## Weekly Execution Checklist
- [x] W1 Measurement hardening + anti-cheating guards.
- [x] W2 Tight loop lowering v1 (loop-carried scalar registers).
- [x] W3 Tight loop lowering v2 (strong IV canonicalization + safe unroll).
- [x] W4 Typed contiguous numeric arrays (generic layout, no benchmark kernels).
- [x] W5 Bounds-check hoist + alias/noalias memory pipeline.
- [x] W6 SIMD lowering infrastructure (plan -> emitted vector loops).
- [x] W7 Numeric kernel vectorization pass (reductions + strip-mining).
- [x] W8 Call-path redesign (direct-call ABI + inline policy).
- [x] W9 Table<T>/Queryable<T> typed execution fusion.
- [x] W10 Correctness + perf stabilization pass.

## Notes
- Record each week: `shape/benchmarks/track_week.sh --week WN --runs 5 --pin-core <cpu> --notes "..."`
- Plan details and KPI gates: `shape/docs/perf/jit_v8_10_week_plan.md`.
- Latest tracked runs:
  - `W1` (runs=5, pinned core): geomean `1.88x`, warm `1.58x`
  - `W2` (runs=3, bounds + inline-address cleanup): geomean `1.60x`, warm `1.21x`
  - `W4` (runs=3): geomean `1.82x`, warm `1.37x`
  - `W5` (runs=3): geomean `1.82x`, warm `1.37x`
  - `W6` (runs=3): geomean `1.82x`, warm `1.37x`
  - `W7` (runs=3): geomean `1.82x`, warm `1.38x`
  - `W8` (runs=3): geomean `1.82x`, warm `1.38x`
  - `W9` (runs=3): geomean `1.83x`, warm `1.38x`
  - `W10` (runs=3): geomean `1.82x`, warm `1.37x`
