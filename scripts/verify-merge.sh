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
# CHECK 2 — canonical clean-check gate exits 0
# -----------------------------------------------------------------------------
# Same target set as `just check-clean`: `--all-targets` (benches rejoined
# the gate 2026-07-05 after the stale shape-vm bench stub was deleted).
if [[ "$fast_mode" -eq 0 ]]; then
  echo "=== CHECK 2: cargo check --workspace --all-targets ==="
  if cargo check --workspace --all-targets 2>&1 | tail -5; then
    record_pass "cargo check --workspace --all-targets"
    echo "  -> CLEAN"
  else
    record_fail "cargo check --workspace --all-targets" "exit non-zero"
    echo "  -> FAILED (exit non-zero from cargo)"
  fi
  echo
else
  echo "=== CHECK 2: cargo check (canonical gate) (SKIPPED in --fast mode) ==="
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
# CHECK 6b — JIT retain/release HeapKind lockstep (tables 5 & 6)
# -----------------------------------------------------------------------------
# Audit 2026-07-04 (docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md §5):
# the JIT retain/release dispatch in
#   crates/shape-jit/src/mir_compiler/ownership.rs
#     (`retain_func_for_place` / `release_func_for_place`)
# and the per-HeapKind FFI retain/release bodies in
#   crates/shape-jit/src/ffi/v2/collection_arc.rs
# are a 5th/6th HeapKind dispatch table OUTSIDE the 4-table lockstep above.
# A new HeapKind variant silently falls through to `_ => arc_retain /
# arc_release` — the legacy `UnifiedValue<T>` HeapHeader carrier (refcount at
# offset +4), which is the WRONG carrier shape for typed-Arc kinds (Rust Arc
# control block, refcount at offset -16). This exact class has already
# produced three documented segfault families (W12 string-carrier, W15.2
# closure-carrier, r5c-2-β-δ TypedArray-carrier).
#
# Rule: every HeapKind variant must EITHER
#   (a) have an explicit retain arm AND release arm in ownership.rs whose
#       `self.ffi.*` targets resolve to defined `pub extern "C" fn jit_*`
#       symbols in crates/shape-jit/src (retain/release as a pair), OR
#   (b) be listed in the audited legacy-carrier baseline below — variants
#       that fell through to the legacy fallback at baseline-freeze time
#       (2026-07-05). The baseline may only SHRINK: a baseline variant that
#       gains arms must be removed here (stale-baseline failure).
#
# Exit-code / list-comparison based, NOT grep -c of cargo output.
echo "=== CHECK 6b: JIT retain/release HeapKind lockstep (tables 5/6) ==="
jit_ownership_table="crates/shape-jit/src/mir_compiler/ownership.rs"
jit_collection_arc_table="crates/shape-jit/src/ffi/v2/collection_arc.rs"

# Audited 2026-07-05: these variants have NO explicit arm in the JIT
# retain/release dispatch and ride the legacy `arc_retain`/`arc_release`
# HeapHeader fallback. Frozen residual surface — new variants may NOT be
# added here without a dated audit note proving the legacy HeapHeader
# carrier is the actual carrier shape for the new kind.
# (Note: `String` is carried as `NativeKind::String`, not
# `Ptr(HeapKind::String)`; its dedicated arm is checked via the FFI-target
# resolution pass below, so it sits in this list for the Ptr-arm scan only.)
declare -a jit_lockstep_baseline=(
  String
  Decimal
  BigInt
  DataTable
  Future
  TaskGroup
  Temporal
  TableView
  Content
  Instant
  IoHandle
  NativeScalar
  NativeView
  Char
  FilterExpr
  Reference
  SharedCell
  Iterator
  Range
  TraitObject
  ModuleFn
  Matrix
  MatrixSlice
)

jit_lockstep_fail=0

