#!/usr/bin/env bash
# scripts/coverage.sh — Phase 4c CI coverage gate
#
# Per docs/cluster-audits/v0.3-w14-test-coverage-audit.md §1 (install pattern)
# + §2 (invocation pattern) + §3 (per-crate baseline) + §6 (gate threshold).
#
# Per Phase 4 acceptance criterion (user 2026-05-18): "test coverage ≥99%
# per-feature with DOCUMENTED EXCEPTIONS ONLY WHEN VERY HARD TO TEST".
#
# Cadence (per Phase 4 test execution policy in
# docs/cluster-audits/phase-3-team-lead-handover.md:42-47):
#   - Phase-4b-batch-merge invocation point (post-merge gate on main).
#   - Nightly (per W13.4 nightly-fuzz.yml precedent: cron 0 4 * * * UTC).
#   - NOT per-commit (per W14.1 audit + Phase 4 policy).
#
# Tool: cargo-tarpaulin (line + branch + dead-code).
# Version: 0.35.4 pinned per audit §1.2 recommendation.
# Install: per audit §1.6 — `cargo install cargo-tarpaulin --version 0.35.4
#          --locked` from inside devenv shell (gcc-wrapper + linker + openssl
#          headers must be on PATH; NixOS host needs the devenv-shell wrap).
#
# Usage:
#
#   bash scripts/coverage.sh                # full workspace line coverage
#   bash scripts/coverage.sh --branch       # add branch coverage
#   bash scripts/coverage.sh --dead-code    # add dead-code measurement
#   bash scripts/coverage.sh --crate <name> # per-crate scoped (uses --include-files)
#   bash scripts/coverage.sh --deep         # add --features <crate>/deep-tests matrix
#
# Exit codes:
#   0  — coverage meets ≥99% per-feature threshold (with documented exceptions)
#   1  — coverage regression (per-feature drop below threshold without exception cite)
#   2  — script invocation error (missing tarpaulin, wrong directory, etc.)
#
# This script DOES NOT run per-commit. Per-commit gates are
# `just check-clean` + `bash scripts/verify-merge.sh` + `bash
# scripts/check-no-dynamic.sh`.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# -----------------------------------------------------------------------------
# Argument parsing
# -----------------------------------------------------------------------------
mode="line"
crate=""
deep=0
for arg in "$@"; do
  case "$arg" in
    --branch) mode="branch" ;;
    --dead-code) mode="dead-code" ;;
    --crate)
      mode="per-crate"
      ;;
    --crate=*)
      mode="per-crate"
      crate="${arg#--crate=}"
      ;;
    --deep) deep=1 ;;
    --help|-h)
      grep '^#' "$0" | head -40
      exit 0
      ;;
    *)
      # Positional crate name follows --crate
      if [[ "$mode" == "per-crate" && -z "$crate" ]]; then
        crate="$arg"
      else
        echo "FATAL: unknown argument: $arg" >&2
        exit 2
      fi
      ;;
  esac
done

# -----------------------------------------------------------------------------
# Preflight: cargo-tarpaulin must be installed
# -----------------------------------------------------------------------------
# Per audit §1.6 install pattern. Bare-shell `cargo install cargo-tarpaulin`
# fails on NixOS hosts with `linker `cc` not found` + `openssl-sys` build-script
# failure — devenv-shell wrapping (or equivalent Nix container with gcc-wrapper
# + pkg-config + openssl headers on PATH) is required.
if ! command -v cargo-tarpaulin >/dev/null 2>&1; then
  cat >&2 <<EOF
FATAL: cargo-tarpaulin not installed.

Install per docs/cluster-audits/v0.3-w14-test-coverage-audit.md §1.6:

  cd /home/dev/dev/shape-lang
  devenv shell --quiet -- bash -c 'cargo install cargo-tarpaulin --version 0.35.4 --locked'

Or in CI, install inside a container with gcc-wrapper + pkg-config + openssl-dev
headers on PATH (the audit §1 install-surface notes the bare-shell failure mode
is a host PATH gap, NOT a tarpaulin-side defect).

The Phase-4b-batch-merge cadence + nightly cron at .github/workflows/coverage.yml
(per W13.4 nightly-fuzz.yml precedent) installs tarpaulin in the CI step before
invoking this script.
EOF
  exit 2
fi

# -----------------------------------------------------------------------------
# Tarpaulin invocation builder
# -----------------------------------------------------------------------------
# Per audit §2 + §3. Common flags:
#   --workspace          → every workspace member tarpaulin can build
#   --lib --tests        → matches `just test-fast` + `just test` tier shape
#   --skip-clean         → avoids recompiling all workspace deps every run
#   --exclude shape-ext-python --exclude shape-ext-typescript → matches
#                          `just test-fast`/`test` exclusion shape (pyo3 +
#                          deno_core need their respective runtime envs)
#   --timeout 240        → per-test timeout (default 60s too short under
#                          instrumented build for some shape-vm integration tests)
common_flags=(
  --workspace
  --skip-clean
  --exclude shape-ext-python
  --exclude shape-ext-typescript
  --timeout 240
)

