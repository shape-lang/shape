# Strictly-Typed Baseline (Phase 0)

Captured: 2026-05-06 at tag `pre-strictly-typed` (HEAD = `fa45c00`).

Reference plan: `~/.claude/plans/stop-native-vs-tagged-tax.md`.

This is the measurement snapshot before the Phase 1 bulldozer. Every number below is a target to reach 0 (or a known-residual to delete) by Phase 5.

## ValueWord machinery — actual size

The plan's estimate of "~2,650 LoC for `value_word.rs`" was wrong. The machinery is split across multiple files in `crates/shape-value/src/`:

| File | LoC | Role |
|---|---:|---|
| `value_word.rs` | 192 | Type def |
| `value_word_ext.rs` | **2,497** | Extension methods (the heavy module) |
| `value_word_drop.rs` | 581 | Drop / refcount logic |
| `value_bits.rs` | 567 | The renamed "ValueBits shim" from the prior defection |
| `tag_bits.rs` | 408 | Tag constants |
| **Total** | **~4,245** | All to delete in Phase 1 |

`heap_value.rs` is 2,126 LoC but stays — it is the typed `HeapValue` sum type with typed pointers, not part of the dynamic dispatch machinery. (1,634 `HeapValue` references workspace-wide remain valid.)

**Net deletion estimate revised**: ~4,200 LoC (machinery) + ~150 boundary-helper call sites (below) = ~4,500 LoC removed across the workspace.

## ValueWord references workspace-wide

| Crate | Files touching `ValueWord` |
|---|---:|
| `shape-vm` | 200 |
| `shape-runtime` | 86 |
| `shape-jit` | 30 |
| `shape-value` | 27 |
| `shape-gc` | 3 |
| `extensions/python` | **0** |
| `extensions/typescript` | **0** |
| **Total grep hits** | **8,154** |

Polyglot ABI revision (Phase 5 in the plan) is much smaller than estimated: **extensions do not touch `ValueWord` at all**. The polyglot ABI flows through trait methods on `LanguageRuntimeVTable` rather than direct `ValueWord` plumbing. Phase 5 mostly verifies typed-return signatures.

## Pattern A/B/C/D inventory

These are the runtime decode bridges to delete.

| Pattern | Symbol | Call sites |
|---|---|---:|
| A | `last_program_return_kind` field/stamp | 18 |
| A | `typed_return_with_kind` helper | 13 |
| B | `synthesize_value_word_from_raw` | 42 |
| B | `is_tagged()` in `crates/shape-vm/src/executor/` | 83 |
| C | `capture_as_value` | 62 |
| C | `rebox_native_bits` | 14 |
| D | `normalize_persisted_for_slot` | 5 |
| — | `exec_arithmetic_dynamic_fallback` / `exec_comparison_dynamic_fallback` | 9 |
| — | `as_i64_arg` / `as_f64_arg` / `as_bool_arg` decode helpers | 10 |
| — | `last_emitted_native_kind` (sparse tracker — replace with `prove_native_kind`) | 17 |
| — | `nan_box` / `NanBox` / `NanTag` residuals | 45 |
| **Total bridge call sites** | | **~318** |

Phase 4 gate: every entry above → 0.

## Generic opcodes — mostly already deleted

Generic kind-less opcodes (`Add`, `Sub`, `Mul`, `Div`, `Mod`, `Pow`, `Neg`, `Not`, `Lt`, `Lte`, `Gt`, `Gte`, `Eq`, `Neq`) are largely gone on `main` already, per the existing audit docs:

- `crates/shape-vm/src/compiler/V2_AUDIT_WAVE2.md`
- `crates/shape-vm/src/compiler/V2_TYPED_OPCODE_STATUS.md`

Surviving generic-opcode emit sites in `crates/shape-vm/src/compiler/`: **16**, mostly `OpCode::Not` (logical bool-only) and `OpCode::Pow`.

