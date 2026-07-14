# Wave-16 JIT GC Write-Barrier Perf Plan

Date: 2026-07-09
Role: Wave-16A JIT GC write-barrier perf scout

This is a command plan only. No cargo, build, test, or benchmark command was run
by this scout.

## Sources Checked

- `crates/shape-vm/Cargo.toml`: `shape-vm` defaults to `jit,gc`; `gc`
  forwards to `shape-value/gc`.
- `crates/shape-jit/Cargo.toml`: `shape-jit/gc` is default-off and gates the
  real JIT write-barrier/safepoint hook bodies.
- `docs/cluster-audits/gc-on-readiness-report.md`: the current CLI default build
  resolves `shape-jit/gc`, while `--no-default-features --features jit` keeps
  `shape-jit/gc` off but still leaves `shape-vm/gc` and `shape-value/gc` on
  through dependency defaults. This is a JIT-barrier-off comparator, not a full
  GC-off binary.
- `benchmarks/run_all.sh`: the stock harness rebuilds `shape` internally with
  `cargo build --release --bin shape --features shape-cli/jit`, so it must not be
  used unchanged for this two-binary comparison.
- `crates/shape-jit/src/mir_compiler/places.rs`: `inline_typed_field_set` emits
  an old-slot load plus `jit_write_barrier(old, new, tag)` only under
  `shape-jit/gc` and only for cycle-capable heap field kinds.
- `crates/shape-jit/src/executor.rs`: JIT compile fallthrough is visible as a
  `[jit-fallback]` stderr line; benchmark rows with that marker are interpreter
  rows, not native-JIT rows.

## Build Commands

Run from `/home/dev/dev/shape-lang/shape`. These commands isolate artifacts in
separate target dirs so the second build does not overwrite the first.

```bash
mkdir -p /tmp/shape-jit-gc-perf/results /tmp/shape-jit-gc-perf/logs
```

Shipped default, JIT plus JIT GC barrier:

```bash
systemd-run --user --wait --collect --pipe \
  -p WorkingDirectory=/home/dev/dev/shape-lang/shape \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 \
      CARGO_TARGET_DIR=/tmp/shape-jit-gc-perf/target/default \
      cargo build --release -p shape-cli --bin shape
```

JIT-barrier-off comparator:

```bash
systemd-run --user --wait --collect --pipe \
  -p WorkingDirectory=/home/dev/dev/shape-lang/shape \
  -p MemoryMax=12G -p MemorySwapMax=0 -p TasksMax=256 \
  env CARGO_BUILD_JOBS=2 \
      CARGO_TARGET_DIR=/tmp/shape-jit-gc-perf/target/jit-barrier-off \
      cargo build --release -p shape-cli --bin shape \
        --no-default-features --features jit
```

Artifacts:

```bash
DEFAULT_BIN=/tmp/shape-jit-gc-perf/target/default/release/shape
BARRIER_OFF_BIN=/tmp/shape-jit-gc-perf/target/jit-barrier-off/release/shape
```

## Existing Benchmark Subset

Use direct `shape -m jit` invocations, not `benchmarks/run_all.sh`, because
`run_all.sh` rebuilds the binary and its hard-coded list omits the object-heavy
rows after `10_primes_count`.

Existing rows worth classifying for this question:

```bash
EXISTING_BENCHES=(
  benchmarks/shape/03_sieve.shape
  benchmarks/shape/05_spectral.shape
  benchmarks/shape/06_ackermann.shape
  benchmarks/shape/09_matrix_mul.shape
  benchmarks/shape/11_object_property_loop.shape
  benchmarks/shape/12_polymorphic_dispatch.shape
  benchmarks/shape/13_hashmap_build_query.shape
  benchmarks/shape/14_string_concat.shape
  benchmarks/shape/15_gc_pressure_tree.shape
  benchmarks/shape/16_array_of_objects.shape
)
```

Classification pass, one run per variant and row:

