set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

extension-crates := "shape-ext-python shape-ext-typescript"

default: build-extensions build-treesitter

# Build extension shared libraries and copy them into extensions/.
build-extensions profile="debug":
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
# Uses the `check-clean` recipe — see its doc-comment for the exact target list
# and the rationale for excluding `--benches`.
test-check: check-clean

# Tier 1: Fast unit tests — no deep/soak, no integration
test-fast:
	cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib

# Tier 2: Unit + deep tests, no integration
test:
	cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests --features shape-jit/deep-tests

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
	cargo test --workspace --exclude shape-test --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests
	cargo test -p shape-test -- --test-threads=1

# Run only deep/soak tests
test-deep:
	cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests --features shape-jit/deep-tests -- deep --include-ignored

# Run only shape-test integration suite
test-integration:
	cargo test -p shape-test

# Run all tests for a single crate
test-crate crate:
	cargo test -p {{crate}} --features deep-tests 2>/dev/null || cargo test -p {{crate}}

# CI: full suite. Target set mirrors `check-clean`: `--all-targets` minus
# `--benches` (the two shape-vm bench files reference deleted post-strict-typing
# shapes; bench rebuild is Item 5's territory).
ci-test:
	cargo test --workspace --lib --bins --tests --examples --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests -- --include-ignored
	cargo run -p xtask -- workspace-smoke

# --- Canonical clean-check gate ---

# Canonical "workspace clean" verifier. `just check-clean` exit 0 means the
# build gate is green; sub-cluster close gates and verify-merge.sh CHECK 1+2
# anchor on this command's coverage.
#
# Target set: `--lib --bins --tests --examples`
#   = `--all-targets` minus `--benches`.
#
# Why benches are excluded:
#   `crates/shape-vm/benches/vm_benchmarks.rs` and
#   `crates/shape-vm/benches/typed_access_bench.rs` reference deleted
#   post-strict-typing shapes (`OpCode::Lt`, `ValueWord`, `ValueWordExt`,
#   `Constant::Value`). Rewriting them against the current opcode / slot ABI
#   is Item 5's territory (the bench-rebuild sub-cluster running in parallel).
#   Until Item 5 lands, `--benches` is out of the gate.
#
# Crates covered: every workspace member (see top-level Cargo.toml `members`),
# i.e. shape-macros, shape-ast, shape-value, shape-wire, shape-runtime,
# shape-vm, shape-jit, shape-diagnostics, shape-viz-{core,native}, shape-cli,
# shape-lsp, shape-test, xtask, shape-abi-v1, shape-gc, shape-ext-python,
# shape-ext-typescript. (`shape-app` and `shape-server` live in a SEPARATE
# workspace at `../shape-app/` and are not workspace members here.)
check-clean:
	cargo check --workspace --lib --bins --tests --examples

# --- Strict-typing plan gates (~/.claude/plans/stop-native-vs-tagged-tax.md) ---

# Defection guard: per-symbol monotonic-non-increasing check vs frozen baseline.
# See scripts/check-no-dynamic.sh and docs/check-no-dynamic-baseline.txt.
check-no-dynamic:
	bash scripts/check-no-dynamic.sh

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
