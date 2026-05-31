# Compile-cache Phase 2 — eval-path per-test slowness diagnosis

**Date:** 2026-05-31
**Branch:** `compile-cache` (worktree `shape-compile-cache`, base `60e5ddce`)
**Goal:** localize the per-test ~4–5s cost in the shape-test integration gate and
determine whether it is the cacheable prelude type-inference (what the landed
compile-cache replays) or a different root.

## TL;DR — the team-lead's premise was WRONG (verified, not assumed)

The per-test cost is **NOT prelude type-inference re-run** and is **NOT the
thing `decide_cache_action` / `replay_resolved_interface` replays**. It is
**Pest re-parsing of the embedded stdlib `.shape` source modules**, redone from
scratch on every test because each test builds a fresh `ModuleLoader` with an
empty per-loader `ModuleCache`. The dominant single contributor is parsing
`std::core::vec` (323 lines → ~2.6s in a debug build, a Pest backtracking
cliff, not a size effect).

The interface-replay cache cannot help here: it skips *type inference* of
`.shapec` *dependencies*, but the embedded stdlib is always loaded *from
source*, and the cost is in *parsing*, which happens before any inference and
which the replay path never touches.

## Method

Temporary `eprintln!` timers gated on `SHAPE_PROBE_COMPILE=1`, plus a probe test
(`tools/shape-test/tests/annotation_targets/zzz_timing_probe.rs`). All timers
removed after diagnosis. Measured on a debug test build (the profile the gate
uses), `--test-threads=1`.

## Timing breakdown (trivial program `let x = 1\nx`)

```
ShapeEngine::new()                         ~0.3 ms     (negligible)
parse + desugar of the USER program        ~2.5 ms     (negligible)
compile_program_impl                       ~3.95 s     <-- entire per-test cost
  └ build_graph_and_stdlib_names           ~3.90 s
      ├ collect_prelude_import_paths        ~3.40 s    <-- DOMINANT
      │   └ loader.load_module("std::core::prelude")
      │       └ recursive load of stdlib tree, the cost is PARSING:
      │            parse std::core::vec        ~2.66 s  <-- single worst
      │            parse std::core::math        ~290 ms
      │            parse std::core::io          ~219 ms
      │            parse std::core::json_value   ~142 ms
      │            parse std::core::string_methods ~64 ms
      │            parse std::core::state         ~60 ms
      │            (~15 more smaller modules)
      └ build_module_graph                  ~0.50 s     (resolve-imports loop, 66 nodes)
  └ compile_with_graph_and_prelude          ~0.047 s    (Phase1 dep compile 43ms + Phase2 root 4ms)
```

Per-iteration with a FRESH engine each time (mirrors the eval helper): every
iteration pays the full ~3.9s — **the cost is NOT amortized**, because nothing
caches the parsed stdlib ASTs across `ModuleLoader` instances.

Cross-check counters during one graph build:
- `load_module` (inside `visit_module`): 33 calls, **27µs total** — cache HITs,
  cheap. (The expensive first parse happened earlier, inside
  `collect_prelude_import_paths`.)
- `resolve_module_source_kind`: 33 calls, ~0.9ms — negligible.
- `compile_module` (export collection, incl. const comptime-eval): no module
  ≥10ms — **not** the cost.
- Per-module `compile_module_from_graph` (bytecode compile + per-module type
  checking, Phase 1): 43ms total for the whole graph — **not** the cost.

## Root cause (file:line)

1. `crates/shape-vm/src/module_resolution.rs:33` —
   `collect_prelude_import_paths(loader)` is the first loader touch and forces a
   recursive from-source load of the entire stdlib tree reachable from
   `std::core::prelude`.
2. `crates/shape-vm/src/module_graph.rs:1017` —
   `collect_prelude_imports` → `loader.load_module("std::core::prelude")`.
3. `crates/shape-runtime/src/module_loader/mod.rs:656` (and the resolved-path
   twin at `:614`) — `load_module_from_source_artifact` →
   `parse_program(source)`. This Pest parse is the hot spot;
   `std::core::vec` parse ≈ 2.6s.
4. The parse is repeated per test because the `ModuleLoader` (and its
   `ModuleCache`, `crates/shape-runtime/src/module_loader/mod.rs:141`) is built
   fresh inside `BytecodeExecutor::compile_program_impl`
   (`crates/shape-vm/src/execution.rs:113`) on every `engine.execute(...)`, and
   the eval helper builds a fresh `ShapeEngine` + `BytecodeExecutor` per test
   (`tools/shape-test/src/shape_test.rs:219`, `:237`).

## Why the CLI is fast but tests are slow — same code path

The CLI `run_script` (`bin/shape-cli/src/commands/script_cmd.rs:45`) uses the
**identical** `ShapeEngine::new()` + `BytecodeExecutor` + `engine.execute()`
path. It is fast (~317ms) only because it is **one process = one program = one
parse**, in a **release** build. The integration gate runs hundreds of programs
in one process but each rebuilds the loader and re-parses the stdlib from
scratch, in a **debug** build where Pest is far slower. So the CLI's 320ms is
not evidence of a cache HIT on the compile path — it is the one-shot cost.

(The landed compile-cache's `decide_cache_action` / `replay_resolved_interface`
HIT path applies to the `#[cfg(not(test))]` embedded-prelude `.shapec` bundle
consumed by the *runtime bytecode* load, and to `.shapec` *dependencies*. It is
real and correct, but orthogonal to the per-test parse tax measured here.)

## Disposition

The root is a **cacheable load** (parsing is a pure, deterministic function of
the immutable embedded stdlib source), but it is **stdlib source re-parsing**,
not prelude type-inference. The correct fix is the prompt's explicitly-offered
acceptable alternative: a **process-level cache of the parsed stdlib module
ASTs**, shared across `ModuleLoader` instances via `OnceLock`, populated once
per test binary. It preserves byte-identical results (same source → same AST)
and test isolation (each test still gets its own engine / execution context;
only the immutable parsed `Program` ASTs are shared, read-only).

This is NOT a JIT problem and the interface-replay cache is the wrong tool;
both are explicitly out of scope per the STEP 1 gate.

## Fix landed (Phase 2)

Process-global parsed-AST memo (`parse_program_cached`,
`crates/shape-runtime/src/module_loader/mod.rs`) keyed by exact source content,
consulted at the loader's two from-source parse sites
(`load_module_from_resolved_path`, `load_module_from_source_artifact`). Commit
`db335d55` on branch `compile-cache`.

Verified (annotation_targets, 24 tests): **99.70s → 5.52s**, with the per-test
pass/fail set byte-identical (8 ok / 16 FAILED, same names — pre-existing
failures, diffed against base `60e5ddce`). `scripts/check-no-dynamic.sh` EXIT 0;
workspace `cargo check` green.