```bash
systemd-run --user --wait --collect --pipe \
  -p WorkingDirectory=/home/dev/dev/shape-lang/shape \
  -p MemoryMax=4G -p MemorySwapMax=0 -p TasksMax=128 \
  bash -s <<'BASH'
set -euo pipefail
DEFAULT_BIN=/tmp/shape-jit-gc-perf/target/default/release/shape
BARRIER_OFF_BIN=/tmp/shape-jit-gc-perf/target/jit-barrier-off/release/shape
OUT=/tmp/shape-jit-gc-perf/results/existing-classification
mkdir -p "$OUT"
BENCHES=(
  benchmarks/shape/03_sieve.shape
  benchmarks/shape/05_spectral.shape
  benchmarks/shape/06_ackermann.shape
  benchmarks/shape/09_matrix_mul.shape
  benchmarks/shape/11_object_property_loop.shape
  benchmarks/shape/12_polymorphic_dispatch.shape
  benchmarks/shape/13_hashmap_build_query.shape
  benchmarks/shape/14_string_concat.shape
  benchmarks/shape/15_gc_pressure_tree.shape
  benchmarks/shape/16_array_of_objects.shape
)
printf "variant\tbench\tstatus\tfallback\n" > "$OUT/summary.tsv"
for variant in default barrier_off; do
  if [ "$variant" = default ]; then bin="$DEFAULT_BIN"; else bin="$BARRIER_OFF_BIN"; fi
  for bench in "${BENCHES[@]}"; do
    name="$(basename "$bench" .shape)"
    log="$OUT/${variant}_${name}.log"
    err="$OUT/${variant}_${name}.err"
    status=0
    timeout 120 "$bin" -m jit "$bench" >"$log" 2>"$err" || status=$?
    if grep -q "\[jit-fallback\]" "$err"; then fallback=yes; else fallback=no; fi
    printf "%s\t%s\t%s\t%s\n" "$variant" "$name" "$status" "$fallback" >> "$OUT/summary.tsv"
  done
done
cat "$OUT/summary.tsv"
BASH
```

Timed pass for rows that classify as `fallback=no` in both variants:

```bash
systemd-run --user --wait --collect --pipe \
  -p WorkingDirectory=/home/dev/dev/shape-lang/shape \
  -p MemoryMax=4G -p MemorySwapMax=0 -p TasksMax=128 \
  bash -s <<'BASH'
set -euo pipefail
DEFAULT_BIN=/tmp/shape-jit-gc-perf/target/default/release/shape
BARRIER_OFF_BIN=/tmp/shape-jit-gc-perf/target/jit-barrier-off/release/shape
CLASS=/tmp/shape-jit-gc-perf/results/existing-classification/summary.tsv
OUT=/tmp/shape-jit-gc-perf/results/existing-timing
mkdir -p "$OUT"
awk -F '\t' '$1 == "default" && $4 == "no" {print $2}' "$CLASS" | sort > "$OUT/default.native"
awk -F '\t' '$1 == "barrier_off" && $4 == "no" {print $2}' "$CLASS" | sort > "$OUT/barrier_off.native"
comm -12 "$OUT/default.native" "$OUT/barrier_off.native" > "$OUT/native_rows.txt"
printf "variant\tbench\trun\telapsed_s\tmaxrss_kb\n" > "$OUT/timing.tsv"
printf "variant\tbench\trun\tstatus\tfallback\n" > "$OUT/status.tsv"
for variant in default barrier_off; do
  if [ "$variant" = default ]; then bin="$DEFAULT_BIN"; else bin="$BARRIER_OFF_BIN"; fi
  while read -r name; do
    [ -n "$name" ] || continue
    bench="benchmarks/shape/${name}.shape"
    for run in 1 2 3 4 5; do
      log="$OUT/${variant}_${name}_${run}.log"
      err="$OUT/${variant}_${name}_${run}.err"
      status=0
      /usr/bin/time -f "${variant}\t${name}\t${run}\t%e\t%M" -a -o "$OUT/timing.tsv" \
        timeout 120 "$bin" -m jit "$bench" >"$log" 2>"$err" ||
        status=$?
      if grep -q "\[jit-fallback\]" "$err"; then
        fallback=yes
      else
        fallback=no
      fi
      printf "%s\t%s\t%s\t%s\t%s\n" "$variant" "$name" "$run" "$status" "$fallback" >> "$OUT/status.tsv"
      if [ "$fallback" = yes ]; then
        printf "fallback during timing: %s %s run %s\n" "$variant" "$name" "$run" >&2
        exit 2
      fi
    done
  done < "$OUT/native_rows.txt"
done
BASH
```

