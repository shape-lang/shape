#!/usr/bin/env bash
# ADR-020 / #239 native-execution acceptance gate.
#
# WHY THIS EXISTS
#
# The corpus differential compares VM output against JIT output. That gate is
# only meaningful while the native tier actually runs. After #257 deleted the
# fabricated `Int64` destination default, programs whose slot kinds are unproven
# whole-program-bail and execute interpreted — so `--mode jit` and `--mode vm`
# run the SAME interpreter and agree trivially.
#
# Measured 2026-08-01: native-dispatch rate fell from 121/481 corpus programs to
# 11/482. Five known-red entries flipped to MATCH — and all five were verified to
# have `program_fallback: jit-compile-error` with ZERO native dispatches. They
# match because the JIT never runs. That is the defect being MASKED, not fixed,
# and it is the same shape as #224's original finding ("whole program previously
# bailed via unit-returning fn main, so VM==JIT trivially") which is what
# unmasked #231 and #232 in the first place.
#
# So VM==JIT agreement is currently nearly free and proves almost nothing about
# the value channel. This gate asserts the property the differential can no
# longer see: that the listed programs execute NATIVELY.
#
# NOTE the two metrics are not interchangeable. `program_fallback == null` and
# `sum(native_dispatches) > 0` differ by an order of magnitude on the same
# sample (16.7% vs 0%). Absence of a program-level bail does NOT mean native
# code ran — a per-function bail leaves `program_fallback` null. This gate
# requires non-zero DISPATCHES.
#
# EXPECTED STATE
#
# Until #239's conversion lands this gate FAILS for every row, by design. It is
# the conversion's acceptance criterion, not a merge blocker for the steps
# before it. A row going from 0 dispatches to non-zero is the only unambiguous
# signal that the channel was converted rather than renamed — "the deopt message
# is gone" is not, because a message can be deleted without the path changing.
#
# Usage:  bash scripts/check-jit-native-acceptance.sh [--report-only]
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${SHAPE_BIN:-$ROOT/target/release/shape}"
CORPUS="$ROOT/tools/vmjit-diff/corpus"
REPORT_ONLY=0
[[ "${1:-}" == "--report-only" ]] && REPORT_ONLY=1

if [[ ! -x "$BIN" ]]; then
  echo "error: no shape binary at $BIN"
  echo "  build first: direnv exec <workspace> cargo build --release --bin shape --jobs 4"
  echo "  (a stale binary lies in the MASKING direction — rebuild before believing this gate)"
  exit 2
fi

# The acceptance set: fixtures that MUST execute natively once the channel
# carries raw typed values. Each names the deopt class it retires.
FIXTURES=(
  "SYN__datetime-method-native.shape|STAGE-F3 VM-only typed-Arc receivers (whole-PROGRAM deopt today)"
  "SYN__string-scalar-method-native.shape|STAGE-StringJIT scalar-returning string methods"
  "SYN__closure-calls-closure-guarded.shape|closure-calls-closure, correct via bail today"
  "SYN__callvalue-stop-returns-value.shape|#259 stop path; must stop WITHOUT a value AND run native"
  "SYN__closure-l106.shape|L106 baseline — the simplest closure call there is"
)

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
printf '%-46s %10s %10s  %s\n' "FIXTURE" "DISPATCH" "FALLBACK" "RETIRES"
printf '%s\n' "------------------------------------------------------------------------------------------"
for row in "${FIXTURES[@]}"; do
  id="${row%%|*}"
  why="${row#*|}"
  path="$CORPUS/$id"
  if [[ ! -f "$path" ]]; then
    printf '%-46s %10s %10s  %s\n' "$id" "MISSING" "-" "$why"
    fail=1
    continue
  fi
  timeout 30 "$BIN" run --mode jit --native-witness "$tmp/w.json" "$path" >/dev/null 2>&1
  read -r nd fb < <(python3 - "$tmp/w.json" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    print("-1 nowitness"); raise SystemExit
nd = sum(f.get("native_dispatches", 0) for f in d.get("functions", []))
pf = d.get("program_fallback")
print(f'{nd} {(pf or {}).get("reason_class", "none")}')
PY
)
  rm -f "$tmp/w.json"
  status_ok=0
  [[ "$nd" =~ ^[0-9]+$ ]] && (( nd > 0 )) && status_ok=1
  (( status_ok )) || fail=1
  printf '%-46s %10s %10s  %s\n' "$id" "$nd" "$fb" "$why"
done

echo
if (( fail )); then
  echo "NATIVE-ACCEPTANCE: NOT MET — one or more fixtures execute zero native dispatches."
  echo "  Before the #239 conversion this is EXPECTED. After it, this is the gate failing."
  echo "  A VM==JIT match on these programs is NOT evidence: with the JIT inert both"
  echo "  tiers run the same interpreter (see the header of this script)."
  (( REPORT_ONLY )) && exit 0
  exit 1
fi
echo "NATIVE-ACCEPTANCE: MET — every acceptance fixture executes native code."
exit 0
