#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")/.."
TRACKING_DIR="$SCRIPT_DIR/tracking"
SNAPSHOT_DIR="$TRACKING_DIR/snapshots"

WEEK=""
NOTES=""
FAST=true
RUNS=5
PIN_CORE=""
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --week)
      WEEK="$2"
      shift 2
      ;;
    --notes)
      NOTES="$2"
      shift 2
      ;;
    --no-fast)
      FAST=false
      shift
      ;;
    --runs)
      RUNS="$2"
      shift 2
      ;;
    --pin-core)
      PIN_CORE="$2"
      shift 2
      ;;
    --)
      shift
      EXTRA_ARGS+=("$@")
      break
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ -z "$WEEK" ]]; then
  echo "Usage: $0 --week W1 [--notes \"...\"] [--runs N] [--pin-core N] [--no-fast] [-- <run_all args>]"
  exit 2
fi

if ! [[ "$RUNS" =~ ^[1-9][0-9]*$ ]]; then
  echo "--runs must be a positive integer"
  exit 2
fi

if [[ -n "$PIN_CORE" ]]; then
  if ! [[ "$PIN_CORE" =~ ^[0-9]+$ ]]; then
    echo "--pin-core must be a non-negative integer CPU index"
    exit 2
  fi
  if ! command -v taskset >/dev/null 2>&1; then
    echo "--pin-core requested but taskset is not available"
    exit 2
  fi
fi

mkdir -p "$SNAPSHOT_DIR"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

is_numeric() {
  [[ "$1" =~ ^-?[0-9]+([.][0-9]+)?$ ]]
}

first_non_empty() {
  for val in "$@"; do
    if [[ -n "$val" ]]; then
      printf "%s\n" "$val"
      return
    fi
  done
  printf "\n"
}