The timing block writes raw elapsed/RSS data to `timing.tsv` and exit/fallback
classification to `status.tsv`.

## Coverage Finding

The existing `benchmarks/shape` corpus does not contain a JIT-hot heap field
overwrite benchmark:

- `11_object_property_loop` and `12_polymorphic_dispatch` allocate typed objects
  and read scalar fields; they do not overwrite heap fields.
- `16_array_of_objects` allocates objects and pushes them into an array, then
  reads scalar fields; it does not overwrite object fields.
- `03_sieve`, `05_spectral`, and `09_matrix_mul` mutate arrays, but with scalar
  bool/number elements rather than cycle-capable heap object fields.
- `13_hashmap_build_query` and `14_string_concat` are heap-ish workloads but do
  not target the JIT typed-object field-store barrier.
- `15_gc_pressure_tree` is scalar recursion despite its name.

So existing rows can provide native-JIT control data and fallback classification,
but they cannot prove the cost of the JIT write barrier on the path that matters:
repeated overwrite of a cycle-capable heap field by native JIT code.

## Proposed Fixture

Do not land this from the scout role. Add it only in a later implementation lane
or create it under `/tmp/shape-jit-gc-perf/fixtures` for the supervisor run. It
must be discarded as perf evidence if either binary emits `[jit-fallback]`.

Heap-field overwrite probe:

```shape
type Node {
    peer: Option<Node>,
    payload: int
}

function heap_field_existing_swap(n: int) -> int {
    let a = Node { peer: None, payload: 1 }
    let b = Node { peer: None, payload: 2 }
    let c = Node { peer: None, payload: 3 }
    b.peer = Some(b)
    c.peer = Some(c)

    var checksum = 0
    var i = 0
    while i < n {
        if i % 2 == 0 {
            a.peer = Some(b)
            checksum = checksum + 1
        } else {
            a.peer = Some(c)
            checksum = checksum + 2
        }
        i = i + 1
    }
    return checksum
}

print(heap_field_existing_swap(5000000))
```

Scalar-field control with the same loop shape:

```shape
type ScalarBox {
    value: int
}

function scalar_field_overwrite(n: int) -> int {
    let holder = ScalarBox { value: 0 }
    var checksum = 0
    var i = 0
    while i < n {
        holder.value = i
        checksum = checksum + holder.value
        i = i + 1
    }
    return checksum
}

print(scalar_field_overwrite(5000000))
```

Interpret the heap probe only with the scalar-field control beside it. The heap
probe targets the barrier path; the scalar probe controls for loop, field-store,
and binary-difference noise that is unrelated to `jit_write_barrier`.

## Expected Interpretation

- If an existing row is `fallback=yes`, its timing is VM timing and must not be
  used for JIT barrier conclusions.
- If an existing row is `fallback=no`, it is still only a control unless it
  overwrites a cycle-capable heap field. Current existing rows do not.
- If the proposed heap-field fixture is `fallback=no` in both binaries, compare
  5-run medians of default vs JIT-barrier-off, then subtract/contrast the scalar
  field-control delta. That is the closest shipped-binary estimate of JIT
  write-barrier cost.
- If the proposed heap-field fixture falls back, the honest result is "unmeasured
  on native JIT"; the next step is either a JIT lowering fix for that source
  shape or a lower-level MIR/FFI microbench that calls the native field-store
  path directly.

## Risks

- The comparator is not full GC-off. It isolates `shape-jit/gc` off while VM and
  value GC stay enabled through dependency defaults.
- Collection and candidate-buffer behavior may be part of the shipped cost. If
  the goal is only the fast-path call overhead, the fixture needs to avoid
  triggering collection, or the supervisor needs a lower-level JIT/FFI benchmark.
- CPU scheduling noise can swamp small deltas. Prefer pinned cores or isolated
  runners for final numbers; at minimum use medians and keep raw logs.
- Do not use `benchmarks/run_all.sh` unchanged for this comparison. It rebuilds
  the binary internally and is not artifact-selectable.
