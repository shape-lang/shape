#!/usr/bin/env bash
# RSS growth profiler for a libtest binary.
#
# Runs a test binary under a sidecar that samples the test process's RSS and,
# at each sample, the number of tests completed so far. The joined series
# (tests_done, rss_kb) is the per-test retention profile. The sidecar kills the
# run if RSS crosses a hard cap, so a memory investigation can never itself OOM
# the machine.
#
# usage: rss-profile.sh <outdir> <test-binary> [args...]
# env:   RSS_CAP_KB (default 8388608 = 8 GiB), SAMPLE_INTERVAL (default 0.2)
#
# outputs: <outdir>/samples.tsv  (elapsed_s, tests_done, rss_kb)
#          <outdir>/testlog.txt  (raw test output)

set -uo pipefail

OUTDIR="$1"; shift
BIN="$1"; shift

RSS_CAP_KB="${RSS_CAP_KB:-8388608}"
SAMPLE_INTERVAL="${SAMPLE_INTERVAL:-0.2}"

mkdir -p "$OUTDIR"
: > "$OUTDIR/testlog.txt"

# stdbuf execs the binary in place, so $! is the test process itself.
stdbuf -oL -eL "$BIN" "$@" > "$OUTDIR/testlog.txt" 2>&1 &
PIPE_PID=$!
PID="$PIPE_PID"

echo "rss-profile: sampling pid $PID (cap ${RSS_CAP_KB} KB, interval ${SAMPLE_INTERVAL}s)" >&2
printf 'elapsed_s\ttests_done\trss_kb\n' > "$OUTDIR/samples.tsv"

START="$(date +%s.%N)"
KILLED=0
while kill -0 "$PID" 2>/dev/null; do
  RSS="$(awk '/^VmRSS:/ {print $2}' "/proc/$PID/status" 2>/dev/null)"
  if [ -n "$RSS" ]; then
    DONE="$(grep -c ' \.\.\. ' "$OUTDIR/testlog.txt" 2>/dev/null)"
    DONE="${DONE:-0}"
    NOW="$(date +%s.%N)"
    printf '%s\t%s\t%s\n' \
      "$(awk -v a="$NOW" -v b="$START" 'BEGIN{printf "%.2f", a-b}')" \
      "$DONE" "$RSS" >> "$OUTDIR/samples.tsv"
    if [ "$RSS" -gt "$RSS_CAP_KB" ]; then
      echo "rss-profile: RSS ${RSS} KB exceeded cap ${RSS_CAP_KB} KB — killing $PID" >&2
      kill -9 "$PID" 2>/dev/null
      KILLED=1
      break
    fi
  fi
  sleep "$SAMPLE_INTERVAL"
done

wait $PIPE_PID
PEAK="$(awk -F'\t' 'NR>1 && $3>m {m=$3} END{print m+0}' "$OUTDIR/samples.tsv")"
NTESTS="$(awk -F'\t' 'NR>1 {n=$2} END{print n+0}' "$OUTDIR/samples.tsv")"
echo "rss-profile: done killed=$KILLED tests=$NTESTS peak_rss_kb=$PEAK outdir=$OUTDIR" >&2
exit $KILLED
