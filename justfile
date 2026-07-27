set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

extension-crates := "shape-ext-python shape-ext-typescript"

# Per-process virtual-memory backstop for test runs (KiB). A runaway test (an
# unbounded allocating loop, e.g. from an inference regression) is bounded
# in-process by the VM's per-buffer heap ceiling + instruction cap; this ulimit
# is the hard system-level backstop so ANY runaway the in-VM caps miss (e.g. a
# many-small-retained-buffer climb) fails THIS process at ~48 GiB instead of
# climbing to ~83 GiB and OOM-killing / hanging the whole host. 48 GiB is
# generous for rustc/link + legitimate tests (which use < 1 GiB).
test-mem-cap-kib := "50331648"

default: build-extensions build-treesitter

# Build extension shared libraries and copy them into extensions/.
# WF-2A extension-hardening: default to `release` so `just build-extensions`
# produces an artifact matching the typical release host (`cargo build
# --release --bin shape`), reconciling the book (python-extension.mdx documents
# --release). Correctness no longer depends on this: the loader's structural
# ABI build-fingerprint gate (crates/shape-runtime/src/plugins/loader.rs) makes
# any true host/extension incompatibility fail cleanly at load, and the
# fingerprint is profile-independent so a `debug` build still loads into a
# release host. Pass `profile=debug` for a faster local build.
build-extensions profile="release":
	mkdir -p extensions
	for crate in {{extension-crates}}; do \
	  echo "Building ${crate} (profile={{profile}})"; \
	  if [[ "{{profile}}" == "release" ]]; then \
	    cargo build -p "${crate}" --release; src="target/release/lib${crate//-/_}.so"; \
	  else \
	    cargo build -p "${crate}"; src="target/debug/lib${crate//-/_}.so"; \
	  fi; \
	  if [[ -f "${src}" ]]; then cp "${src}" "extensions/$(basename "${src}")"; else echo "Skipping ${crate}: no artifact at ${src}"; fi; \
	done

# Compile the tree-sitter parser shared library for editors.
build-treesitter:
	mkdir -p tree-sitter-shape/parser
	cc -o tree-sitter-shape/parser/shape.so -shared -fPIC -fno-exceptions \
		-Itree-sitter-shape/src tree-sitter-shape/src/parser.c

