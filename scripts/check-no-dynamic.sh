#!/usr/bin/env bash
# Strict-typing plan defection guard. See ~/.claude/plans/stop-native-vs-tagged-tax.md
# and the "Forbidden Patterns" section of CLAUDE.md.
#
# Per-symbol monotonic-non-increasing check against a frozen baseline. A symbol's
# count may decrease (deletion progress); it may not increase (regression). Once
# a symbol's baseline reaches 0 it stays at 0 forever. Phases 2-4 of the plan
# walk these counts down to 0; the recipe's job is to keep agents from sneaking
# any count back up.
#
# Scope: source trees only (crates/, bin/, tools/, extensions/). Documentation
# trees (docs/, CLAUDE.md, plans/) intentionally NOT scanned — they discuss the
# forbidden patterns by name as part of the enforcement contract.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

baseline="docs/check-no-dynamic-baseline.txt"
scope=(crates bin tools extensions)

if [[ ! -f "$baseline" ]]; then
  echo "FATAL: baseline file $baseline missing" >&2
  exit 2
fi

count_one() {
  # rg returns exit 1 when there are zero matches; that's not an error here.
  { rg --no-heading -c -P "$1" "${scope[@]}" 2>/dev/null || true; } \
    | awk -F: '{s+=$2} END {print s+0}'
}

fail=0
progress=0
while IFS=$'\t' read -r limit pattern note; do
  [[ -z "${limit:-}" || "$limit" == \#* ]] && continue
  actual=$(count_one "$pattern")
  if (( actual > limit )); then
    printf 'FAIL  %-50s baseline=%-3d actual=%-3d  (regression: +%d)\n' \
      "$note" "$limit" "$actual" "$((actual - limit))"
    fail=1
  elif (( actual < limit )); then
    printf 'OK    %-50s baseline=%-3d actual=%-3d  (progress: -%d — update baseline)\n' \
      "$note" "$limit" "$actual" "$((limit - actual))"
    progress=1
  fi
done < "$baseline"

# ─────────────────────────────────────────────────────────────────────────────
# ADR-009 ticket C1 — K1: ONE CAPTURE SELECTOR.
#
# `CaptureKind::{Immutable,OwnedMutable,Shared}` may be named in EXACTLY ONE
# bytecode-compiler file: comptime_builtins/capture_plan.rs. Every other site
# reads the plan (`CapturePlan::kind()` / `CapturePlan::access()`) or the pack.
#
# Why this is a build failure and not a review norm: closure-capture emission
# used to be driven by two coupled vectors (`mutable_flags` + `capture_kinds`).
# A second producer is exactly how a DECLARED capture mode gets validated and
# then discarded while inference stays authoritative — the defect that got C1
# rejected once already, and the same shape as the ValueWord walk-back this
# script exists to prevent. One selector, or the build stops.
capture_kind_offenders=$(
  rg --no-heading -l -P 'CaptureKind::(Immutable|OwnedMutable|Shared)\b' \
    crates/shape-vm/src/compiler 2>/dev/null | grep -v 'capture_plan\.rs' || true
)
if [[ -n "$capture_kind_offenders" ]]; then
  echo "FAIL  ADR-009 C1 K1: a SECOND CaptureKind producer exists."
  echo "      Only crates/shape-vm/src/compiler/comptime_builtins/capture_plan.rs may"
  echo "      name a CaptureKind variant. Offending files:"
  while IFS= read -r f; do echo "        $f"; done <<< "$capture_kind_offenders"
  fail=1
fi

# ─────────────────────────────────────────────────────────────────────────────
# ADR-009 ticket C1 (slice 2) — K1b: ONE MINT FOR NODE-BORNE PROVENANCE.
#
# `GeneratedNodeOrigin` is the Wave-46 capture gate's predicate: a closure node
# carrying one IS generated code. Its constructor may be called in EXACTLY ONE
# compiler file — comptime_builtins/expansion_provenance.rs — where it is
# projected from a REGISTERED `GeneratedOrigin` (whose `SymbolId` derive is
# private to that module, ProofGap-style). Anywhere else, emit code could
# fabricate a stamp from a name or a span and re-open the identity hole that got
# C1 rejected (a span-keyed pack table + `DeclaredCapture { name: String }`).
# The AST-side `impl` block in shape-ast is the definition, not a producer.
node_origin_offenders=$(
  rg --no-heading -l -P 'GeneratedNodeOrigin::new\b' \
    crates/shape-vm/src crates/shape-runtime/src tools 2>/dev/null \
    | grep -v 'expansion_provenance\.rs' || true
)
if [[ -n "$node_origin_offenders" ]]; then
  echo "FAIL  ADR-009 C1 K1b: a SECOND GeneratedNodeOrigin mint exists."
  echo "      Only crates/shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs"
  echo "      may mint a node stamp (from a registered GeneratedOrigin). Offending files:"
  while IFS= read -r f; do echo "        $f"; done <<< "$node_origin_offenders"
  fail=1
fi

if (( fail )); then
  echo
  echo "Forbidden symbols regressed. See CLAUDE.md 'Forbidden Patterns' and the strict-typing plan."
  exit 1
fi

if (( progress )); then
  echo
  echo "Counts decreased — edit $baseline to record the new lower bound."
fi
exit 0
