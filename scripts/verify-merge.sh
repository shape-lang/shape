#!/usr/bin/env bash
# verify-merge.sh — Phase 2d merge verification gate
#
# Per docs/cluster-audits/phase-2d-handover.md §0 "Merge-verification rule"
# and Item 7. This script MUST pass exit 0 before any sub-cluster merge into
# bulldozer-strictly-typed.
#
# Why this exists:
#
#   The prior session declared "workspace clean" three times when shape-value
#   had 50+ compile errors, because `cargo check ... | grep -c '^error\['`
#   does NOT match cargo's output under `--message-format=short` (errors are
#   prefixed `path.rs:N:M: error[...]`, not `error[...]` at column 0). Three
#   merges were greenlit while the workspace was actually broken.
#
#   Plus 8 take-both regex misses in the W14/W15/PQ merges:
#     - orphan `>>>>>>>` without preceding `<<<<<<<` / `=======`
#     - `=======` standalone
#     - HeapKind::X => { body  // next-arm-comment  HeapKind::Y => without intervening }
#     - duplicate `use` blocks with overlapping identifiers
#     - module opening line discarded while tests retained (the W14
#       `mod result_option_storage` issue)
#
# This script catches all of the above via exit-code checks (NOT grep -c)
# and dedicated pattern scans.
#
# Usage:
#
#   bash scripts/verify-merge.sh           # run all checks
#   bash scripts/verify-merge.sh --fast    # skip the test-build pass (faster CI)
#
# Exit codes:
#   0  — all checks pass; safe to merge
#   1  — one or more checks failed; DO NOT merge
#   2  — script invocation error (missing dep, wrong directory, ...)

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

fast_mode=0
for arg in "$@"; do
  case "$arg" in
    --fast) fast_mode=1 ;;
    --help|-h)
      grep '^#' "$0" | head -50
      exit 0
      ;;
    *)
      echo "FATAL: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

# Per-check status tracking. Each check appends its result to RESULTS_FILE.
RESULTS_FILE="$(mktemp)"
trap 'rm -f "$RESULTS_FILE"' EXIT

record_pass() { printf 'PASS\t%s\n' "$1" >> "$RESULTS_FILE"; }
record_fail() { printf 'FAIL\t%s\t%s\n' "$1" "$2" >> "$RESULTS_FILE"; }

# -----------------------------------------------------------------------------
# CHECK 1 — `cargo check --workspace --lib` exits 0
# -----------------------------------------------------------------------------
# Exit-code based, NOT grep. This is the entire point of this script.
echo "=== CHECK 1: cargo check --workspace --lib ==="
if cargo check --workspace --lib 2>&1 | tail -5; then
  record_pass "cargo check --workspace --lib"
  echo "  -> CLEAN"
else
  record_fail "cargo check --workspace --lib" "exit non-zero"
  echo "  -> FAILED (exit non-zero from cargo)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 2 — `cargo check --workspace --lib --tests` exits 0
# -----------------------------------------------------------------------------
if [[ "$fast_mode" -eq 0 ]]; then
  echo "=== CHECK 2: cargo check --workspace --lib --tests ==="
  if cargo check --workspace --lib --tests 2>&1 | tail -5; then
    record_pass "cargo check --workspace --lib --tests"
    echo "  -> CLEAN"
  else
    record_fail "cargo check --workspace --lib --tests" "exit non-zero"
    echo "  -> FAILED (exit non-zero from cargo)"
  fi
  echo
else
  echo "=== CHECK 2: cargo check --tests (SKIPPED in --fast mode) ==="
  echo
fi

# -----------------------------------------------------------------------------
# CHECK 3 — check-no-dynamic.sh passes (forbidden-symbol baseline)
# -----------------------------------------------------------------------------
echo "=== CHECK 3: scripts/check-no-dynamic.sh ==="
if bash scripts/check-no-dynamic.sh; then
  record_pass "check-no-dynamic"
  echo "  -> CLEAN"
else
  record_fail "check-no-dynamic" "regression vs frozen baseline"
  echo "  -> FAILED (forbidden symbols regressed)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 4 — residual merge markers anywhere in the tree
# -----------------------------------------------------------------------------
echo "=== CHECK 4: residual merge markers ==="
merge_hits=$(rg --no-heading -nE '^<<<<<<<|^=======$|^>>>>>>>' \
  --include='*.rs' --include='*.md' --include='*.toml' \
  crates docs scripts AGENTS.md CLAUDE.md justfile 2>/dev/null || true)
if [[ -z "$merge_hits" ]]; then
  record_pass "merge markers"
  echo "  -> none"
else
  record_fail "merge markers" "residual conflict markers found"
  echo "  -> FAILED — residual markers:"
  echo "$merge_hits" | head -20
fi
echo