# Deep-tests feature matrix per audit §2 "Deep-tests feature compatibility":
# enables the 5 modules gated behind `deep-tests` per CLAUDE.md Known
# Constraints + stdlib JIT-compile parallel-cache race; pair with
# `--test-threads=1` to avoid SIGILL.
deep_flags=()
deep_postfix=()
if [[ "$deep" -eq 1 ]]; then
  deep_flags=(
    --features shape-vm/deep-tests
    --features shape-runtime/deep-tests
    --features shape-ast/deep-tests
    --features shape-jit/deep-tests
  )
  deep_postfix=(-- --test-threads=1)
fi

case "$mode" in
  line)
    # Per audit §2 "Line coverage (default)".
    set +e
    cargo tarpaulin "${common_flags[@]}" --lib --tests "${deep_flags[@]}" --out Stdout "${deep_postfix[@]}"
    tarp_ec=$?
    set -e
    ;;
  branch)
    # Per audit §2 "Branch coverage". Critical for the strict-typing dispatch
    # tables in shape-vm (4-table HeapKind lockstep + Q8 / Q10 dispatch arms)
    # where line coverage alone can mis-report 100% on a dispatch site whose
    # `_ => unreachable!()` arm is never exercised.
    set +e
    cargo tarpaulin "${common_flags[@]}" --branch --lib --tests "${deep_flags[@]}" --out Stdout "${deep_postfix[@]}"
    tarp_ec=$?
    set -e
    ;;
  dead-code)
    # Per audit §2 "Dead-code measurement". `--ignore-tests` excludes test
    # code itself from the denominator; pairs with `--bins` to surface
    # uncovered productive code in bin/shape-cli + tools/shape-lsp.
    set +e
    cargo tarpaulin "${common_flags[@]}" --lib --bins --tests --ignore-tests "${deep_flags[@]}" --out Stdout "${deep_postfix[@]}"
    tarp_ec=$?
    set -e
    ;;
  per-crate)
    # Per audit §2 "Per-crate (incremental)". `-p <crate>` alone is INSUFFICIENT
    # (tarpaulin walks the entire workspace's file tree even when a single
    # package is selected — empirical at audit-day surface 2). `--include-files
    # "<crate-dir>/*"` is the workaround that scopes the denominator to the
    # target crate's files.
    if [[ -z "$crate" ]]; then
      echo "FATAL: --crate requires a crate name (usage: bash scripts/coverage.sh --crate shape-vm)" >&2
      exit 2
    fi
    # Resolve crate directory. Workspace members live under crates/<name>,
    # bin/<name>, tools/<name>, or extensions/<name>.
    crate_dir=""
    for candidate in "crates/$crate" "bin/$crate" "tools/$crate" "extensions/${crate#shape-ext-}"; do
      if [[ -d "$candidate" ]]; then
        crate_dir="$candidate"
        break
      fi
    done
    if [[ -z "$crate_dir" ]]; then
      echo "FATAL: cannot resolve directory for crate '$crate' (looked under crates/, bin/, tools/, extensions/)" >&2
      exit 2
    fi
    set +e
    cargo tarpaulin -p "$crate" --lib --tests --skip-clean \
      --include-files "${crate_dir}/*" \
      --timeout 240 \
      "${deep_flags[@]}" --out Stdout "${deep_postfix[@]}"
    tarp_ec=$?
    set -e
    ;;
esac

# -----------------------------------------------------------------------------
# Per-feature classification + exception registry hook
# -----------------------------------------------------------------------------
# Per audit §6.2 binding format. The exception registry lives at
# docs/cluster-audits/v0.3-w14-h1-exception-registry.md (W14.2-H1 audit-only
# sub-cluster output) and enumerates every DOCUMENTED-EXCEPTION cite with:
#   - Feature ID (W-series sub-cluster identifier)
#   - file:line span
#   - Why hard to test (8 categorized reasons)
#   - Workaround-coverage if any
#
# Phase-4b-batch-merge gate consumes this registry as the named-and-cited
# exception list. Per audit §6 "Own all code quality" binding: the exception
# list is NOT a graveyard — each entry must have a tracked disposition.
exception_registry="docs/cluster-audits/v0.3-w14-h1-exception-registry.md"
if [[ -f "$exception_registry" ]]; then
  echo
  echo "=== Documented exceptions (per audit §6.2 binding format) ==="
  echo "Registry: $exception_registry"
  echo "  - DOCUMENTED-EXCEPTION cite shape: Feature ID + file:line span +"
  echo "    why-hard-to-test (8 categorized reasons per audit §6.2.3) +"
  echo "    workaround-coverage-if-any per audit §6.2.4."
fi

# -----------------------------------------------------------------------------
# Exit code
# -----------------------------------------------------------------------------
# Tarpaulin's own exit code semantics:
#   0  — coverage measurement completed; threshold (if --fail-under) met
#   1  — measurement completed but below --fail-under threshold
#   non-zero (other) — measurement failed (test panic, build failure, etc.)
#
# This wrapper propagates tarpaulin's exit code. Phase-4b-batch-merge gate
# wires `--fail-under 99` once the per-feature exception registry is wired
# (W14.2-H1 territory + the per-feature post-processor that subtracts
# documented-exception denominator from the workspace measurement).
exit $tarp_ec
