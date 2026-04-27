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

# Tier 0: Compile all tests without running them (~5-8s)
test-check:
	cargo check --workspace --tests --all-targets

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

# CI: full suite
ci-test:
	cargo test --workspace --all-targets --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests -- --include-ignored
	cargo run -p xtask -- workspace-smoke
