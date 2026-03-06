#!/bin/bash
# ci/bench-gate.sh — Performance regression gate for Shape VM benchmarks.
#
# Usage:
#   # On the main branch (establish baseline):
#   cargo bench --bench vm_benchmarks -- --save-baseline main
#
#   # On a PR branch:
#   ./ci/bench-gate.sh
#
# Environment variables:
#   BENCH_REGRESSION_THRESHOLD  Max allowed regression % (default: 10)
#   BENCH_SAMPLE_SIZE           Criterion sample size (default: 100)
#   BENCH_MEASUREMENT_TIME      Criterion measurement time in seconds (default: 5)
#
# Exit codes:
#   0  All benchmarks within threshold (or no baseline to compare)
#   1  Regression detected or execution failure
#
# Acceptance criteria:
#   - No benchmark regresses >THRESHOLD% from baseline with p<0.05
#   - Trusted ops (when available) must be faster than guarded (p<0.05)
#   - GC young pause p99 tracked but no assumed target until baseline established

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THRESHOLD=${BENCH_REGRESSION_THRESHOLD:-10}
SAMPLE_SIZE=${BENCH_SAMPLE_SIZE:-100}
MEASUREMENT_TIME=${BENCH_MEASUREMENT_TIME:-5}

echo "=== Shape VM Benchmark Gate ==="
echo "Regression threshold: ${THRESHOLD}%"
echo "Sample size: ${SAMPLE_SIZE}"
echo "Measurement time: ${MEASUREMENT_TIME}s"
echo ""

# Attempt fixed CPU governor (non-fatal, requires root)
cpupower frequency-set -g performance 2>/dev/null || true

# Warmup run (discard) — primes caches and compilation
echo "--- Warmup run (discarded) ---"
cargo bench --bench vm_benchmarks -- --sample-size 10 --measurement-time 2 \
  >/dev/null 2>&1 || true

# Actual measurement — compare against stored 'main' baseline if it exists
echo "--- Benchmark measurement ---"
BENCH_ARGS=(
    --sample-size "${SAMPLE_SIZE}"
    --measurement-time "${MEASUREMENT_TIME}"
    --save-baseline pr
)

# If a 'main' baseline exists, compare against it
if [ -d "target/criterion" ]; then
    BENCH_ARGS+=(--baseline main)
fi

cargo bench --bench vm_benchmarks -- "${BENCH_ARGS[@]}" 2>&1

echo ""
echo "=== Benchmark run complete ==="
echo "Reports: target/criterion/"
echo ""

# Run Python regression checker
echo "--- Checking for regressions ---"
python3 "${SCRIPT_DIR}/check_regressions.py" --threshold "${THRESHOLD}"
exit_code=$?

if [ $exit_code -ne 0 ]; then
    echo ""
    echo "FAILED: Benchmark regression detected. See above for details."
    echo ""
    echo "If this is expected, update the baseline:"
    echo "  cargo bench --bench vm_benchmarks -- --save-baseline main"
    exit 1
fi

echo ""
echo "To establish a new baseline on main:"
echo "  cargo bench --bench vm_benchmarks -- --save-baseline main"
