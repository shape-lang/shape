#!/usr/bin/env bash
# Process-TREE RSS profiler. Same idea as rss-profile.sh but sums RSS across a
# whole process tree, which is what `cargo test` actually is: cargo + N rustc +
# a linker + the test binaries. Use this to attribute peak memory between the
# BUILD phase and the RUN phase.
#
# usage: rss-tree-profile.sh <outdir> <command...>
# env:   RSS_CAP_KB (default 12582912 = 12 GiB), SAMPLE_INTERVAL (default 1)
#
# outputs: <outdir>/tree.tsv (elapsed_s, n_procs, tree_rss_kb, top_proc)

set -uo pipefail

OUTDIR="$1"; shift
RSS_CAP_KB="${RSS_CAP_KB:-12582912}"
SAMPLE_INTERVAL="${SAMPLE_INTERVAL:-1}"

mkdir -p "$OUTDIR"
: > "$OUTDIR/cmdlog.txt"

setsid "$@" > "$OUTDIR/cmdlog.txt" 2>&1 &
ROOT=$!
PGID="$ROOT"

printf 'elapsed_s\tn_procs\ttree_rss_kb\ttop_proc\ttop_rss_kb\n' > "$OUTDIR/tree.tsv"
START="$(date +%s.%N)"
KILLED=0

while kill -0 "$ROOT" 2>/dev/null; do
  read -r N TOTAL TOPC TOPR <<<"$(ps -eo pgid=,rss=,comm= --no-headers 2>/dev/null \
    | awk -v g="$PGID" '$1==g { n++; t+=$2; if ($2>m) { m=$2; c=$3 } } END { printf "%d %d %s %d", n+0, t+0, (c==""?"-":c), m+0 }')"
  NOW="$(date +%s.%N)"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$(awk -v a="$NOW" -v b="$START" 'BEGIN{printf "%.1f", a-b}')" \
    "$N" "$TOTAL" "$TOPC" "$TOPR" >> "$OUTDIR/tree.tsv"
  if [ "${TOTAL:-0}" -gt "$RSS_CAP_KB" ]; then
    echo "rss-tree: tree RSS ${TOTAL} KB exceeded cap ${RSS_CAP_KB} KB — killing pgid $PGID" >&2
    kill -9 -"$PGID" 2>/dev/null
    KILLED=1
    break
  fi
  sleep "$SAMPLE_INTERVAL"
done

wait "$ROOT" 2>/dev/null
PEAK="$(awk -F'\t' 'NR>1 && $3>m {m=$3} END{print m+0}' "$OUTDIR/tree.tsv")"
PEAKP="$(awk -F'\t' 'NR>1 && $5>m {m=$5; c=$4} END{print c"/"m+0}' "$OUTDIR/tree.tsv")"
echo "rss-tree: done killed=$KILLED peak_tree_rss_kb=$PEAK peak_single=$PEAKP outdir=$OUTDIR" >&2
exit $KILLED