Phase 1 bulldozer scope on this front: replace `Not` with `NotBool`, `Pow` with `PowI64`/`PowF64`. Small.

## Type-system completeness

| Metric | Count |
|---|---:|
| `Type::Variable` references in `shape-vm/src/compiler/` + `shape-runtime/src/compiler/` | **5** |
| `SlotKind::Dynamic` / `SlotKind::Unknown` references | 76 |

Phase 3 gate: `Type::Variable` reaching emission → 0; `SlotKind::Dynamic` and `SlotKind::Unknown` variants deleted from the enum (Phase 1).

The 5 `Type::Variable` reference sites in compiler code are the inference-engine internals — they're load-bearing during inference itself. The check is: does any `Type::Variable` *survive* inference into the bytecode emission path. Phase 1 will surface this directly when emit code asks `prove_native_kind` and gets `ProofGap`.

## ValueWord touches outside the typed runtime path

| Subsystem | `ValueWord` refs |
|---|---:|
| `shape-jit` (Cranelift codegen via MirToIR) | 32 across files; some load-bearing for FFI boundary |
| `shape-runtime` (bytecode compiler, stdlib) | 86 across files |
| `shape-value/src/heap_value.rs` (typed sum type) | enum payloads where `ValueWord` is the dynamic value — to be replaced by per-variant typed payloads |

Phase 1 deletion expects to break ~5–10 different shape-jit translation sites; expect Phase 2 reconstruction work concentrated in `shape-jit/src/mir_compiler/conversions.rs` (the file the original plan flagged as "FFI-boundary bridge — V5.7 follow-up").

## Test status

`cargo test -p shape-vm --lib` at `pre-strictly-typed`:
- 2284 pass / 4 fail / 8 ignored (1 of the 4 is the known parallel-isolation flake `test_e4_real_program_fallback_baseline`).

These test counts will be invalidated by Phase 1 (workspace won't compile). Tracking will resume at Phase 2 gate.

## Stale debug instrumentation flagged for cleanup

The working tree on `main` had three uncommitted `eprintln!`-instrumentation hunks left from W-series exploration. Not committed at the Phase 0 boundary. Should be reverted before Phase 1 starts:

- `crates/shape-vm/src/compiler/helpers.rs` — `[DBG W1]` eprintln in `infer_top_level_return_kind_from_item`
- `crates/shape-vm/src/compiler/helpers_binding.rs` — `[DBG W1]` eprintln in `emit_return_value_with_ownership`
- `crates/shape-vm/src/compiler/mod.rs` — `function_inferred_return_kinds` field added (W1 comment refs unmerged code)

## Phase 1 scope summary

| Target | Estimated work |
|---|---|
| Delete `value_word*.rs` + `value_bits.rs` + `tag_bits.rs` | ~4,245 LoC removed |
| Delete generic opcodes (`Not`, `Pow` survivors) | ~16 emit sites updated |
| Delete Pattern A/B/C/D + dynamic-fallback bridges | ~318 call sites removed |
| Delete `SlotKind::Dynamic` / `SlotKind::Unknown` variants | 76 use sites flagged for fix-or-delete |
| Add `ProofGap` private-constructor type | ~50 LoC new in `compiler/type_tracking.rs` |

Workspace will not compile after Phase 1. Compile-error count is the Phase 2 input metric. Tag the broken state as `bulldozer-complete`.

## Verification gates (recap)

| Phase | Gate |
|---|---|
| 0 (this) | Baseline captured. Tag `pre-strictly-typed` placed. |
| 1 | Workspace does not compile (expected). All forbidden symbols deleted from source. |
| 2 | Workspace compiles. Suite runs. |
| 3 | `Type::Variable` reaching emission → 0. Suite passes. |
| 4 | `prove_native_kind` panics → 0. |
| 5 | `just check-no-dynamic` returns clean. Sentinel test green. Benchmarks no-slowdown. |
