#!/usr/bin/env bash
# Extract and compare Rust libtest failure lists from saved cargo logs.
#
# Usage:
#   scripts/compare_cargo_failures.sh \
#     --log current=/tmp/current.log \
#     --log base=/tmp/base.log \
#     --compare current:base \
#     --write-dir /tmp/failure-sets
#
# The parser reads plain `cargo test` output, extracts the final `failures:`
# name list plus the final `test result:` summary, and optionally writes one
# sorted `<label>.failures` file per log. Comparisons report names that are only
# present on the left or right side of a `--compare LEFT:RIGHT` pair. With
# `--write-dir`, comparison-only lists are also saved as
# `<left>_only_vs_<right>.failures`.

set -euo pipefail

usage() {
  sed -n '2,14p' "$0" >&2
}

declare -a labels=()
declare -a compares=()
declare -A paths=()
declare -A summaries=()
declare -A failure_files=()
write_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --log)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      raw="$2"
      [[ "$raw" == *=* ]] || { echo "--log entries must be LABEL=PATH" >&2; exit 2; }
      label="${raw%%=*}"
      path="${raw#*=}"
      [[ -n "$label" ]] || { echo "log label cannot be empty" >&2; exit 2; }
      [[ -f "$path" ]] || { echo "log path does not exist: $path" >&2; exit 2; }
      if [[ -v "paths[$label]" ]]; then
        echo "duplicate --log label: $label" >&2
        exit 2
      fi
      labels+=("$label")
      paths["$label"]="$path"
      shift 2
      ;;
    --compare)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      raw="$2"
      [[ "$raw" == *:* ]] || { echo "--compare entries must be LEFT:RIGHT" >&2; exit 2; }
      compares+=("$raw")
      shift 2
      ;;
    --write-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      write_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

[[ ${#labels[@]} -gt 0 ]] || { echo "at least one --log LABEL=PATH entry is required" >&2; exit 2; }

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

if [[ -n "$write_dir" ]]; then
  mkdir -p "$write_dir"
fi

for label in "${labels[@]}"; do
  path="${paths[$label]}"
  summary="$(awk '/^test result:/ { line = $0 } END { print line }' "$path")"
  summaries["$label"]="$summary"

  output="$tmp_dir/$label.failures"
  awk '
    $0 == "failures:" {
      delete names
      count = 0
      in_block = 1
      next
    }
    in_block && /^$/ {
      next
    }
    in_block && /^    / {
      name = $0
      sub(/^    /, "", name)
      names[++count] = name
      next
    }
    in_block {
      in_block = 0
      next
    }
    END {
      for (idx = 1; idx <= count; idx++) {
        print names[idx]
      }
    }
  ' "$path" | sort -u > "$output"

  if [[ -n "$write_dir" ]]; then
    cp "$output" "$write_dir/$label.failures"
    failure_files["$label"]="$write_dir/$label.failures"
  else
    failure_files["$label"]="$output"
  fi
done

for label in "${labels[@]}"; do
  echo "[$label] ${paths[$label]}"
  echo "summary: ${summaries[$label]:-<missing>}"
  echo "failures: $(wc -l < "${failure_files[$label]}")"
  if [[ -n "$write_dir" ]]; then
    echo "failure_list: ${failure_files[$label]}"
  fi
  echo
done

for compare in "${compares[@]}"; do
  left="${compare%%:*}"
  right="${compare#*:}"
  [[ -v "failure_files[$left]" ]] || { echo "unknown comparison label: $left" >&2; exit 2; }
  [[ -v "failure_files[$right]" ]] || { echo "unknown comparison label: $right" >&2; exit 2; }

  left_only="$tmp_dir/$left-only-$right.failures"
  right_only="$tmp_dir/$right-only-$left.failures"
  comm -23 "${failure_files[$left]}" "${failure_files[$right]}" > "$left_only"
  comm -13 "${failure_files[$left]}" "${failure_files[$right]}" > "$right_only"

  if [[ -n "$write_dir" ]]; then
    cp "$left_only" "$write_dir/${left}_only_vs_${right}.failures"
    cp "$right_only" "$write_dir/${right}_only_vs_${left}.failures"
  fi

  echo "[compare $left:$right]"
  echo "${left}_only: $(wc -l < "$left_only")"
  if [[ -n "$write_dir" ]]; then
    echo "${left}_only_list: $write_dir/${left}_only_vs_${right}.failures"
  fi
  sed 's/^/  /' "$left_only"
  echo "${right}_only: $(wc -l < "$right_only")"
  if [[ -n "$write_dir" ]]; then
    echo "${right}_only_list: $write_dir/${right}_only_vs_${left}.failures"
  fi
  sed 's/^/  /' "$right_only"
  echo
done
