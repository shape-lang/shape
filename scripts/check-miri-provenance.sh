#!/usr/bin/env bash
set -euo pipefail

# Narrow Miri provenance gate for the strict-flip provenance worker.
#
# Run through direnv so the repo's linker/devenv environment is active:
#   direnv exec "$(git rev-parse --show-toplevel)" bash scripts/check-miri-provenance.sh
#
# The active devenv cargo is stable; use rustup-run nightly instead of
# `cargo +nightly`.

ROOT="${SHAPE_ROOT:-$(git rev-parse --show-toplevel)}"
cd "$ROOT"

RUSTUP="${RUSTUP:-/home/dev/.cargo/bin/rustup}"
if [[ ! -x "$RUSTUP" ]]; then
  echo "missing executable rustup at: $RUSTUP" >&2
  exit 2
fi

export CARGO_TERM_COLOR=never

run_miri() {
  local label="$1"
  local flags="$2"
  local crate="$3"
  local filter="$4"

  echo
  echo "==> ${label}"
  echo "    crate: ${crate}"
  echo "    filter: ${filter}"
  if [[ -n "$flags" ]]; then
    echo "    MIRIFLAGS=${flags}"
    env MIRIFLAGS="$flags" "$RUSTUP" run nightly cargo miri test -p "$crate" --lib "$filter"
  else
    env -u MIRIFLAGS "$RUSTUP" run nightly cargo miri test -p "$crate" --lib "$filter"
  fi
}

run_miri "shape-value provenance anchors, Stacked Borrows" "" \
  shape-value provenance
run_miri "shape-value provenance anchors, Tree Borrows" "-Zmiri-tree-borrows" \
  shape-value provenance

run_miri "shape-vm Result/Option carrier, Stacked Borrows" "" \
  shape-vm result_option_carrier
run_miri "shape-vm Result/Option carrier, Tree Borrows" "-Zmiri-tree-borrows" \
  shape-vm result_option_carrier

run_miri "shape-vm typed-object get_prop raw read, Stacked Borrows" "" \
  shape-vm get_prop_typed_object_int_field_reads_via_raw
run_miri "shape-vm typed-object get_prop raw read, Tree Borrows" "-Zmiri-tree-borrows" \
  shape-vm get_prop_typed_object_int_field_reads_via_raw

run_miri "shape-vm Result/Option carrier, Strict Provenance" "-Zmiri-strict-provenance" \
  shape-vm result_option_carrier

echo
echo "Miri provenance gate complete."