median_numeric() {
  if [[ $# -eq 0 ]]; then
    printf "\n"
    return
  fi
  printf "%s\n" "$@" | awk '
    { vals[++n] = $1 }
    END {
      if (n == 0) {
        print ""
        exit
      }
      asort(vals)
      if (n % 2 == 1) {
        print vals[(n + 1) / 2]
      } else {
        print (vals[n / 2] + vals[n / 2 + 1]) / 2.0
      }
    }
  '
}

min_numeric() {
  if [[ $# -eq 0 ]]; then
    printf "\n"
    return
  fi
  printf "%s\n" "$@" | sort -g | head -n 1
}

max_numeric() {
  if [[ $# -eq 0 ]]; then
    printf "\n"
    return
  fi
  printf "%s\n" "$@" | sort -g | tail -n 1
}

format_float2() {
  local val="$1"
  if [[ -z "$val" ]]; then
    printf "\n"
  else
    awk -v v="$val" 'BEGIN { printf "%.2f\n", v }'
  fi
}

format_int() {
  local val="$1"
  if [[ -z "$val" ]]; then
    printf "\n"
  else
    awk -v v="$val" 'BEGIN { printf "%.0f\n", v }'
  fi
}

run_args=()
if $FAST; then
  run_args+=(--fast)
fi
run_args+=("${EXTRA_ARGS[@]}")

ratio_files=()
metrics_files=()
run_snapshot_files=()
geomean_samples=()
warm_geomean_samples=()

for ((run = 1; run <= RUNS; run++)); do
  tmp_ratios="$tmp_dir/ratios_run${run}.tsv"
  tmp_metrics="$tmp_dir/metrics_run${run}.tsv"
  cmd=("$SCRIPT_DIR/run_all.sh" "${run_args[@]}" --write-ratios "$tmp_ratios" --write-metrics "$tmp_metrics")
  if [[ -n "$PIN_CORE" ]]; then
    cmd=(taskset -c "$PIN_CORE" "${cmd[@]}")
  fi

  echo "[track-week] run ${run}/${RUNS}"
  "${cmd[@]}"

  ratio_files+=("$tmp_ratios")
  metrics_files+=("$tmp_metrics")

  g="$(awk '$1=="GEOMEAN"{print $2}' "$tmp_ratios" | tail -n 1)"
  w="$(awk '$1=="GEOMEAN_WARM"{print $2}' "$tmp_ratios" | tail -n 1)"
  if [[ -n "$g" ]] && is_numeric "$g"; then
    geomean_samples+=("$g")
  fi
  if [[ -n "$w" ]] && is_numeric "$w"; then
    warm_geomean_samples+=("$w")
  fi
done

if [[ ${#metrics_files[@]} -eq 0 ]]; then
  echo "No benchmark runs completed."
  exit 1
fi

agg_metrics="$tmp_dir/metrics_aggregated.tsv"
{
  echo -e "benchmark\trust_ms\tnode_ms\tpython_ms\tvm_ms\tjit_ms\tjit_node_ratio\tjit_node_warm_ratio\tjit_node_ratio_100x\tjit_node_warm_ratio_100x\ttyped_numeric_ops\tgeneric_numeric_ops\tbytecode_compile_ms\tjit_compile_ms\tjit_exec_ms\tgeomean_jit_node\twarm_geomean_jit_exec_node"
} > "$agg_metrics"

mapfile -t benches < <(awk -F'\t' 'NR>1 && $1!="__summary__" {print $1}' "${metrics_files[0]}")

collect_values() {
  local bench="$1"
  local col="$2"
  local out=()
  local file
  for file in "${metrics_files[@]}"; do
    local val
    val="$(awk -F'\t' -v b="$bench" -v c="$col" '$1==b {print $c; exit}' "$file")"
    out+=("$val")
  done
  printf "%s\n" "${out[@]}"
}

median_field() {
  local bench="$1"
  local col="$2"
  local mode="$3"
  local vals=()
  local numeric_vals=()
  local line
  while IFS= read -r line; do
    vals+=("$line")
    if is_numeric "$line"; then
      numeric_vals+=("$line")
    fi
  done < <(collect_values "$bench" "$col")

  if [[ "$mode" == "string" ]]; then
    first_non_empty "${vals[@]}"
    return
  fi

  if [[ ${#numeric_vals[@]} -eq 0 ]]; then
    first_non_empty "${vals[@]}"
    return
  fi

  local median
  median="$(median_numeric "${numeric_vals[@]}")"
  if [[ "$mode" == "int" ]]; then
    format_int "$median"
  else
    format_float2 "$median"
  fi
}

for bench in "${benches[@]}"; do
  rust_ms="$(median_field "$bench" 2 int)"
  node_ms="$(median_field "$bench" 3 int)"
  python_ms="$(median_field "$bench" 4 int)"
  vm_ms="$(median_field "$bench" 5 int)"
  jit_ms="$(median_field "$bench" 6 int)"
  jit_node_ratio="$(median_field "$bench" 7 float)"
  jit_node_warm_ratio="$(median_field "$bench" 8 float)"
  jit_node_ratio_100x="$(median_field "$bench" 9 int)"
  jit_node_warm_ratio_100x="$(median_field "$bench" 10 int)"
  typed_numeric_ops="$(median_field "$bench" 11 int)"
  generic_numeric_ops="$(median_field "$bench" 12 int)"
  bytecode_compile_ms="$(median_field "$bench" 13 int)"
  jit_compile_ms="$(median_field "$bench" 14 int)"
  jit_exec_ms="$(median_field "$bench" 15 int)"

  echo -e "${bench}\t${rust_ms}\t${node_ms}\t${python_ms}\t${vm_ms}\t${jit_ms}\t${jit_node_ratio}\t${jit_node_warm_ratio}\t${jit_node_ratio_100x}\t${jit_node_warm_ratio_100x}\t${typed_numeric_ops}\t${generic_numeric_ops}\t${bytecode_compile_ms}\t${jit_compile_ms}\t${jit_exec_ms}\t\t" >> "$agg_metrics"
done

geomean="$(format_float2 "$(median_numeric "${geomean_samples[@]}")")"
warm_geomean="$(format_float2 "$(median_numeric "${warm_geomean_samples[@]}")")"
geomean_min="$(format_float2 "$(min_numeric "${geomean_samples[@]}")")"
geomean_max="$(format_float2 "$(max_numeric "${geomean_samples[@]}")")"
warm_geomean_min="$(format_float2 "$(min_numeric "${warm_geomean_samples[@]}")")"
warm_geomean_max="$(format_float2 "$(max_numeric "${warm_geomean_samples[@]}")")"

echo -e "__summary__\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t${geomean}\t${warm_geomean}" >> "$agg_metrics"

date_utc="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
date_slug="$(date -u +"%Y%m%d_%H%M%S")"
git_rev="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo "unknown")"
snapshot_rel="shape/benchmarks/tracking/snapshots/${date_slug}_${WEEK}.tsv"
snapshot_abs="$ROOT_DIR/$snapshot_rel"
cp "$agg_metrics" "$snapshot_abs"

for ((run = 1; run <= RUNS; run++)); do
  run_snapshot_rel="shape/benchmarks/tracking/snapshots/${date_slug}_${WEEK}_run${run}.tsv"
  run_snapshot_abs="$ROOT_DIR/$run_snapshot_rel"
  cp "${metrics_files[$((run - 1))]}" "$run_snapshot_abs"
  run_snapshot_files+=("$run_snapshot_rel")
done

wins="$(awk -F'\t' 'NR>1 && $1!="__summary__" && $9!="" {if (($9 + 0) <= 100) w++} END {print w + 0}' "$agg_metrics")"
losses="$(awk -F'\t' 'NR>1 && $1!="__summary__" && $9!="" {if (($9 + 0) > 100) l++} END {print l + 0}' "$agg_metrics")"
notes_clean="$(printf "%s" "$NOTES" | tr '\t\r\n' '   ')"
if [[ -n "$notes_clean" ]]; then
  notes_clean="${notes_clean}; "
fi
notes_clean="${notes_clean}runs=${RUNS}; geomean_range=${geomean_min}-${geomean_max}; warm_range=${warm_geomean_min}-${warm_geomean_max}; pin_core=${PIN_CORE:-none}"

summary_file="$TRACKING_DIR/jit_weekly_history.tsv"
if [[ ! -f "$summary_file" ]]; then
  echo -e "date_utc\tweek\tgit_rev\tgeomean_jit_node\twarm_geomean_jit_exec_node\twins\tlosses\tsnapshot_file\tnotes" > "$summary_file"
fi
echo -e "${date_utc}\t${WEEK}\t${git_rev}\t${geomean}\t${warm_geomean}\t${wins}\t${losses}\t${snapshot_rel}\t${notes_clean}" >> "$summary_file"

run_history_file="$TRACKING_DIR/jit_weekly_run_history.tsv"
if [[ ! -f "$run_history_file" ]]; then
  echo -e "date_utc\tweek\tgit_rev\trun_index\tgeomean_jit_node\twarm_geomean_jit_exec_node\tsnapshot_file\tpin_core\tnotes" > "$run_history_file"
fi
for ((run = 1; run <= RUNS; run++)); do
  run_ratio_file="${ratio_files[$((run - 1))]}"
  run_geomean="$(awk '$1=="GEOMEAN"{print $2}' "$run_ratio_file" | tail -n 1)"
  run_warm_geomean="$(awk '$1=="GEOMEAN_WARM"{print $2}' "$run_ratio_file" | tail -n 1)"
  run_snapshot_rel="${run_snapshot_files[$((run - 1))]}"
  echo -e "${date_utc}\t${WEEK}\t${git_rev}\t${run}\t${run_geomean}\t${run_warm_geomean}\t${run_snapshot_rel}\t${PIN_CORE}\t${NOTES}" >> "$run_history_file"
done

bench_file="$TRACKING_DIR/jit_weekly_benchmark_history.tsv"
if [[ ! -f "$bench_file" ]]; then
  echo -e "date_utc\tweek\tgit_rev\tbenchmark\trust_ms\tnode_ms\tpython_ms\tvm_ms\tjit_ms\tjit_node_ratio\tjit_node_warm_ratio\tjit_node_ratio_100x\tjit_node_warm_ratio_100x\ttyped_numeric_ops\tgeneric_numeric_ops\tbytecode_compile_ms\tjit_compile_ms\tjit_exec_ms" > "$bench_file"
fi
awk -F'\t' -v date="$date_utc" -v week="$WEEK" -v git="$git_rev" '
  NR>1 && $1!="__summary__" {
    print date "\t" week "\t" git "\t" $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6 "\t" $7 "\t" $8 "\t" $9 "\t" $10 "\t" $11 "\t" $12 "\t" $13 "\t" $14 "\t" $15
  }
' "$agg_metrics" >> "$bench_file"

echo "Wrote weekly snapshot:"
echo "  runs:     $RUNS"
echo "  geomean:  $geomean (range ${geomean_min}-${geomean_max})"
if [[ -n "$warm_geomean" ]]; then
  echo "  warm:     $warm_geomean (range ${warm_geomean_min}-${warm_geomean_max})"
fi
echo "  summary:  $summary_file"
echo "  run-log:  $run_history_file"
echo "  per-bench: $bench_file"
echo "  snapshot: $snapshot_rel"
