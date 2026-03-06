# P0-P11 Extension/Capability + VMValue Retirement Roadmap

Status: Active
Owner: runtime/language core
Scope: `shape-runtime`, `shape-vm`, `shape-cli`, `shape-lsp`, extension/plugin ecosystem

## Tracking

- Canonical execution tracker: `tasks/p0-p11-todo.md`
- VMValue inventory/guard tooling: `cargo xtask vmvalue`
- VMValue baseline allowlist: `shape/tasks/vmvalue_allowlist.txt`
- VMValue category baseline: `shape/tasks/vmvalue_allowlist_categories.tsv`
- P8 detailed stream plan: `shape/docs/vision/p8-stream-capability-plan.md`
- Book alignment baseline for this track: `shape/docs/book/src/advanced/extending.md`, `shape/docs/book/src/advanced/projects.md`, `shape/docs/book/src/advanced/frontmatter.md`

Current snapshot (2026-02-17):

- VMValue references: `1336`
- VMValue files: `71`
- Per-crate VMValue references:
  - `shape-vm`: `640`
  - `shape-value`: `375`
  - `shape-runtime`: `318`
  - `shape-jit`: `2`
- Location categories (files):
  - `hot_path`: `37`
  - `boundary`: `3`
  - `support_or_legacy`: `22`
  - `test_or_bench`: `9`

Phase status:

| Phase | Status | Notes |
| --- | --- | --- |
| P0 | completed | Inventory + allowlist + category baseline are scripted and versioned |
| P1 | completed | Guard script is wired into workspace smoke command and contributor allowlist workflow is documented |
| P2 | completed | Static connector wiring removed; CLI/runtime paths now plugin-dispatch only with regression coverage for no fallback |
| P3 | completed | Parser/config SSoT in runtime; script/REPL/TUI/LSP use unified declared-module registration and shared config parsing (including dependency resolver manifest parsing via shared project parser) |
| P4 | completed | Base `shape.module` ABI formalized; runtime binds module namespaces/functions via plugin capability metadata |
| P5 | completed | DuckDB migrated to ABI `shape.module` plugin path (`.so`), with contract tests and CLI runtime load validation |
| P6 | completed | Postgres migrated to ABI `shape.module` plugin path (`.so`), with contract tests and CLI runtime load validation |
| P7 | completed | Unified lock/artifact helpers in runtime, connector lock wrappers removed, stale/invalid diagnostics covered |
| P8 | pending | Streaming capability contracts |
| P9 | pending | Output/event capability hardening |
| P10 | pending | Full VMValue purge + allowlist drain |
| P11 | pending | Cleanup + conformance suite |

## Goals

1. One unified extension mechanism: host-loaded `.so` modules declared in `shape.toml` `[[extensions]]`.
2. No hardcoded host logic for duckdb/postgres (or any specific connector).
3. Mandatory base plugin contract (`shape.module`) + optional semantic capabilities (`shape.datasource`, `shape.output_sink`, etc.).
4. Systematic VMValue retirement in favor of `NanBoxed` + `shape-wire` boundaries.
5. Single-source-of-truth behavior across CLI, runtime, compiler, and LSP.

## Architecture Principles

- Single loader path for extensions.
- C ABI stability (`shape_abi_v1`) over Rust ABI coupling.
- `shape-wire` as the cross-boundary payload model.
- Capability layering:
  - Base: `shape.module`
  - Optional: `shape.datasource`, `shape.output_sink`, `shape.compute`, `shape.model`, ...
- No duplicated parsing/manifest logic (shape.toml parsing shared across runtime + tooling).

## Phase Plan

### P0. Baseline and inventory

- Produce VMValue usage inventory across workspace.
- Tag references by location category (hot path / boundary / legacy/test).
- Establish measurable baseline count and file list.

Acceptance:
- Automated inventory command exists.
- Baseline allowlist is versioned.

### P1. Stop-the-bleeding guard

- Add guard to block new VMValue references outside approved baseline.
- Keep temporary allowlist explicit and reviewable.

Acceptance:
- Guard fails when VMValue appears in a non-allowlisted source file.

### P2. Plugin-only extension loading

- Remove built-in duckdb/postgres runtime registration path.
- Runtime/CLI only load extension modules from project/frontmatter `[[extensions]]`, global config, or CLI plugin flags.

Acceptance:
- No duckdb/postgres host registration path remains.
- Scripts run using `.so` extensions only.

### P3. Unified module symbol registration

- Ensure compiler/LSP module knowledge derives from same sources as runtime extension loading.
- Eliminate feature-gated split-brain module registration.

Acceptance:
- LSP and runtime agree on extension module availability.
- Regression tests cover `shape.toml` extension globals.

### P4. `shape.module` capability contract

- Introduce mandatory base contract for exported module metadata + invocation.
- Runtime creates module namespace/functions from contract, not hardcoded host bindings.

Acceptance:
- At least one plugin exposes module exports exclusively via `shape.module`.

### P5. DuckDB migration to plugin contract

- DuckDB extension provides module exports and datasource semantics via ABI capability contracts.
- Remove duckdb-specific legacy host paths.

Acceptance:
- DuckDB examples execute without built-in crate registration.

### P6. Postgres migration to plugin contract

- Same as P5 for postgres.

Acceptance:
- Postgres examples execute without built-in crate registration.

### P7. Lock/artifact unification for external compile-time inputs

- Keep external schema artifacts in unified `shape.lock` flows.
- No connector-specific lock side systems.

Acceptance:
- One lock artifact pipeline for external inputs.

### P8. Streaming capability design + implementation pass

- Treat streaming as datasource mode with explicit contracts (subscribe/unsubscribe/backpressure/checkpoint).
- Detailed patch order and acceptance criteria are tracked in `shape/docs/vision/p8-stream-capability-plan.md`.

Acceptance:
- Stream replay + resume behavior tested.

### P9. Output/event capability hardening

- Stabilize output sink envelope, retry/idempotency semantics, and routing.
- Add typed rendering metadata contract:
  - f-string format spec lowered to typed AST/enums (no stringly runtime format options).
  - renderer-agnostic fragments routed through `OutputAdapter` backends (ANSI/HTML/plain).
  - typed table formatting configuration (alignment/color/precision via enums/structs).
  - AST/span-driven parser integration so expression/LSP support remains intact.

Acceptance:
- Sink behavior covered by contract tests.
- Typed formatting behavior covered by parser/type/LSP/runtime adapter regression tests.

### P10. Full VMValue purge

- Remove compatibility shims after all active paths are NanBoxed/wire-native.
- Delete residual VMValue APIs from runtime hot paths and extension boundaries.

Acceptance:
- Guard allowlist is empty (or compatibility-only module that is then removed).

### P11. Final cleanup + conformance

- Remove dead/legacy code paths.
- Add conformance tests for capability contracts and module-loading single-source-of-truth.

Acceptance:
- Clean architecture invariants documented and enforced by tests.

## Non-goals for this track

- Immediate MLIR implementation itself.
- Full query IR redesign in this patch series.

Those remain downstream, but this roadmap is required substrate for those tiers.
