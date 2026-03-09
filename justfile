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
	cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests

# Tier 3: Everything — unit + deep + soak + integration (~10-15 min)
test-all:
	cargo test --workspace --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests -- --include-ignored

# Run only deep/soak tests
test-deep:
	cargo test --workspace --exclude shape-test --exclude shape-ext-python --exclude shape-ext-typescript --lib --features shape-vm/deep-tests --features shape-runtime/deep-tests --features shape-ast/deep-tests -- deep --include-ignored

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
