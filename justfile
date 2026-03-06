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

test:
	cargo test --workspace

ci-test:
	cargo test --workspace --all-targets
	cargo run -p xtask -- workspace-smoke
