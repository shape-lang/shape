#!/usr/bin/env bash
# Bounded-RSS tripwire (#206).
#
# Two guards, both cheap enough for CI:
#
#   RUN-PHASE  Executes a fixed test slice in ONE process and asserts both the
#              peak RSS and the per-test RSS slope stay under declared bounds.
#              This is the guard against genuine per-test resource accumulation
#              (leaked JIT pages, uncollected Arc cycles, unbounded interners) —
#              if any is introduced, this fails loudly instead of at the next OOM.
#
#   BUILD-PHASE  Asserts the build-parallelism bound is still configured.
#              Peak build RSS is ~1.86 GB * jobs on this workspace (~60 test
#              binaries, each a workspace-sized static link). Unbounded jobs is
#              what actually OOM'd the container on 2026-07-28; the run phase
#              was never the problem.
#
# Bounds are declared here and justified in
# docs/program/test-infra/accumulation-diagnosis.md.
#
# usage: rss-tripwire.sh            # build slice if needed, then check
#        SKIP_BUILD=1 rss-tripwire.sh

set -uo pipefail
cd "$(dirname "$0")/.."

# ---- declared bounds -------------------------------------------------------
# Measured 2026-07-28 at 38345e03: peak 113 MB, steady-state slope ~8.5 KB/test
# over 3583 tests. Bounds are ~2-3x measured, to absorb allocator and libtest
# noise without letting a real regression through.
PEAK_BOUND_KB="${PEAK_BOUND_KB:-262144}"      # 256 MB
SLOPE_BOUND_B="${SLOPE_BOUND_B:-24576}"       # 24 KB per test
MAX_BUILD_JOBS="${MAX_BUILD_JOBS:-12}"        # => peak build RSS <= ~22 GB

OUT="${OUT:-target/rss-tripwire}"
FAIL=0

say() { printf '%s\n' "$*"; }

# ---- guard 1: build-parallelism bound --------------------------------------
JOBS="$(awk -F'=' '/^[[:space:]]*jobs[[:space:]]*=/ {gsub(/[^0-9]/,"",$2); print $2; exit}' \
  .cargo/config.toml 2>/dev/null)"

if [ -z "$JOBS" ]; then
  say "FAIL [build] .cargo/config.toml has no [build] jobs bound."
  say "     Unbounded jobs => peak build RSS ~1.86 GB * nproc (~60 GB at nproc=32)."
  FAIL=1
elif [ "$JOBS" -gt "$MAX_BUILD_JOBS" ]; then
  say "FAIL [build] jobs = $JOBS exceeds the bound of $MAX_BUILD_JOBS"
  say "     (projected peak build RSS ~$((JOBS * 186 / 100)) GB)."
  FAIL=1
else
  say "ok   [build] jobs = $JOBS  (projected peak build RSS ~$((JOBS * 186 / 100)) GB)"
fi

# ---- guard 2: run-phase peak + slope ---------------------------------------
BIN=""
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  cargo test -p shape-vm --lib --no-run >/dev/null 2>&1
fi
BIN="$(cargo test -p shape-vm --lib --no-run --message-format=json 2>/dev/null \
  | python3 -c '
import json,sys
for l in sys.stdin:
    try: m=json.loads(l)
    except Exception: continue
    if m.get("profile",{}).get("test") and m.get("executable"):
        print(m["executable"]); break
')"

if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  say "FAIL [run] could not locate the shape-vm lib test binary"
  exit 1
fi

# --test-threads=1 is deliberate: it maximises the lifetime of a single process
# over the whole slice, which is exactly the accumulation scenario. It is safe
# here because this slice is measured and bounded.
SAMPLE_INTERVAL=0.5 RSS_CAP_KB="$((PEAK_BOUND_KB * 8))" \
  bash scripts/rss-profile.sh "$OUT" "$BIN" --test-threads=1 >/dev/null 2>&1

# Slope is a least-squares fit over the STEADY-STATE region only. The first few
# hundred tests are process startup (binary paging in, lazily-built registries)
# and are a one-time cost, not accumulation; including them roughly triples the
# apparent slope and makes the tripwire flap.
WARMUP="${WARMUP:-300}"
read -r PEAK SLOPE NTESTS <<<"$(awk -F'\t' -v w="$WARMUP" '
  NR>1 && $3>0 {
    if ($3>peak) peak=$3
    lastn=$2
    if ($2>=w) { n++; sx+=$2; sy+=$3; sxx+=$2*$2; sxy+=$2*$3 }
  }
  END {
    d = n*sxx - sx*sx
    slope = (n>1 && d!=0) ? (n*sxy - sx*sy) / d * 1024 : 0
    printf "%d %.0f %d", peak+0, slope, lastn+0
  }' "$OUT/samples.tsv")"

say "     [run] $NTESTS tests, peak ${PEAK} KB ($((PEAK / 1024)) MB), slope ${SLOPE} B/test"

if [ "$NTESTS" -lt 3000 ]; then
  say "FAIL [run] only $NTESTS tests executed; slice is not representative"
  FAIL=1
fi
if [ "$PEAK" -gt "$PEAK_BOUND_KB" ]; then
  say "FAIL [run] peak RSS ${PEAK} KB exceeds bound ${PEAK_BOUND_KB} KB"
  FAIL=1
else
  say "ok   [run] peak RSS within bound ($((PEAK / 1024)) MB <= $((PEAK_BOUND_KB / 1024)) MB)"
fi
if [ "${SLOPE%.*}" -gt "$SLOPE_BOUND_B" ]; then
  say "FAIL [run] per-test RSS slope ${SLOPE} B/test exceeds bound ${SLOPE_BOUND_B} B/test"
  FAIL=1
else
  say "ok   [run] per-test slope within bound (${SLOPE} <= ${SLOPE_BOUND_B} B/test)"
fi

[ "$FAIL" -eq 0 ] && say "rss-tripwire: PASS" || say "rss-tripwire: FAIL"
exit "$FAIL"
