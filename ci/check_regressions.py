#!/usr/bin/env python3
"""Parse Criterion benchmark JSON output and flag regressions.

Usage:
    python3 ci/check_regressions.py [--threshold 10] [--criterion-dir target/criterion]

Scans Criterion's output directory for benchmark estimates (new/estimates.json)
and, if a baseline comparison exists (change/estimates.json), checks whether
any benchmark regressed beyond the configured threshold with p < 0.05.

Exit codes:
    0  No regressions detected (or no baseline to compare against)
    1  At least one regression exceeds the threshold
"""

import argparse
import json
import os
import sys
from pathlib import Path


def find_benchmark_dirs(criterion_dir: Path):
    """Yield (name, bench_dir) for each benchmark that has estimates."""
    for root, dirs, files in os.walk(criterion_dir):
        root_path = Path(root)
        if root_path.name == "new" and "estimates.json" in files:
            bench_dir = root_path.parent
            # Derive a human-readable name from the path
            rel = bench_dir.relative_to(criterion_dir)
            yield str(rel), bench_dir


def load_json(path: Path):
    """Load a JSON file, returning None if it does not exist."""
    if not path.exists():
        return None
    with open(path) as f:
        return json.load(f)


def check_regressions(criterion_dir: Path, threshold_pct: float):
    """Check all benchmarks for regressions. Returns list of failures."""
    failures = []
    checked = 0

    for name, bench_dir in find_benchmark_dirs(criterion_dir):
        change_path = bench_dir / "change" / "estimates.json"
        change = load_json(change_path)
        if change is None:
            # No baseline comparison available — skip
            continue

        checked += 1

        # Criterion stores change as a fraction (0.05 = 5% regression)
        # The "mean" field has point_estimate, standard_error, confidence_interval
        mean_change = change.get("mean", {})
        point_estimate = mean_change.get("point_estimate", 0.0)
        ci = mean_change.get("confidence_interval", {})
        ci_lower = ci.get("lower_bound", 0.0)
        ci_upper = ci.get("upper_bound", 0.0)

        # Convert fraction to percentage
        change_pct = point_estimate * 100.0
        ci_lower_pct = ci_lower * 100.0
        ci_upper_pct = ci_upper * 100.0

        # A regression means positive change (slower).
        # We flag if the lower bound of the 95% CI is above the threshold
        # (i.e., we are 95%+ confident the regression exceeds the threshold).
        # For a stricter check: flag if point estimate exceeds threshold
        # and lower bound is positive (p < 0.05 that it's a regression at all).
        is_regression = change_pct > threshold_pct and ci_lower_pct > 0.0

        status = "REGRESSION" if is_regression else "ok"
        print(
            f"  {status:11s}  {name:50s}  "
            f"{change_pct:+7.2f}% [{ci_lower_pct:+.2f}%, {ci_upper_pct:+.2f}%]"
        )

        if is_regression:
            failures.append(
                f"{name}: {change_pct:+.2f}% "
                f"(CI: [{ci_lower_pct:+.2f}%, {ci_upper_pct:+.2f}%])"
            )

    return failures, checked


def main():
    parser = argparse.ArgumentParser(
        description="Check Criterion benchmarks for regressions."
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=float(os.environ.get("BENCH_REGRESSION_THRESHOLD", "10")),
        help="Maximum allowed regression percentage (default: 10)",
    )
    parser.add_argument(
        "--criterion-dir",
        type=Path,
        default=Path("target/criterion"),
        help="Path to Criterion output directory",
    )
    args = parser.parse_args()

    criterion_dir = args.criterion_dir
    if not criterion_dir.exists():
        print(f"Criterion directory not found: {criterion_dir}")
        print("Run benchmarks first: cargo bench --bench vm_benchmarks")
        sys.exit(0)

    print(f"=== Regression Check (threshold: {args.threshold}%) ===")
    print()

    failures, checked = check_regressions(criterion_dir, args.threshold)

    print()
    if checked == 0:
        print("No baseline comparisons found. Run benchmarks on main first:")
        print("  cargo bench --bench vm_benchmarks -- --save-baseline main")
        sys.exit(0)

    print(f"Checked {checked} benchmark(s) against baseline.")

    if failures:
        print()
        print(f"FAILED: {len(failures)} benchmark(s) regressed beyond {args.threshold}%:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    else:
        print("All benchmarks within threshold.")
        sys.exit(0)


if __name__ == "__main__":
    main()
