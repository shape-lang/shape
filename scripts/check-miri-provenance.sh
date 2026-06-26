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

run_expected_strict_failure() {
  local out
  out="$(mktemp "${TMPDIR:-/tmp}/shape-miri-strict-provenance.XXXXXX")"

  echo
  echo "==> strict provenance expected failure"
  echo "    crate: shape-vm"
  echo "    filter: result_option_carrier"
  echo "    MIRIFLAGS=-Zmiri-strict-provenance"

  set +e
  env MIRIFLAGS="-Zmiri-strict-provenance" "$RUSTUP" run nightly cargo miri test \
    -p shape-vm --lib result_option_carrier >"$out" 2>&1
  local status=$?
  set -e

  if [[ "$status" -eq 0 ]]; then
    cat "$out"
    rm -f "$out"
    echo "unexpected strict-provenance pass; update the audit/gate after the blocker is fixed" >&2
    return 1
  fi

  if grep -Fq "integer-to-pointer casts" "$out" \
    && grep -Fq "crates/shape-vm/src/executor/result_option_carrier.rs:229" "$out"; then
    echo "expected strict-provenance failure observed:"
    grep -F "integer-to-pointer casts" "$out" | head -n 1
    grep -F "crates/shape-vm/src/executor/result_option_carrier.rs:229" "$out" | head -n 2
    rm -f "$out"
    return 0
  fi

  cat "$out"
  rm -f "$out"
  echo "unexpected strict-provenance failure signature" >&2
  return 1
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

run_expected_strict_failure

echo
echo "Miri provenance gate complete."
