#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "${script_dir}/.." && pwd)

shape_bin=${SHAPE_BIN:-"${repo_root}/target/debug/shape"}
shape_fuzz_bin=${SHAPE_FUZZ_BIN:-"${repo_root}/target/debug/shape-fuzz"}
timeout_secs=${SHAPE_FUZZ_TIMEOUT_SECS:-30}
findings_dir=${SHAPE_FUZZ_FINDINGS_DIR:-"${repo_root}/tools/shape-fuzz/findings/per-commit"}

# Curated golden subset only. The full corpus intentionally stays in
# nightly-fuzz.yml because it includes known negative/divergent seeds.
seeds=(
  "tools/shape-fuzz/tests/corpus/arithmetic/a01_add_int.shape"
  "tools/shape-fuzz/tests/corpus/arithmetic/a09_for_sum.shape"
  "tools/shape-fuzz/tests/corpus/collections/c01_typed_map_sum.shape"
  "tools/shape-fuzz/tests/corpus/closures/f01_map_inline.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m03_option_some.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m04_option_none.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m05_result_ok.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m09_result_err.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m10_option_question.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m11_result_question.shape"
  "tools/shape-fuzz/tests/corpus/patterns/m12_result_context_bangbang.shape"
  "tools/shape-fuzz/tests/corpus/generics/g01_id_int.shape"
  "tools/shape-fuzz/tests/corpus/fallthrough/w01_module_read.shape"
)

if [[ ! -x "${shape_bin}" ]]; then
  echo "differential-gate: missing executable shape binary: ${shape_bin}" >&2
  echo "differential-gate: build it with: cargo build -p shape-cli --bin shape" >&2
  exit 2
fi

if [[ ! -x "${shape_fuzz_bin}" ]]; then
  echo "differential-gate: missing executable shape-fuzz binary: ${shape_fuzz_bin}" >&2
  echo "differential-gate: build it with: cargo build -p shape-fuzz --bin shape-fuzz" >&2
  exit 2
fi

mkdir -p "${findings_dir}"

echo "differential-gate: running ${#seeds[@]} curated VM-vs-JIT seeds"
echo "differential-gate: shape=${shape_bin}"
echo "differential-gate: shape-fuzz=${shape_fuzz_bin}"

for seed in "${seeds[@]}"; do
  seed_path="${repo_root}/${seed}"
  echo "differential-gate: ${seed}"
  "${shape_fuzz_bin}" run \
    --corpus "${seed_path}" \
    --shape-bin "${shape_bin}" \
    --timeout-secs "${timeout_secs}" \
    --findings-dir "${findings_dir}"
done

echo "differential-gate: curated VM-vs-JIT subset converged"