if [[ ! -f "$jit_ownership_table" || ! -f "$jit_collection_arc_table" ]]; then
  echo "  -> JIT LOCKSTEP TABLE MOVED: $jit_ownership_table and/or $jit_collection_arc_table not found — relocate the tables and update this check"
  jit_lockstep_fail=1
else
  # --- Table 5: per-variant retain+release arm coverage in ownership.rs ---
  for variant in $variants; do
    arm_count=$(rg -c "NativeKind::Ptr\(HeapKind::${variant}\)\) =>" "$jit_ownership_table" 2>/dev/null || true)
    arm_count=${arm_count:-0}
    in_baseline=0
    for b in "${jit_lockstep_baseline[@]}"; do
      if [[ "$b" == "$variant" ]]; then
        in_baseline=1
        break
      fi
    done
    if [[ "$in_baseline" -eq 1 ]]; then
      if [[ "$arm_count" -gt 0 ]]; then
        echo "  -> STALE BASELINE: HeapKind::$variant now has JIT dispatch arms — remove it from jit_lockstep_baseline in this script"
        jit_lockstep_fail=1
      fi
    else
      if [[ "$arm_count" -lt 2 ]]; then
        echo "  -> JIT LOCKSTEP MISS: HeapKind::$variant has $arm_count/2 arms (retain + release) in $jit_ownership_table"
        jit_lockstep_fail=1
      fi
    fi
  done

  # --- Table 5 -> 6 linkage: every HeapKind/String arm target must resolve
  # to a defined `pub extern "C" fn jit_<field>` and come as a
  # retain/release pair.
  jit_ffi_fields=$(
    {
      rg -oN 'NativeKind::Ptr\(HeapKind::\w+\)\) => self\.ffi\.(\w+)' -r '$1' "$jit_ownership_table" 2>/dev/null || true
      rg -oN 'NativeKind::String\) => self\.ffi\.(\w+)' -r '$1' "$jit_ownership_table" 2>/dev/null || true
    } | LC_ALL=C sort -u
  )
  for field in $jit_ffi_fields; do
    if ! rg -q "pub extern \"C\" fn jit_${field}\(" crates/shape-jit/src 2>/dev/null; then
      echo "  -> JIT LOCKSTEP MISS: dispatch arm targets self.ffi.${field} but no \`pub extern \"C\" fn jit_${field}\` is defined under crates/shape-jit/src"
      jit_lockstep_fail=1
    fi
    case "$field" in
      *_retain)
        sibling="${field%_retain}_release"
        if ! printf '%s\n' "$jit_ffi_fields" | grep -qx "$sibling"; then
          echo "  -> JIT LOCKSTEP MISS: retain target ${field} has no matching release arm target (${sibling}) in $jit_ownership_table"
          jit_lockstep_fail=1
        fi
        ;;
      *_release)
        sibling="${field%_release}_retain"
        if ! printf '%s\n' "$jit_ffi_fields" | grep -qx "$sibling"; then
          echo "  -> JIT LOCKSTEP MISS: release target ${field} has no matching retain arm target (${sibling}) in $jit_ownership_table"
          jit_lockstep_fail=1
        fi
        ;;
    esac
  done

  # --- Table 6 internal pairing: every per-kind retain body in
  # collection_arc.rs must have its release sibling in the same file
  # (and vice versa).
  ca_retains=$(rg -oN 'pub extern "C" fn (jit_arc_\w+)_retain\(' -r '$1' "$jit_collection_arc_table" 2>/dev/null | LC_ALL=C sort -u || true)
  ca_releases=$(rg -oN 'pub extern "C" fn (jit_arc_\w+)_release\(' -r '$1' "$jit_collection_arc_table" 2>/dev/null | LC_ALL=C sort -u || true)
  if [[ "$ca_retains" != "$ca_releases" ]]; then
    echo "  -> JIT LOCKSTEP MISS: retain/release pair mismatch in $jit_collection_arc_table:"
    diff <(printf '%s\n' "$ca_retains") <(printf '%s\n' "$ca_releases") | head -10 || true
    jit_lockstep_fail=1
  fi
