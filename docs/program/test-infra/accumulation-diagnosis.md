# Test-infra memory diagnosis (#206)

**Date:** 2026-07-28 · **Baseline commit:** `38345e03` · **Branch:** `wave1-infra`
**Host:** 94 GB RAM, `nproc` = 32, GNU `ld`, no `mold`/`lld` installed.

## Headline

**The ticket's premise is refuted. There is no meaningful per-test resource
accumulation.** A single process runs the entire 3,583-test `shape-vm --lib`
suite at `--test-threads=1` with a peak RSS of **113–121 MB**.

The ~60 GB OOM is a **build-phase** failure, not a test-run failure:
`cargo test -p shape-test` links ~60 integration-test binaries, each a
workspace-sized static link costing ~1.8 GB of linker RSS, and Cargo releases
them to the jobserver simultaneously. Peak build RSS is linear in `jobs`:

| `--jobs` | peak tree RSS | processes at peak | GB per job |
|---:|---:|---:|---:|
| 8  | 15.1 GB | 41  | 1.89 |
| 16 | 29.2 GB | 81  | 1.83 |
| 32 (default = nproc) | **≥29.8 GB and still climbing when the 20 GB safety cap fired at t=6.3s** | 157 | — |

Extrapolating the measured 1.86 GB/job to the default `jobs = 32` predicts
**~59 GB**, which matches the reported ~60 GB. Two concurrent worktrees each
building at default parallelism is not required to explain the incident; one is
almost enough.

## Method

Two samplers were added, both of which hard-kill their subject if it crosses a
cap, so a memory investigation can never itself OOM the host:

- `scripts/rss-profile.sh` — samples one test process's `VmRSS` and, at each
  sample, the number of tests completed. Emits `(elapsed, tests_done, rss_kb)`,
  which is the per-test retention profile directly, with no timestamp joining.
- `scripts/rss-tree-profile.sh` — sums RSS across a whole process *tree*
  (`setsid` + process-group match). `cargo test` is cargo + N rustc + N linkers
  + test binaries; only a tree measurement attributes peak memory between the
  build and run phases.

No test semantics were changed and no test was skipped or deleted.

## Evidence per suspected source

### Ruled out — per-test accumulation in the VM (Arc cycles, VM teardown)

`shape-vm --lib`, all 3,583 tests, one process, `--test-threads=1`:

```
tests=296   rss= 65 MB      tests=1794  rss= 86 MB      tests=3014  rss= 89 MB
tests=594   rss= 68 MB      tests=2083  rss= 87 MB      tests=3488  rss= 92 MB
tests=1001  rss= 77 MB      tests=2551  rss= 88 MB      tests=3531  rss= 92 MB
tests=1265  rss= 84 MB      tests=2784  rss= 87 MB      peak       113 MB
```

Steady-state least-squares slope past warmup: **9.2 KB/test**. RSS goes flat
after ~1,500 tests and even dips (2,551 → 2,784 tests: 88 → 87 MB), which is
inconsistent with monotonic retention. Whatever Arc cycles a dropped
`VirtualMachine` leaves behind, they do not add up to a memory problem: 3,583
VM lifecycles cost ~27 MB total.

At default parallelism (32 threads) the same suite peaks at 1.17 GB in 8s —
that is 32 concurrent VMs, i.e. concurrency, not accumulation.

### Ruled out — JIT code pages

JIT-heavy targets show no elevated retention relative to interpreter-only ones:

| shape-test binary | tests | peak RSS |
|---|---:|---:|
| `numeric_conversions_jit` | 118 | 390 MB |
| `jit` | 44 | 313 MB |
| `operators` | 618 | 363 MB |
| `control_flow` | 490 | 360 MB |
| `snapshots_resume` | — | 180 MB |

`jit` (44 tests) retains *less* than `operators` (618 tests). If code pages
leaked per JIT-compiled function, the JIT targets would dominate. They do not.

### Ruled out — unbounded static caches / interners

A monotonic global cache would produce a constant positive slope. The measured
slope is 9.2 KB/test and the curve flattens. Over the whole suite this is ~30 MB
— real but three orders of magnitude away from the incident.

### Confirmed — build-phase link fan-out

