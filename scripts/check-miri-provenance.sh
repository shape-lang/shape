#!/usr/bin/env bash
set -euo pipefail

# Narrow Miri provenance gate for the strict-flip provenance worker.
#
# This is targeted evidence, not a whole-runtime "no UB" proof. It covers only
# the test filters listed in print_coverage below. It deliberately does not run
# all crate tests, ignored tests, the full VM/JIT/FFI surface, or arbitrary
# Shape program execution. Do not summarize a passing run as "UB-free".
#
# Supervisor run shape:
#   systemd-run --user --wait --collect --pipe \
#     -p MemoryMax=16G -p MemorySwapMax=0 -p TasksMax=256 \
#     --setenv=PATH="$PATH" \
#     bash -c 'set -euo pipefail; cd /path/to/shape; direnv exec "$PWD" env CARGO_BUILD_JOBS=2 bash scripts/check-miri-provenance.sh'
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
if [[ -z "${CARGO_BUILD_JOBS:-}" ]]; then
  export CARGO_BUILD_JOBS=2
elif [[ "$CARGO_BUILD_JOBS" != "1" && "$CARGO_BUILD_JOBS" != "2" ]]; then
  echo "invalid CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}; expected 1 or 2" >&2
  echo "rerun under the supervisor cgroup policy with CARGO_BUILD_JOBS=2" >&2
  exit 2
fi
export CARGO_BUILD_JOBS

owned_target_dir=""
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  target_parent="${SHAPE_MIRI_TARGET_PARENT:-${XDG_CACHE_HOME:-$HOME/.cache}/shape-miri-targets}"
  mkdir -p "$target_parent"
  owned_target_dir="$(mktemp -d "${target_parent%/}/check-miri-provenance.XXXXXX")"
  export CARGO_TARGET_DIR="$owned_target_dir"
fi

cleanup() {
  if [[ -n "$owned_target_dir" && -d "$owned_target_dir" ]]; then
    rm -rf -- "$owned_target_dir" || true
  fi
}
trap cleanup EXIT

print_coverage() {
  cat <<'EOF'
Miri provenance gate coverage:
  - shape-value --lib provenance
      default Miri / Stacked Borrows
      MIRIFLAGS=-Zmiri-tree-borrows
  - shape-vm --lib result_option_carrier
      default Miri / Stacked Borrows
      MIRIFLAGS=-Zmiri-tree-borrows
      MIRIFLAGS=-Zmiri-strict-provenance
  - shape-vm --lib get_prop_typed_object_int_field_reads_via_raw
      default Miri / Stacked Borrows
      MIRIFLAGS=-Zmiri-tree-borrows
  - shape-vm --lib get_prop_typed_object_string_field_reads_via_raw
      default Miri / Stacked Borrows
      MIRIFLAGS=-Zmiri-tree-borrows
  - shape-vm --lib miri_stack_provenance
      stack sidecar read/pop/truncate/overwrite probes
      default Miri / Stacked Borrows
      MIRIFLAGS=-Zmiri-tree-borrows
      MIRIFLAGS=-Zmiri-strict-provenance

Boundary: passing this gate is evidence for the probes above only. It is not a
full UB proof for the VM, runtime, JIT, FFI, snapshots, or arbitrary Shape
program execution, all stack overwrite sites, and it does not classify or
execute ignored tests.
EOF
  echo
  echo "Resource settings:"
  echo "  CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}"
  echo "  CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
  if [[ -n "$owned_target_dir" ]]; then
    echo "  cleanup=enabled for private target dir"
  else
    echo "  cleanup=disabled; caller supplied CARGO_TARGET_DIR"
  fi
}

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
    echo "    command: MIRIFLAGS=${flags} ${RUSTUP} run nightly cargo miri test -p ${crate} --lib ${filter}"
    env MIRIFLAGS="$flags" "$RUSTUP" run nightly cargo miri test -p "$crate" --lib "$filter"
  else
    echo "    command: ${RUSTUP} run nightly cargo miri test -p ${crate} --lib ${filter}"
    env -u MIRIFLAGS "$RUSTUP" run nightly cargo miri test -p "$crate" --lib "$filter"
  fi
}

print_coverage

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

run_miri "shape-vm typed-object get_prop string raw read, Stacked Borrows" "" \
  shape-vm get_prop_typed_object_string_field_reads_via_raw
run_miri "shape-vm typed-object get_prop string raw read, Tree Borrows" "-Zmiri-tree-borrows" \
  shape-vm get_prop_typed_object_string_field_reads_via_raw

run_miri "shape-vm stack Miri provenance sidecar read/pop/truncate/overwrite, Stacked Borrows" "" \
  shape-vm miri_stack_provenance
run_miri "shape-vm stack Miri provenance sidecar read/pop/truncate/overwrite, Tree Borrows" "-Zmiri-tree-borrows" \
  shape-vm miri_stack_provenance
run_miri "shape-vm stack Miri provenance sidecar read/pop/truncate/overwrite, Strict Provenance" "-Zmiri-strict-provenance" \
  shape-vm miri_stack_provenance

run_miri "shape-vm Result/Option carrier, Strict Provenance" "-Zmiri-strict-provenance" \
  shape-vm result_option_carrier

echo
echo "Miri provenance gate complete for the targeted probes listed above."