fi

if [[ "$jit_lockstep_fail" -eq 0 ]]; then
  record_pass "HeapKind JIT retain/release lockstep (tables 5/6)"
  echo "  -> all non-baseline variants have paired JIT arms + resolved FFI targets"
else
  record_fail "HeapKind JIT retain/release lockstep (tables 5/6)" "variant missing JIT retain/release arm or unresolved FFI target"
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
# CHECK 12 — JIT-private HK_* ordinal-collision guard
# -----------------------------------------------------------------------------
# Per phase-2d-hardening item (i) close (W17-jit-legacy-ordinal-disambiguation,
# 2026-05-12): every JIT-private `HK_*` constant in
# `crates/shape-jit/src/ffi/value_ffi.rs` must either alias `HeapKind::X as u16`
# directly or sit at or above `JIT_LEGACY_HK_BASE` (currently 256). This guard
# prevents the regression where a future agent re-introduces a literal small
# u16 (e.g. `pub const HK_FOO: u16 = 12;`) that would collide with a runtime-
# tier HeapKind ordinal.
echo "=== CHECK 12: JIT-private HK_* ordinal-collision guard ==="
hk_table="crates/shape-jit/src/ffi/value_ffi.rs"
hk_violations=$(awk '
  /^pub const HK_[A-Z0-9_]+: u16 =/ {
    line = $0
    # Match `pub const HK_FOO: u16 = <RHS>;` and extract RHS.
    if (match(line, /^pub const (HK_[A-Z0-9_]+): u16 = ([^;]+);/, m)) {
      name = m[1]
      rhs = m[2]
      gsub(/^[ \t]+|[ \t]+$/, "", rhs)
      # Accept: HeapKind::<X> as u16 (canonical Tier 1 alias)
      if (rhs ~ /^HeapKind::[A-Za-z0-9_]+ as u16$/) next
      # Accept: JIT_LEGACY_HK_BASE [+ N] (Tier 2 JIT-private contiguous block)
      if (rhs ~ /^JIT_LEGACY_HK_BASE([ \t]*\+[ \t]*[0-9]+)?$/) next
      # Accept: numeric literal >= 256 (JIT-private private-range manually written)
      if (rhs ~ /^[0-9]+$/) {
        if (rhs + 0 >= 256) next
      }
      # Otherwise: violation
      print FILENAME":"NR" — "name" = "rhs" (must alias HeapKind::X as u16 OR be JIT_LEGACY_HK_BASE [+ N] OR be >= 256)"
      fail = 1
    }
  }
  END { if (fail) exit 1 }
' "$hk_table")

if [[ -z "$hk_violations" ]]; then
  record_pass "JIT-private HK_* ordinal guard"
  echo "  -> clean (every HK_* is HeapKind-aliased or >= 256)"
else
  record_fail "JIT-private HK_* ordinal guard" "low-numbered JIT-private HK_*"
  echo "  -> FAILED:"
  echo "$hk_violations"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 13 — colon-return-type doc-drift guard (Rust-shaped syntax discipline)
# -----------------------------------------------------------------------------
# Per user disposition 2026-05-18: Shape uses arrow `->` for return types,
# colon `:` only for type annotations (param types, field types, let-bindings,
# generic bounds). The TypeScript-shaped `name(...): RetType` form was never
# authorized and has been deleted from the grammar (shape.pest:186 trait_member_signature
# Form A removed; new Form B requires `fn` keyword + arrow return).
#
# This check catches re-introduction in living docs + stdlib + LSP hover/completion
# text. Source trees (crates/, tools/, bin/) are independently gated by parser
# tests in shape-ast — if a test fixture uses Form A, it fails to parse.
echo "=== CHECK 13: colon-return-type doc-drift guard ==="
colon_return_scope=(CLAUDE.md README.md docs crates/shape-runtime/stdlib-src tools/shape-lsp/src/hover.rs tools/shape-lsp/src/completion)
# Pattern: trait method signature shape `name(...): TypeIdent[;]?` at line start.
# Requires the post-colon text to be a single type identifier (PascalCase + optional
# generic OR known primitive) followed by end-of-line / `;` / `,`. This excludes
# natural-language false positives like `free(): invalid pointer` (C error text)
# and `precedent (survey 03 §1.12): functional-style ...` (citation continuation)
# which have multi-word sentence text after the colon.
colon_return_hits=$(rg --no-heading -n -P '^\s*[a-z_][a-zA-Z0-9_]*\s*\([^)]*\)\s*:\s*([A-Z][a-zA-Z0-9_]*(?:<[^>]*>)?|bool|int|string|number|byte|unit|any|void|Self|float|i32|i64|f32|f64|u8|u16|u32|u64|usize|isize|duration|datetime|decimal|bigint)\s*[;,]?\s*$' "${colon_return_scope[@]}" 2>/dev/null \
  | rg -v ':\s*//' \
  | rg -v '\bfn\s+[a-z_]' \
  | rg -v '\bmethod\s+[a-z_]' \
  | rg -v '^docs/cluster-audits/v0\.3(\.3)?-?(classification)?/' \
  || true)

if [[ -z "$colon_return_hits" ]]; then
  record_pass "colon-return-type doc-drift guard"
  echo "  -> clean (no Form A trait method signatures in living docs scope)"
else
  record_fail "colon-return-type doc-drift guard" "Form A re-introduced"
  echo "  -> FAILED — colon-return-type sites found (use arrow \`-> RetType\` instead):"
  echo "$colon_return_hits" | head -20
fi
echo

# -----------------------------------------------------------------------------
# CHECK 14 — HeapKind wildcard dispatch guard
# -----------------------------------------------------------------------------
# New `HeapKind` variants must not disappear into broad heap-pointer /
# legacy-HK wildcard arms. This non-cargo guard has an audited baseline for
# current residual surfaces and fails on newly introduced wildcard absorbers.
echo "=== CHECK 14: HeapKind wildcard dispatch guard ==="
if bash scripts/check-heapkind-wildcards.sh; then
  record_pass "HeapKind wildcard dispatch guard"
  echo "  -> clean"
else
  record_fail "HeapKind wildcard dispatch guard" "new wildcard HeapKind dispatch pattern"
  echo "  -> FAILED (new wildcard HeapKind dispatch pattern)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 15 — ADR-011–016 legacy-authority baselines (R14 shrink-only ratchet)
# -----------------------------------------------------------------------------
# The #133/#134/#135 legacy inventories may only shrink. Fails on total
# growth, a new owner path, or per-owner growth; exit 2 on a hand-edited
# baseline (sets_sha256 mismatch). Regenerate legitimately shrunk sets with
# `just regen-legacy-baselines` and commit the diff.
echo "=== CHECK 15: legacy-authority baselines (shrink-only) ==="
if node scripts/check-adr011-012-legacy-baselines.mjs; then
  record_pass "legacy-authority baselines"
  echo "  -> clean"
else
  record_fail "legacy-authority baselines" "a legacy set grew or a baseline was hand-edited"
  echo "  -> FAILED (a legacy set grew or a baseline was hand-edited)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 16 — ADR-011–016 legacy identity manifest (R14 identity-default routing)
# -----------------------------------------------------------------------------
# The #136 manifest enumerates every name-selected builtin identity that
# currently holds legacy privilege, keyed by BuiltinFunction variant rather than
# by spelling. An identity or spelling that is NOT listed gets no privilege:
# adding one fails here until it is listed in the same commit, so the grant is
# reviewable. Also fails when a legacy mechanism (prefix gate,
# allow_internal_builtins, stdlib-name membership, module-builtin route) spreads
# to a new file. Exit 2 on a hand-edited manifest.
echo "=== CHECK 16: legacy identity manifest (no unlisted privilege) ==="
if node scripts/check-adr011-012-legacy-identity-manifest.mjs; then
  record_pass "legacy identity manifest"
  echo "  -> clean"
else
  record_fail "legacy identity manifest" "an unlisted identity/spelling gained privilege, a mechanism spread, or the manifest was hand-edited"
  echo "  -> FAILED (unlisted privilege, mechanism spread, or hand-edited manifest)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 17 — ADR-016 public feature manifest contract (#112, R19)
# -----------------------------------------------------------------------------
# The PublicFeatureManifest is Shape's inventory of user-observable features, and
# this check is what stops a coverage gate being made green by editing the
# denominator. A row's status is derived from its evidence rather than asserted
# beside it; status moves forward only, so a public feature cannot be relabelled
# `planned` to shed its runnable-evidence obligation; every published feature_id
# stays in the inventory forever, as a live row or a tombstoned removed row, so
# identities are never reused and features never silently disappear; and no
# source revision, counterpart SHA, attestation or mutable verification state may
# enter — the gate rejects a SCHEMA declaring such a field, not only a manifest
# carrying one. --self-test runs the forced negatives, each of which also asserts
# its unmutated positive control is accepted.
echo "=== CHECK 17: public feature manifest contract (ADR-016) ==="
if node scripts/check-adr011-012-public-feature-manifest.mjs --self-test; then
  record_pass "public feature manifest"
  echo "  -> clean"
else
  record_fail "public feature manifest" "a status moved backward, an identity was reused or dropped, pair-evidence state entered the manifest, or a tripwire stopped firing"
  echo "  -> FAILED (backward status, reused/dropped identity, pair-evidence state, or a dead tripwire)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 18 — ADR-016 Book coverage manifest contract (#113, R19)
# -----------------------------------------------------------------------------
# The coverage half of the pair CHECK 17 opens. A fence's classification is
# derived from the fields it carries, so a hard example cannot be relabelled
# illustrative-only to stop being executed; section and fence identities carry no
# line number or ordinal position, so moving prose cannot read as removing and
# adding a feature; every published identity stays live or tombstoned, and a
# tombstone is frozen; a fence identity is declared exactly once, because a fence
# is one physical block. The same revision/attestation/verification fields CHECK
# 17 forbids are rejected here in the schema as well as the manifest, and the
# three enums shared with the PublicFeatureManifest schema must be identical —
# an obligation one manifest declares and the other cannot express is a rule that
# silently never applies. --self-test runs the forced negatives, each of which
# also asserts its unmutated positive control is accepted.
echo "=== CHECK 18: Book coverage manifest contract (ADR-016) ==="
if node scripts/check-adr011-012-book-coverage-manifest.mjs --self-test; then
  record_pass "book coverage manifest"
  echo "  -> clean"
else
  record_fail "book coverage manifest" "a fence was reclassified, an identity was reused or dropped, an enum drifted from the public manifest, pair-evidence state entered the manifest, or a tripwire stopped firing"
  echo "  -> FAILED (reclassified fence, reused/dropped identity, enum drift, pair-evidence state, or a dead tripwire)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 19 — ADR-016 public feature candidate inventory (#114, R19)
# -----------------------------------------------------------------------------
# ADR-016 §3 lets a complete inventory be built in bounded waves "only after
# their exact stable rows and content hash are committed". This re-derives those
# rows from the grammar, the stdlib sources, the CLI enums, the LSP capabilities
# and the ABI on every run, so the denominator the wave breakdown is planned
# against cannot silently go stale. Shrinkage is reported before growth: a
# public surface that disappears from the candidate inventory before it was ever
# entered in the manifest never gets the removed-row tombstone ADR-016 §2
# requires. Regenerate with `just regen-public-feature-candidates`; the diff is
# the review surface.
echo "=== CHECK 19: public feature candidate inventory (ADR-016) ==="
if node scripts/check-adr011-012-public-feature-candidates.mjs --self-test; then
  record_pass "public feature candidates"
  echo "  -> clean"
else
  record_fail "public feature candidates" "a public surface appeared or disappeared without regenerating the inventory, a scan gap was dropped, or a candidate row acquired a classification"
  echo "  -> FAILED (inventory drift, dropped scan gap, or a classified candidate row)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 20 — ADR-016 Book fence universe (#115, R19)
# -----------------------------------------------------------------------------
# ADR-016 §6 requires the full Shape-fence universe, not a curated subset, and
# §5 requires every fence to carry a stable explicit identity and one
# classification. The committed inventory holds every fence in shape-web's
# COMMITTED content with the markers saying what §5 still wants. This check
# recomputes both content hashes rather than trusting the stored fields, verifies
# the counts against the rows, refuses a scanned row that acquired a real §5
# classification, and rejects a fence identity built from a line number or an
# ordinal. When a shape-web checkout is present it also re-derives from the
# corpus; when it is not, it says so rather than reporting a skipped check as a
# pass.
echo "=== CHECK 20: Book fence universe (ADR-016) ==="
if node scripts/check-adr011-012-book-fence-inventory.mjs --self-test; then
  record_pass "book fence universe"
  echo "  -> clean"
else
  record_fail "book fence universe" "the inventory drifted from the corpus, a count or marker was edited, a scanned row acquired a classification, or an identity carried an ordinal"
  echo "  -> FAILED (inventory drift, edited count/marker, classified scan row, or an ordinal identity)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 21 — ADR-018 §2 whole-program JIT bail ratchet (#187, R24)
# -----------------------------------------------------------------------------
# An unsupported construct costs its enclosing function native execution, never
# the program. The whole-program bail set is shrink-only and ratcheted to zero.
# Four shapes of growth fail: a new bail site; a residual widened from Owner to
# Program scope, which turns a one-function cost into a whole-program one; a
# refusal added without its `// WHOLE-PROGRAM-BAIL[..]` marker, which is how a
# ratchet gets evaded rather than raised; and a marker retargeted at a different
# construct while the count stays flat. --self-test runs the forced negatives,
# each of which also asserts its unmutated positive control is accepted.
# Regenerate a legitimately shrunk baseline with `just regen-jit-bail-baseline`.
echo "=== CHECK 21: whole-program JIT bail ratchet (ADR-018 §2) ==="
if node scripts/check-jit-bail-baseline.mjs --self-test; then
  record_pass "whole-program JIT bail ratchet"
  echo "  -> clean"
else
  record_fail "whole-program JIT bail ratchet" "a bail site was added, a residual widened to Program scope, a refusal carries no marker, a marker was retargeted, or the baseline was hand-edited"
  echo "  -> FAILED (new bail, widened residual, unmarked refusal, retargeted marker, or hand-edited baseline)"
fi
echo

# -----------------------------------------------------------------------------
# CHECK 22 — JIT FFI reachability (registration is not reachability)
# -----------------------------------------------------------------------------
# Three instances in one day of a symbol that LOOKED covered while nothing a
# user could run could reach it: jit_v2_string_eq (correct, tested, never
# registered), the jit_typeof/jit_to_string/... family (declared, zero call
# sites), and jit_v2_retain/jit_v2_release (registered AND declared, never
# given a FuncRef — the size-blind Layout(8,8) dealloc filed as live UB in
# #216 that was in fact unreachable). This makes the class fail by
# construction instead of being rediscovered one ticket at a time.
echo "=== CHECK 22: JIT FFI reachability ratchet (#216) ==="
if node scripts/check-jit-ffi-reachability.mjs --self-test; then
  record_pass "JIT FFI reachability ratchet"
  echo "  -> clean"
else
  record_fail "JIT FFI reachability ratchet" "a symbol was registered or declared without a FuncRef, a FuncRef field went unreferenced, a fixed symbol was left in the debt list, or an allow-list entry carries no reason"
  echo "  -> FAILED (unreachable registration, dead FuncRef, stale debt entry, or reasonless allow-list entry)"
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