clean-extensions:
	rm -f extensions/*.so

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

# --- Test Tiers ---

# Tier 0: Compile-check the canonical clean-gate target set (~5-8s).
# Uses the `check-clean` recipe — see its doc-comment for the exact target list.
test-check: check-clean

# Tier 1: Fast unit tests — no deep/soak, no integration
test-fast:
	ulimit -v {{test-mem-cap-kib}} && cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib

# Tier 2: Unit + deep tests, no integration
test:
	ulimit -v {{test-mem-cap-kib}} && cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests --features shape-jit/deep-tests

# Tier 3: Everything that should currently pass — unit + deep + soak + integration (~10-15 min)
#
# `--include-ignored` is intentionally NOT used here. There are pre-existing
# `#[ignore]`'d tests across crates that document known-broken subsystems
# (path-c2 c-alias gated 4 v2-raw-heap aliasing tests this way; shape-jit has
# ~23 width-aware/kernel/inline-array tests pre-existing on
# jit-v2-phase1@53a06ce; etc.). `just test-all` should hit 0 failed so it can
# serve as the merge-blocker gate; for ignored test inspection use
# `cargo test ... -- --ignored` per crate.
#
# `shape-jit/deep-tests` is also NOT enabled here: those heavy execution
# tests JIT-compile ~118 stdlib functions per test and SIGILL the JIT code
# cache under default n-cpu parallelism (the bug the path-c c-jit tier-gating
# works around). Run them via
# `cargo test -p shape-jit --lib --features deep-tests` or `just test-deep`
# with `--test-threads=1` instead.
#
# `shape-test` is excluded from the parallel sweep and run separately with
# `--test-threads=1` because its `annotations_comptime` integration suite has
# a parallel-contention flake (different test fails each run); single-thread
# is stable. Same precedent as path-c2's gating decisions.
test-all:
	ulimit -v {{test-mem-cap-kib}} && cargo test --workspace --exclude shape-test --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests
	ulimit -v {{test-mem-cap-kib}} && cargo test -p shape-test -- --test-threads=1

# Run only deep/soak tests
test-deep:
	ulimit -v {{test-mem-cap-kib}} && cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests --features shape-jit/deep-tests -- deep --include-ignored

# Run only shape-test integration suite
test-integration:
	ulimit -v {{test-mem-cap-kib}} && cargo test -p shape-test

# Foreign-call (FFI) e2e tier (ffi-rebuild §7, WF-2A stage 5).
#
# `bin/shape-cli/tests/ffi_e2e.rs` drives the real `shape` binary end-to-end
# across all three foreign verticals. The `extern C` probes need no extension
# and ALSO run in the default gate (`cargo test --workspace --all-targets`);
# the `fn python` / `fn typescript` probes are `#[ignore]`'d there because they
# need the built runtime `.so`s + CPython/V8. This tier builds the extensions
# first, then runs the FULL matrix via `--include-ignored`. Wired into
# `.github/workflows/ci.yml` as the `ffi` job so it is a CI tier that actually
# runs (the 2026-07-04 audit's root cause was foreign e2e tests gated out of
# every tier). `SHAPE_FFI_EXT_DIR` points the harness at the built `.so`s;
# absent extensions make the harness PANIC (never silently skip).
#
# NOTE: unlike the other test recipes this tier does NOT wrap the run in
# `ulimit -v {{test-mem-cap-kib}}`. Loading the TypeScript extension initializes
# V8, whose pointer-compression sandbox reserves a ~1 TB contiguous *virtual*
# address range up front (committed lazily). A `ulimit -v` cap makes that
# reservation fail with "Fatal process out of memory: Oilpan: CagedHeap
# reservation" the instant the `.so` is loaded — even for the python probes,
# since the harness loads every extension in SHAPE_FFI_EXT_DIR. Foreign-runtime
# memory is outside the VM-heap accounting anyway (ffi-rebuild §4.8.3), so the
# VM memory cap does not apply here. The CI `ffi` job runs the same commands
# without a cap for the same reason.
test-ffi: build-extensions
	SHAPE_FFI_EXT_DIR="{{justfile_directory()}}/extensions" cargo test -p shape-cli --test ffi_e2e -- --include-ignored
	cargo test -p shape-test --features e2e-python,e2e-typescript --test e2e_gated

# Run all tests for a single crate
test-crate crate:
	ulimit -v {{test-mem-cap-kib}} && (cargo test -p {{crate}} --features deep-tests 2>/dev/null || cargo test -p {{crate}})

# CI: full suite. Target set is `--all-targets` minus `--benches`: benches
# COMPILE under the `check-clean` gate, but criterion bench binaries are not
# run under `cargo test` (they measure, they don't assert).
ci-test:
	ulimit -v {{test-mem-cap-kib}} && cargo test --workspace --lib --bins --tests --examples --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests -- --include-ignored
	ulimit -v {{test-mem-cap-kib}} && cargo run -p xtask -- workspace-smoke

# Lightweight VM-vs-JIT differential gate. This uses the subprocess
# shape-fuzz harness against a curated golden subset; the full corpus stays
# in .github/workflows/nightly-fuzz.yml because it contains known negative
# divergence seeds.
differential-gate:
	ulimit -v {{test-mem-cap-kib}} && cargo build -p shape-cli --bin shape
	ulimit -v {{test-mem-cap-kib}} && cargo build -p shape-fuzz --bin shape-fuzz
	ulimit -v {{test-mem-cap-kib}} && bash scripts/differential-gate.sh

# --- Canonical clean-check gate ---

# Canonical "workspace clean" verifier. `just check-clean` exit 0 means the
# build gate is green; sub-cluster close gates and verify-merge.sh CHECK 1+2
# anchor on this command's coverage.
#
# Target set: `--all-targets` (lib + bins + tests + examples + benches).
# Benches rejoined the gate 2026-07-05 (WF-0A): the stale
# `typed_access_bench.rs` empty-criterion-group stub was deleted and
# `vm_benchmarks.rs` checks clean against the current opcode / slot ABI,
# so the historical `--benches` exclusion no longer applies.
#
# Crates covered: every workspace member (see top-level Cargo.toml `members`),
# i.e. shape-macros, shape-ast, shape-value, shape-wire, shape-runtime,
# shape-vm, shape-jit, shape-diagnostics, shape-viz-{core,native}, shape-cli,
# shape-lsp, shape-test, xtask, shape-abi-v1, shape-ext-python,
# shape-ext-typescript. (`shape-app` and `shape-server` live in a SEPARATE
# workspace at `../shape-app/` and are not workspace members here.)
check-clean:
	cargo check --workspace --all-targets

# --- Strict-typing plan gates (~/.claude/plans/stop-native-vs-tagged-tax.md) ---

# Defection guard: per-symbol monotonic-non-increasing check vs frozen baseline.
# See scripts/check-no-dynamic.sh and docs/check-no-dynamic-baseline.txt.
check-no-dynamic:
	bash scripts/check-no-dynamic.sh

# --- ADR-011..016 step-4 migration baselines (#133 / #134 / #135) ---

# Growth gate for the frozen legacy-authority sets: discovery producers, ambient
# comptime entry points and intrinsic selectors (#133); annotation identities,
# universal/string descriptors and backend exceptions (#134); duplicate LSP
# semantics, stale tests and old documentation claims (#135).
#
# Per ruling R14 a legacy set may only shrink. The gate fails on three shapes of
# growth: a rising set total, a new owner path, and a rising per-owner count
# that a fall elsewhere would otherwise hide. Definitions live in
# scripts/lib/adr011-012-legacy-sets.mjs; baselines in
# docs/program/adr011-012/baselines/.
check-legacy-baselines:
	node scripts/check-adr011-012-legacy-baselines.mjs

# Regenerate the step-4 baselines. Legitimate and expected after real migration
# progress — and always visible, because the committed diff is the review
# surface. Regenerating to silence a rise is the walk-back this gate exists to
# catch.
regen-legacy-baselines:
	node scripts/generate-adr011-012-legacy-baselines.mjs

# --- ADR-011..016 step-5 legacy identity manifest (#136) ---

# Identity-default guard: every name-selected builtin identity that currently
# holds legacy privilege is enumerated in
# docs/program/adr011-012/legacy-identity-manifest.json, keyed by
# BuiltinFunction variant (the behavior) rather than by source spelling.
# Anything unlisted gets no privilege, so adding a privileged arm or a new
# spelling fails until it is listed in the same commit. Also fails when a legacy
# mechanism (prefix gate, allow_internal_builtins, stdlib-name membership,
# module-builtin route) spreads to a new file.
check-legacy-identity:
	node scripts/check-adr011-012-legacy-identity-manifest.mjs

# Regenerate the #136 manifest after real migration progress — a retired
# builtin, a narrowed mechanism. The committed diff is the review surface.
regen-legacy-identity:
	node scripts/generate-adr011-012-legacy-identity-manifest.mjs

# Phase 2d merge gate. Run before merging any sub-cluster branch into
# bulldozer-strictly-typed. Exit-code-based (NOT grep -c) per handover §0.
# See docs/cluster-audits/phase-2d-handover.md §0 + scripts/verify-merge.sh.
verify-merge:
	bash scripts/verify-merge.sh

# Same as `just verify-merge` but skips the --tests pass (faster).
verify-merge-fast:
	bash scripts/verify-merge.sh --fast

# Phase 2 gate: shape-runtime --lib compiles cleanly.
# Reports the current error count; exits non-zero if > 0.
verify-phase-2:
	#!/usr/bin/env bash
	set -uo pipefail
	errors=$(cargo check -p shape-runtime --lib 2>&1 | rg -c '^error' || true)
	echo "shape-runtime --lib errors: ${errors:-0}"
	[[ "${errors:-0}" == "0" ]]

# Phase 5 gate: defection guard clean + sentinel test passes.
# (Sentinel test crates/shape-vm/src/executor/tests/no_dynamic.rs is not yet
# wired up; see CLAUDE.md "Mechanical enforcement". When it lands, add it here.)
verify-phase-5: check-no-dynamic
	@echo "TODO: invoke sentinel test when crates/shape-vm/src/executor/tests/no_dynamic.rs lands"

# Narrow provenance/Miri gate. This wraps the script in direnv because the
# active devenv cargo is stable-only; the script uses rustup-run nightly.
miri-provenance:
	direnv exec {{justfile_directory()}} bash scripts/check-miri-provenance.sh

# --- Phase 4c CI coverage gate (cargo-tarpaulin) ---
#
# Per docs/cluster-audits/v0.3-w14-test-coverage-audit.md §1-§6 + Phase 4
# acceptance criterion (user 2026-05-18): "test coverage ≥99% per-feature
# with DOCUMENTED EXCEPTIONS ONLY WHEN VERY HARD TO TEST".
#
# Cadence (per Phase 4 test execution policy in
# docs/cluster-audits/phase-3-team-lead-handover.md:42-47):
#   - Phase-4b-batch-merge invocation point (post-merge gate on main).
#   - Nightly (per W13.4 nightly-fuzz.yml precedent: cron 0 4 * * * UTC).
#   - NOT per-commit.
#
# Install per audit §1.6 (devenv-shell wrap required on NixOS hosts):
#   devenv shell --quiet -- bash -c 'cargo install cargo-tarpaulin --version 0.35.4 --locked'
#
# Full workspace line coverage.
coverage:
	bash scripts/coverage.sh

# Per-crate scoped coverage (uses --include-files to dodge audit Surface 2's
# workspace-walk denominator-inflation pitfall).
coverage-crate crate:
	bash scripts/coverage.sh --crate {{crate}}

# --- VM-vs-JIT differential harness (WF-0B, tools/vmjit-diff/) ---
#
# Runs every corpus program under `shape run --mode vm` AND `--mode jit`,
# diffs stdout + exit code, classifies MATCH / DIVERGED / VM_FAIL / JIT_FAIL /
# TIMEOUT, and writes tools/vmjit-diff/reports/report.{json,md}. Known
# expected divergences are pinned in tools/vmjit-diff/known-red.json.
# Measurement only — divergences get recorded, not fixed here.

# Full run: build the release binary, then diff the whole corpus.
diff-vmjit *ARGS:
	cargo build --release --bin shape
	node tools/vmjit-diff/run-diff.mjs {{ARGS}}

# Diff without building — uses $SHAPE_BIN or an existing target/release/shape.
diff-vmjit-fast *ARGS:
	node tools/vmjit-diff/run-diff.mjs {{ARGS}}

# Regenerate the committed corpus + SKIPPED.md from the book (canonical
# extractor in ../shape-web/book/book-site), the v0.3.3 acceptance programs,
# and tools/vmjit-diff/synthetic/.
diff-vmjit-corpus *ARGS:
	node tools/vmjit-diff/build-corpus.mjs {{ARGS}}
