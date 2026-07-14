# Wave 19 JIT GC Barrier Current Results

Date: 2026-07-09
Supervisor: book-truth completeness campaign

## Question

GC is enabled in the shipped default binary, so the remaining performance
question is native-JIT write-barrier overhead. This pass compared:

- default release binary: `jit + gc`;
- comparator release binary: `--no-default-features --features jit`.

As in Wave 16, this is a JIT-barrier-off comparison, not a full whole-stack
collector-off comparison, because dependency defaults can still pull GC-capable
runtime crates into the CLI graph.

## Artifacts

Both artifacts were rebuilt after the Wave-19 JIT changes:

| Artifact | Command shape | Service | Result |
|---|---|---|---|
| default | `cargo build --release -p shape-cli --bin shape` | `run-p3600535-i30009686.service` | passed |
| JIT barrier off | `cargo build --release -p shape-cli --bin shape --no-default-features --features jit` | `run-p3600535-i30009686.service` | passed |

Raw probe output lives under `/tmp/shape-jit-gc-perf/results/`.

## Classification

Direct artifact classification used `shape -m jit` with global extension loading
disabled via an empty `SHAPE_CONFIG_DIR`.

Evidence: `run-p3664166-i30076327.service`.

Rows that were native JIT in both variants:

| Workload | Default output | Comparator output |
|---|---:|---:|
| `06_ackermann` | `8189` | `8189` |
| `15_gc_pressure_tree` | `5242875` | `5242875` |
| `18_jit_scalar_field_overwrite` | `12499997500000` | `12499997500000` |

Heap-field overwrite probes still deopt in both variants:

| Workload | Output | Current blocker |
|---|---:|---|
| `17_jit_heap_field_overwrite` | `7500000` | v0.3.3 move-semantics surface |
| `19_jit_heap_field_overwrite_fresh` | `12499997500000` | v0.3.3 move-semantics surface |
| `20_jit_heap_field_overwrite_function` | `12499997500000` | callee failed Phase-4 JIT compile; top-level could not resolve `main_f195_overwrite_child` |

## Native-JIT Timing

Evidence: `run-p3666492-i30078743.service`, five runs per variant and workload.

| Workload | Default median | Comparator median | Delta |
|---|---:|---:|---:|
| `06_ackermann` | `271ms` | `267ms` | `+1.5%` |
| `15_gc_pressure_tree` | `188ms` | `194ms` | `-3.1%` |
| `18_jit_scalar_field_overwrite` | `179ms` | `179ms` | `0.0%` |

These are noise-level results and match the expected shape: compute-bound and
scalar-field mutation workloads do not expose a meaningful GC/barrier cost.

## Wave-20 Native Heap Result

Wave-20A made two heap typed-field overwrite probes native-JIT eligible in both
artifacts while preserving conservative fallback for unsafe direct field reads.

Evidence:

- Final release rebuild: `run-p3799627-i30218114.service`.
- Final native/fallback classification: `run-p3802488-i30221202.service`.
- Book regression subset after the safety gates:
  `run-p3803693-i30222464.service`.
- Full book gate after the safety gates:
  `run-p3804871-i30223682.service`, report
  `/tmp/shape-wave20-book-truth-report.json`.
- Final timing raw data:
  `/tmp/shape-jit-gc-perf/results/wave20-native-heap-timing-final/timing.tsv`.

Final classification:

| Workload | Default | Comparator | Output |
|---|---|---|---:|
| `17_jit_heap_field_overwrite` | native | native | `7500000` |
| `20_jit_heap_field_overwrite_function` | native | native | `12499997500000` |
| `06_ackermann` | native | native | `8189` |
| `19_jit_heap_field_overwrite_fresh` | fallback | fallback | `12499997500000` |
| `18_jit_scalar_field_overwrite` | fallback | fallback | `12499997500000` |

The remaining fallbacks are intentional:

- `19` and `18` read fields in the hot path, and direct `Place::Field` reads
  still have an unproven native lowering for object/trait cases.
- Top-level field writes and field-address creation remain native, so `17` and
  `20` exercise `jit_typed_object_set_field` and the GC write-barrier path.

Five-run medians for the native rows:

| Workload | Default median | Comparator median | Delta |
|---|---:|---:|---:|
| `17_jit_heap_field_overwrite` | `1249ms` | `1358ms` | `-8.0%` |
| `20_jit_heap_field_overwrite_function` | `3059ms` | `3060ms` | `0.0%` |
| `06_ackermann` | `262ms` | `258ms` | `+1.6%` |

The timing cgroup completed all timed executions with `fallback=no`; its inline
summary step failed on shell quoting after the raw TSV was written, so
`summary.tsv` was generated separately from the captured data.

## Conclusion

The current shipped-binary comparison is clean for native-JIT controls that do
not overwrite heap fields: no measurable regression from the default GC/barrier
feature set.

The heap write-barrier cost is now measured on native-JIT heap field overwrite
workloads. This run does not show a hot-path regression from enabling GC by
default: the loop overwrite case was faster in the shipped binary on this run,
the function-local overwrite case was tied, and compute-bound overhead remained
noise-level. The remaining honest JIT gap is direct field-read lowering, not
heap field mutation/write-barrier overhead.
