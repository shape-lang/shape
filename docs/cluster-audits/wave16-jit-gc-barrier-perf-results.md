# Wave 16 JIT GC Barrier Perf Results

Date: 2026-07-09
Supervisor: book-truth completeness campaign

## Question

GC is now on by default for the shipped binary. The earlier readiness report
measured write-barrier overhead on the interpreter only. This wave tried to
measure the shipped native-JIT path by comparing:

- default release binary: `jit + gc`;
- comparator release binary: `--no-default-features --features jit`.

Important caveat: in the current package graph the comparator disables the
`shape-jit/gc` path, but it is not a clean whole-stack `gc-off` binary.
`shape-cli` still reaches `shape-vm`/`shape-value` defaults through dependency
features. This is a JIT-barrier-off comparison, not a complete collector-off
comparison.

## Artifacts

Both artifacts were built in isolated target dirs under cgroups:

| Artifact | Command shape | Service | Result |
|---|---|---|---|
| default | `cargo build --release -p shape-cli --bin shape` | `run-p3032237-i29425116.service` | passed |
| JIT barrier off | `cargo build --release -p shape-cli --bin shape --no-default-features --features jit` | `run-p3059762-i29453954.service` | passed |

Raw results live under `/tmp/shape-jit-gc-perf/results/`.

## Existing Benchmark Classification

`benchmarks/run_all.sh` rebuilds internally, so it cannot compare the two
prebuilt artifacts unchanged. I classified direct artifact runs instead.

The useful native-JIT control is `benchmarks/shape/06_ackermann.shape`:

| Workload | Default status | Comparator status | JIT fallback | Output |
|---|---:|---:|---|---|
| `06_ackermann` | 0 | 0 | no | `8189` |

Most other existing benchmark rows are not useful for this question because
they either fall back to the interpreter or currently fail type/compile checks.
Examples:

- fallback controls: `03_sieve`, `11_object_property_loop`,
  `14_string_concat`, `15_gc_pressure_tree`;
- current type/compile failures: `05_spectral`, `09_matrix_mul`,
  `12_polymorphic_dispatch`, `13_hashmap_build_query`,
  `16_array_of_objects`.

Classification evidence:

- `run-p3087528-i29482830.service`: initial existing benchmark plus custom
  classifier pass;
- `run-p3089870-i29485281.service`: rerun after fixture mutability fix;
- `run-p3093566-i29489190.service`: top-level mutation fixture classifier.

## Native Compute-Bound Result

The native-JIT compute-bound control is clean and shows no meaningful barrier
cost:

| Variant | Runs, seconds | Median |
|---|---|---:|
| default | `0.27, 0.28, 0.28, 0.27, 0.27` | `0.27s` |
| JIT barrier off | `0.27, 0.29, 0.28, 0.29, 0.27` | `0.28s` |

Delta is noise-level (`-3.6%` by the one-decimal summary table, default
slightly faster). This matches the expected shape: compute-bound code with no
heap field overwrite does not expose a barrier regression.

Timing evidence: `run-p3092390-i29487951.service`.

## Mutation-Hot Probes

I added two focused source fixtures so the gap is reproducible:

- `benchmarks/shape/17_jit_heap_field_overwrite.shape`
- `benchmarks/shape/18_jit_scalar_field_overwrite.shape`

They exercise repeated typed-object field overwrites:

| Fixture | Default status | Comparator status | Output | Native JIT? |
|---|---:|---:|---:|---|
| `17_jit_heap_field_overwrite` | 0 | 0 | `7500000` | no, fallback |
| `18_jit_scalar_field_overwrite` | 0 | 0 | `12499997500000` | no, fallback |

Both variants fall back for the same reason before the barrier can be timed:
the current JIT move-semantics surface deopts when a `Move`/`MoveExplicit`
sourced slot is read at a later program point. The diagnostic identifies
ADR-006 section 2.7.14 and says the root fix is per-point Copy/Clone/Move
liveness in JIT operand lowering.

## Wave 17 Follow-Up

Wave 17 narrowed the JIT move/read fallback enough for the scalar field
overwrite control to reach native JIT. The repair aligned the detector and
operand lowering with the JIT's existing `NativeKind::is_refcounted()`
discriminator, so scalar values and scalar typed-object fields are treated as
non-destructive reads even when older `LocalTypeInfo` metadata is still
`Unknown`.

Build evidence:

| Artifact | Service | Result |
|---|---|---|
| default | `run-p3399821-i29802298.service` | passed |
| JIT barrier off | `run-p3401191-i29803773.service` | passed |

Classification evidence: `run-p3403181-i29805881.service`.

| Fixture | Default native JIT? | Comparator native JIT? | Current read |
|---|---|---|---|
| `17_jit_heap_field_overwrite` | no, fallback | no, fallback | Still blocked before heap-barrier timing; final diagnostic is the Wave-17 field-projection assignment preflight exposed by the scalar move-lift. |
| `18_jit_scalar_field_overwrite` | yes | yes | Native-JIT scalar control is now measurable. |

Scalar timing evidence: `run-p3403599-i29806316.service`, `hyperfine --warmup
3 --runs 10`.

| Variant | Mean | Stddev | Median |
|---|---:|---:|---:|
| default | `0.193522s` | `0.002284s` | `0.192652s` |
| JIT barrier off | `0.193487s` | `0.002028s` | `0.193521s` |

Delta default-vs-barrier-off is `+0.02%`, inside measurement noise and in the
expected direction for a control workload with no heap field overwrite.

## Conclusion

The direct shipped-binary measurement has now closed the compute-bound and
scalar mutation controls: neither shows a meaningful regression from enabling
GC/barrier features.

The important mutation-heavy heap question remains open for a sharper reason:
Shape source that performs hot heap typed-object field overwrites still deopts
to the interpreter before it reaches a native-JIT write-barrier fast path. That
means there is not yet an honest native-JIT barrier cost number for heap field
mutation.

Recommended next lane:

1. Fix or narrow the remaining ADR-006 2.7.14 JIT move-semantics fallback for
   `17_jit_heap_field_overwrite`.
2. Rerun the same default vs JIT-barrier-off comparison.
3. If mutation-heavy native JIT then shows a regression, do the barrier
   fast-path review requested by the GC readiness report.
