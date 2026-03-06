# Repository Guidelines

## Project Structure & Module Organization
The `fchart` workspace bundles GPU chart renderer crates. `fchart-core/` exposes the engine and shared domain types; target-specific frontends live in `fchart-cli/` and `fchart-native/`. Shared assets and fixtures are in `data/` and `examples/`. Tests follow Rust conventions: unit suites sit beside modules in each crate’s `src/`, while cross-crate scenarios reside in `tests/`. Keep rendered artifacts (e.g., `chart.png`) out of commits unless they capture deliberate updates.

## Build, Test, and Development Commands
- `cargo build --workspace`: compile every crate in release order to catch cross-crate breakage.
- `cargo check -p fchart-core`: fast iteration on the core engine without producing binaries.
- `cargo run --bin fchart-cli -- demo`: render the reference chart with bundled data; use this after every visual change.
- `./run_test.sh`: run the smoke suite used in CI; execute it before pushing.
- `cargo run --bin fchart-cli -- load -s ES1! -d 30`: handy manual validation using imported sample data.

## Coding Style & Naming Conventions
Target Rust 2021 with 4-space indentation. Keep module files in `snake_case`, types in `PascalCase`, and constants as `SCREAMING_SNAKE_CASE`. Prefer explicit modules over glob imports. Run `cargo fmt` and `cargo clippy --workspace --all-targets --all-features` before committing; treat warnings as errors. Add concise Rustdoc for non-obvious public APIs and for modules that bridge to GPU backends.

## Testing Guidelines
Exercise new logic with unit tests colocated in `src/<module>.rs`. Use descriptive `test_<behavior>` names and cover error branches. For end-to-end rendering or data-flow checks, add integration tests under `*/tests/`. Validate rendering and data-pipeline changes with `cargo test --workspace`; add targeted runs like `cargo test -p fchart-core geometry` when iterating on specific systems. Every bug fix should ship with a regression test mirroring the original failure.

## Commit & Pull Request Guidelines
Adopt Conventional Commits (`feat:`, `fix:`, `refactor:`). Scope each commit to one logical change and avoid mixing formatting with behavior. Pull requests must describe the change, outline architecture impacts, link relevant issues, and attach screenshots or CLI output for visual updates. Confirm `cargo fmt`, `cargo clippy`, `cargo test --workspace`, and `./run_test.sh` pass before requesting review.

## Data & Configuration Notes
Rely on the checked-in sample DuckDB/CSV fixtures for local testing; never commit proprietary data. Store secrets in environment variables or `.env` (ignored by Git) and document any new keys in PRs. Keep large exports like `market_data.duckdb` out of version control unless intentionally refreshed.
