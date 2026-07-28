# Performance charter comparison suite

Authority: [ADR-018 §1](../../docs/adr/018-performance-charter-region-arenas-and-rc-elision.md),
R24, grill Q5 (user-ratified 2026-07-27). Workstream row:
[`docs/program/workstreams/perf.md`](../../docs/program/workstreams/perf.md),
PERF-SUITE. Issue: #186.

Every other PERF ticket closes on a measurement taken here: no measurement, no
close.

## Running it

```bash
just perf-suite                  # measure; writes target/perf-suite/report.json
just perf-suite-noise            # tripwire 1: two runs must agree
just perf-suite-integrity        # tripwire 3: benchmark-file hashes
```

`just perf-suite` builds `target/release/shape` first and records the hash of
the binary it actually executed. Pass `--no-build` to measure a binary that is
already there, `--iterations N` to trade time for stability, and
`--out <path>` to place the report.

To compare two reports (a lane's before and after):

```bash
cargo run -q -p xtask -- perf-suite compare before.json after.json
```

## What the suite contains

| Category | Workloads | Charter bar |
|---|---|---|
| `numeric` | `numeric_mandelbrot`, `numeric_matmul`, `numeric_spline` | ≥ 1.5× |
| `collections` | `collections_pipeline`, `collections_hashmap` | ≥ 1.0×, post closure-nativity |
| `strings_json` | `strings_transform`, `json_roundtrip` | ≥ 1.0× |
| `allocation` | `alloc_object_graph`, `alloc_tree` | ≥ 0.8×, ratcheting to ≥ 1.0× post-arena |
| `startup` | `startup_hello` | ≥ 5× |
| `closures` | `closures_dispatch` | none — informational |

A ratio is **reference time ÷ Shape time**: 1.5× means Shape is 1.5 times
faster than the pinned reference. Each Shape workload has a matched reference
workload computing the identical result; the harness compares the two outputs
and refuses to treat a workload as a comparison when they disagree.

## The reference runtime

Node, pinned in `manifest.json` by version, V8 build, and resolved binary path
(a nix store path is itself an exact pin). If no reference runtime is present,
the harness records `reference: unavailable` as a structured state and reports
Shape's measurements alone. It never fabricates a reference number.

## What the numbers mean, exactly

- The primary statistic is the **minimum** of N samples after a warmup run.
  Timing noise is one-sided — interference only ever adds time — so the minimum
  is the most robust estimator available.
- `ratio` is whole-process wall time on both sides. It includes each runtime's
  own startup. Because Shape's startup floor is large in absolute terms but
  small as a fraction of a multi-second workload, and the reference's is the
  reverse, **this primary ratio is the more flattering number for Shape**.
- `adj` is the same ratio after subtracting each runtime's own measured
  hello-world floor. It is the harsher number, and it is reported alongside,
  not instead. Where the reference's post-subtraction kernel time falls under
  50 ms the report marks the adjusted ratio low-confidence.
- A category whose prerequisite lane has not landed is reported as
  **measured (not gating)** with the precondition named. The suite does not
  claim a pass or a fail for a bar the charter does not yet assert.
- The charter permits exactly one recorded calibration of the multipliers after
  the first measured baseline. Until that calibration is recorded, bar outcomes
  here are measurements against the ratified targets, not a settled gate.

## What the suite does not claim

Recorded `[jit-fallback]` lines are observations, and the harness collects
them because they are the raw material PERF-DEOPT-GRANULARITY works from. The
**absence** of a fallback line is not evidence that a workload executed
natively. That claim requires R15's `NativeExecutionWitness` (NATIVE-WITNESS,
#117) and is out of scope here.

The fallback lines are also not fully deterministic: the same workload at the
same revision emits the whole-program bail on some runs and not others (a
`no JIT FuncRef (callee not in the compiled set)` ordering effect), with no
measurable difference in wall time either way.

## The three tripwires

1. **Reproducibility.** `just perf-suite-noise` runs the suite twice at one
   revision and asserts every measurement agrees within the manifest's declared
   bound (15%, declared from the first baseline, where the worst quiet-machine
   deviation observed was 8.3%). When it fails, the pinned reference acts as the
   control: if the reference moved by the same order on the same workload, the
   verdict is `machine_contended` — a statement about the machine, not about
   Shape. **Widening the bound to make a contended run pass is the failure mode
   this design exists to prevent.** Run it on an idle machine.
2. **Environment identity.** The report renders a comparison only when the
   captured environment identity equals the one pinned in `manifest.json`.
   Otherwise every ratio is withheld, the differing fields are named, and each
   bar becomes `not_evaluated`. Raw measurements are still recorded — refusing
   to compare is not refusing to measure. Re-pinning is
   `perf-suite record-environment`, which rewrites the manifest as a reviewable
   diff.
3. **Benchmark integrity.** Every workload source under `benchmarks/` is hashed
   into `integrity.sha256`; modification, deletion, and unrecorded addition are
   each violations. This is asserted as a unit test, so `just test-fast` fails
   on an edited benchmark, and a violation also refuses the comparison — a
   modified corpus is not the corpus the baseline describes. Result and tracking
   files are deliberately outside the covered set, because outputs are meant to
   change.

Benchmarks measure the compiler; the compiler does not get to rewrite the
benchmarks. Adding type annotations, restructuring, or inserting hints to help
the JIT is forbidden. The one annotation in these workloads that exists for the
compiler's benefit (`let serialized: string` in `json_roundtrip.shape`) is there
because the program does not compile without it, and says so in a comment.

## Files

- `manifest.json` — pinned environment identity, reference pin, category bars
  and their preconditions, workload registry, measurement settings.
- `integrity.sha256` — benchmark-file digests (`sha256sum` format).
- `shape/`, `node/` — the matched workload pairs.
- `baseline/first-baseline.json` — the first measured baseline, committed as
  evidence.
- Harness: `tools/xtask/src/perf_suite/`.