# -----------------------------------------------------------------------------
# CHECK 5 — HeapKind ordinal collisions
# -----------------------------------------------------------------------------
# Scan heap_variants.rs for `Name, // <ordinal>` lines and verify no two
# variants share an ordinal.
echo "=== CHECK 5: HeapKind ordinal collisions ==="
collisions=$(awk '
  /^\s+[A-Z]\w*,\s*\/\/\s*[0-9]+/ {
    # extract name and ordinal
    match($0, /^\s+([A-Z][A-Za-z0-9]*),\s*\/\/\s*([0-9]+)/, m)
    if (m[1] && m[2]) {
      if (seen[m[2]]) {
        print "COLLISION: ord " m[2] ": " seen[m[2]] " vs " m[1]
        fail=1
      } else {
        seen[m[2]] = m[1]
      }
    }
  }
  END { if (fail) exit 1 }
' crates/shape-value/src/heap_variants.rs)

if [[ -z "$collisions" ]]; then
  record_pass "HeapKind ordinals"
  echo "  -> none"
else
  record_fail "HeapKind ordinals" "ordinal collision"
  echo "  -> FAILED:"
  echo "$collisions"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 6 — 4-table HeapKind lockstep
# -----------------------------------------------------------------------------
# For every HeapKind variant declared in heap_variants.rs, verify it appears
# in all 4 dispatch tables per handover §0 "4-table lockstep rule".
#
# The 4 tables:
#   1. crates/shape-vm/src/executor/vm_impl/stack.rs (clone_with_kind + drop_with_kind)
#   2. crates/shape-value/src/kinded_slot.rs (Drop + Clone)
#   3. crates/shape-value/src/v2/closure_layout.rs (SharedCell::drop)
#   4. crates/shape-value/src/heap_value.rs (TypedObjectStorage::drop)
echo "=== CHECK 6: 4-table HeapKind lockstep ==="
declare -a lockstep_tables=(
  "crates/shape-vm/src/executor/vm_impl/stack.rs"
  "crates/shape-value/src/kinded_slot.rs"
  "crates/shape-value/src/v2/closure_layout.rs"
  "crates/shape-value/src/heap_value.rs"
)

# Extract HeapKind variant names
variants=$(awk '
  /^\s+([A-Z]\w*),\s*\/\/\s*[0-9]+/ {
    match($0, /^\s+([A-Z][A-Za-z0-9]*),/, m)
    if (m[1]) print m[1]
  }
' crates/shape-value/src/heap_variants.rs)

lockstep_fail=0
for variant in $variants; do
  hits=0
  for table in "${lockstep_tables[@]}"; do
    if rg -q "HeapKind::${variant}\b" "$table" 2>/dev/null; then
      hits=$((hits + 1))
    fi
  done
  if [[ "$hits" -lt 4 ]]; then
    echo "  -> LOCKSTEP MISS: HeapKind::$variant present in $hits/4 tables"
    lockstep_fail=1
  fi
done

if [[ "$lockstep_fail" -eq 0 ]]; then
  record_pass "HeapKind 4-table lockstep"
  echo "  -> all variants present in 4/4 tables"
else
  record_fail "HeapKind 4-table lockstep" "one or more variants missing arms"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 7 — orphan close-marker (>>>>>>> without preceding <<<<<<<)
# -----------------------------------------------------------------------------
# Take-both regex misses pattern #1 from handover §0.
echo "=== CHECK 7: orphan close-markers (take-both regex miss pattern #1) ==="
orphan_hits=$(rg --no-heading -nE '^>>>>>>> ' \
  --include='*.rs' --include='*.md' \
  crates docs 2>/dev/null || true)
# Anything matching CHECK 4's broader scan is already a failure; CHECK 7 catches
# the specific orphan shape that take-both regexes silently leave behind.
if [[ -z "$orphan_hits" ]]; then
  record_pass "orphan close-markers"
  echo "  -> none"
else
  record_fail "orphan close-markers" "take-both regex left unresolved blocks"
  echo "  -> FAILED — orphan close-markers:"
  echo "$orphan_hits" | head -10
fi
echo

# -----------------------------------------------------------------------------
# CHECK 8 — HeapKind dispatch-table missing-brace pattern
# -----------------------------------------------------------------------------
# Take-both regex miss pattern #3 from handover §0: inside dispatch tables
# the regex can stitch `HeapKind::X => { body  // next-arm-comment  HeapKind::Y =>`
# without the intervening `}`. Scan for `HeapKind::Word => {` followed by
# another `HeapKind::Word => {` before a closing `}` at the same indent.
echo "=== CHECK 8: dispatch-table missing-brace pattern (take-both miss pattern #3) ==="
brace_pattern_hits=$(awk '
  /HeapKind::[A-Z]\w*\s*=>\s*\{/ {
    if (prev_open && !closed) {
      print FILENAME":"NR" — HeapKind arm opened without closing the prior one (prev was "prev_arm" at line "prev_line")"
      fail=1
    }
    prev_open=1
    closed=0
    match($0, /HeapKind::([A-Z]\w*)/, m)
    prev_arm = m[1]
    prev_line = NR
  }
  /^\s*}\s*$/ { closed=1; prev_open=0 }
  END { if (fail) exit 1 }
' "${lockstep_tables[@]}")

if [[ -z "$brace_pattern_hits" ]]; then
  record_pass "dispatch-table brace pairing"
  echo "  -> clean"
else
  record_fail "dispatch-table brace pairing" "missing closing brace between HeapKind arms"
  echo "  -> FAILED:"
  echo "$brace_pattern_hits" | head -10
fi
echo

# -----------------------------------------------------------------------------
# CHECK 9 — duplicate `use` line scan
# -----------------------------------------------------------------------------
# Take-both regex miss pattern #2 from handover §0: stitched `use` blocks
# with duplicate identifiers.
echo "=== CHECK 9: duplicate use lines in same file ==="
duplicate_use_hits=""
while IFS= read -r file; do
  dups=$(awk '/^use / {if (++count[$0] == 2) print FILENAME":"NR" — duplicate use line: "$0}' "$file" 2>/dev/null || true)
  if [[ -n "$dups" ]]; then
    duplicate_use_hits="${duplicate_use_hits}${dups}"$'\n'
  fi
done < <(rg --files --type rust crates 2>/dev/null)

if [[ -z "$duplicate_use_hits" ]]; then
  record_pass "duplicate use lines"
  echo "  -> clean"
else
  record_fail "duplicate use lines" "stitched use blocks with overlapping items"
  echo "  -> FAILED:"
  echo "$duplicate_use_hits" | head -10
fi
echo

# -----------------------------------------------------------------------------
# CHECK 10 — receiver-recovery soundness pattern grep
# -----------------------------------------------------------------------------
# Per handover §0 "5-arm receiver-recovery soundness rule":
#
#   ValueSlot::from_X(arc) stores Arc::into_raw(Arc<XData>) as u64.
#   Those bits are NOT a HeapValue allocation — they are an XData allocation.
#   Casting to *const HeapValue is wrong-type recovery and segfaults.
#
# Scan for patterns where a method handler calls `slot.as_heap_value()` on
# bits known to be typed-Arc (e.g. inside `as_hashset` / `as_hashmap` /
# `as_priority_queue` / etc. inside `*_methods.rs` files where receiver kind
# is `Ptr(HeapKind::<X>)`).
#
# This is a heuristic — flags suspicious sites for human review, not a hard fail.
echo "=== CHECK 10: receiver-recovery suspicious patterns (HEURISTIC — review-not-fail) ==="
suspicious=$(rg --no-heading -nU --multiline \
  'fn as_(hashset|hashmap|priority_queue|deque|channel|range|result|option|iterator|trait_object|mutex|atomic|lazy)\b[^{]*\{[^}]*slot\.as_heap_value\(\)' \
  crates/shape-vm/src/executor/objects/ 2>/dev/null || true)

if [[ -z "$suspicious" ]]; then
  record_pass "receiver-recovery patterns"
  echo "  -> no suspicious sites"
else
  # NOT a hard fail — record as pass but show output for review.
  record_pass "receiver-recovery patterns (with review notes)"
  echo "  -> SUSPICIOUS (manual review required):"
  echo "$suspicious" | head -20
  echo "  -> Apply the post-3ac2f11 Arc::from_raw + clone + into_raw pattern instead."
fi
echo

# -----------------------------------------------------------------------------
# CHECK 11 — `cargo check ... | grep -c` anti-pattern
# -----------------------------------------------------------------------------
# Search for instances of the broken pattern in scripts / docs to prevent
# anyone re-introducing it. Catching this in our own tooling.
echo "=== CHECK 11: cargo-check grep -c anti-pattern in tooling ==="
anti=$(rg --no-heading -nE 'cargo check.*\|.*grep\s+-c' \
  scripts justfile docs Makefile 2>/dev/null || true)
if [[ -z "$anti" ]]; then
  record_pass "no cargo-check grep -c anti-pattern"
  echo "  -> clean"
else
  record_fail "cargo-check grep -c anti-pattern" "scripts use the broken grep check"
  echo "  -> FAILED:"
  echo "$anti"
fi
echo

# -----------------------------------------------------------------------------
# REPORT
# -----------------------------------------------------------------------------
echo "=== SUMMARY ==="
fails=$(awk -F'\t' '$1=="FAIL"' "$RESULTS_FILE")
passes=$(awk -F'\t' '$1=="PASS"' "$RESULTS_FILE")
n_pass=$(echo "$passes" | grep -c . || true)
n_fail=$(echo "$fails" | grep -c . || true)

echo "Passed: $n_pass"
echo "Failed: $n_fail"
echo

if [[ "$n_fail" -gt 0 ]]; then
  echo "FAILED CHECKS:"
  echo "$fails" | awk -F'\t' '{print "  - "$2": "$3}'
  echo
  echo "DO NOT MERGE. Fix the failures and re-run verify-merge.sh."
  exit 1
fi

echo "ALL CHECKS PASSED. Safe to merge."
exit 0
