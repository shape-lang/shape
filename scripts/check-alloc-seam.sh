#!/usr/bin/env bash
# Allocation-seam guard (#194, ADR-018 §4).
#
# Typed heap carriers allocate through ONE seam:
# `shape_value::v2::heap_alloc`. This gate fails the build if any source file
# outside the seam calls the raw allocator directly.
#
# Why a gate and not just a convention: the seam is where the `alloc_budget`
# heap ceiling is enforced and where region allocation (#195) will hook in. A
# single direct `std::alloc::alloc` elsewhere silently reopens the bypass that
# this ticket closed — the ceiling would stop bounding that path, and no test
# would notice, because the bypass is invisible at runtime. The seam's value is
# exactly its exclusivity, so exclusivity is what gets checked.
#
# Scope: source trees only (crates/, bin/, tools/, extensions/). Comments and
# doc-comments are excluded — several of them name the raw functions precisely
# to document the historical allocator-pair defect this discipline exists to
# prevent, and rewriting that prose to satisfy a grep would destroy the record.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

scope=(crates bin tools extensions)

# The seam itself is the one place these calls are allowed.
seam="crates/shape-value/src/v2/heap_alloc.rs"

# Raw allocator entry points. `Layout` and `handle_alloc_error` are NOT
# allocation — they are layout arithmetic and the OOM hook — so they stay legal
# everywhere.
pattern='std::alloc::(alloc|alloc_zeroed|realloc|dealloc)\s*\('

hits="$(
  rg --no-heading --line-number --pcre2 "$pattern" "${scope[@]}" \
    --glob "!${seam##*/}" 2>/dev/null \
    | grep -vE '^[^:]+:[0-9]+:\s*(//|/\*|\*)' \
    || true
)"

if [[ -n "$hits" ]]; then
  echo "FAIL: direct allocator calls outside the #194 seam:" >&2
  echo "$hits" >&2
  echo >&2
  echo "Use shape_value::v2::heap_alloc instead:" >&2
  echo "  alloc_block / alloc_zeroed_block  — no refusal channel (breach recorded, allocation proceeds)" >&2
  echo "  try_alloc_block / try_realloc_block — refusal channel (breach refuses, caller reports)" >&2
  echo "  realloc_block                     — growth with no refusal channel" >&2
  echo "  dealloc_block" >&2
  exit 1
fi

echo "OK    allocation seam: zero direct allocator calls outside $seam"