`cargo test -p shape-test --no-run` after touching `tools/shape-test/src/lib.rs`
(forces relink of all ~60 test binaries), default parallelism:

```
t=0.0  procs=2    tree= 0.0 GB  top=direnv/5MB
t=1.0  procs=2    tree= 0.7 GB  top=rustc/621MB
t=3.0  procs=2    tree= 1.1 GB  top=rustc/989MB
t=4.1  procs=5    tree= 1.6 GB  top=rustc/578MB
t=5.2  procs=157  tree=10.5 GB  top=ld/248MB      <-- fan-out
t=6.3  procs=161  tree=28.4 GB  top=ld/859MB      <-- +18 GB in one second
                                                   (safety cap fired here)
```

The transition at t=5.2s is the whole defect: 5 processes become 157 as every
test binary starts linking at once. Individual `ld` processes peak at ~1.7 GB;
test binaries are ~230 MB each with full debuginfo (16 GB of `target/debug/deps`).

## Interventions measured

| intervention | peak build RSS |
|---|---:|
| default (`jobs=32`) | ~59 GB (projected; ≥29.8 GB measured before cap) |
| `jobs=16` | 29.2 GB |
| **`jobs=8`** | **15.1 GB** (14.2 GB re-measured via config) |
| `jobs=8` + `split-debuginfo=unpacked` | 11.7 GB |
| `jobs=8` + `debug=line-tables-only` | 12.0 GB |

Parallelism is the dominant lever by a wide margin; the debuginfo levers buy
another ~20% and cost debuggability, so they are **not** applied. The fix is the
`jobs` bound alone.

## Why the existing guard never fired

`justfile` carries `test-mem-cap-kib := "50331648"` (48 GiB) applied as
`ulimit -v`. That is a **per-process** virtual-memory cap. The real failure is
~32 concurrent processes of ~1.8 GB each; no single one approaches 48 GiB, so
this backstop cannot fire for this failure mode. It remains useful for the
runaway-single-test case it was written for; it is simply aimed at a different
shape of failure.

## The two siblings

**The 613-test bulk hang does not reproduce at `38345e03`.** The suite is
`tests/operators` (now 618 tests). At `--test-threads=1` it completes in 105s
with a flat 52 → 55 MB RSS; at default parallelism, 618 passed in 11.8s. The
most likely historical explanation is that a bulk run was performed right after
a change, so it included a full rebuild; the build drove the host into swap and
the run appeared to hang. That is consistent with the original report that
subsets and `--exact` runs were fine — those were typically issued against an
already-built target directory.

**The 42-failure parallel-load spook does not reproduce either.** The suite
returns **exactly 7 failures in both modes** — `--test-threads=1` (89.5s) and
default 32-thread parallelism (21.7s) — and they are the known pre-existing #204
set. Parallelism changed the failure count by zero here. This does not prove the
flake is gone, but it removes memory pressure as an explanation on a
non-contended host, and points at #205's nondeterministic selection instead.

## Fix landed

1. `.cargo/config.toml` — `[build] jobs = 8`. Declared bound: **peak build RSS
   ≤ ~16 GB** (measured 14.2–15.1 GB).
2. `scripts/rss-tripwire.sh` + `just rss-tripwire` — CI-runnable, two guards:
   - the build-parallelism bound is still configured and `≤ 12`;
   - one process over the full `shape-vm --lib` slice stays under **256 MB peak**
     (measured 113–121 MB) and **24 KB/test slope** (measured 9.2 KB/test).

Both bounds sit at roughly 2–3x measured, enough to absorb allocator and libtest
noise while still failing loudly on a real regression.

## Open items

- The residual 9.2 KB/test slope is small and flattening, but its source was not
  chased to a named allocation. The tripwire now bounds it, so a regression
  surfaces as a test failure rather than as growth nobody is watching.
- `mold`/`lld` are not installed on this host. Either would cut per-link RSS
  substantially and allow raising `jobs` back up; that is a toolchain/infra
  change, deliberately not made here.
- `.cargo/config.toml`'s comment notes an infra-level `CARGO_BUILD_JOBS`
  deployment as the intended longer-term home for this bound.
