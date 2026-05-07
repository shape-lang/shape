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
