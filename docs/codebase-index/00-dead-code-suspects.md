# Dead-code suspects (collated)

Best-effort scan from the three-agent indexing pass on 2026-05-08. Each
entry is a *suspect*; confirmation requires `cargo check` + a
cross-reference pass before deleting. Some entries are transitional
(known migration backlog), not dead — flagged where applicable.

29 suspects total: **7 high-confidence**, **11 medium-confidence**,
**11 low-confidence**.

## High-confidence (act first)

These are either factually wrong (bug), author-annotated dead, or have
zero callers in the workspace.

### `HeapKind::MAX_VARIANT` — factual bug

- **Path**: `crates/shape-value/src/heap_header.rs:160`.
- **What**: Constant set to `HeapKind::Char` but the actual maximum after
  the Stage C `HashMap` addition (2026-05-07) is `HeapKind::HashMap`
  (ordinal 17 vs `Char` 16). The roundtrip test (`heap_header.rs:319`)
  iterates `0..=MAX_VARIANT` and silently skips `HashMap`.
  `from_u16(17)` returns `None` for a valid variant.
- **Action**: small clean fix. Update the constant, rerun the roundtrip
  test, confirm 18 variants exercised. Good candidate for a
  confidence-building first commit before Phase 1.A starts.

### `crates/shape-jit/src/ffi/window.rs` — 3-line tombstone

- **Path**: `crates/shape-jit/src/ffi/window.rs:1-3`; re-exported via
  `pub use window::*;` in `ffi/mod.rs:58`.
- **What**: File body is a 3-line tombstone comment ("v2-boundary: Window
  FFI functions deleted — no callers from MirToIR or executor. All
  functions were only registered in ffi_symbols (dead wiring).").
  Re-export exposes nothing.
- **Action**: delete the file and the `pub use` line.

### `JITSignalBuilder` + `signal_builder.rs`

- **Path**: `crates/shape-jit/src/ffi/call_method/signal_builder.rs:1-67`;
  struct at `crates/shape-jit/src/context.rs:434-468`.
- **What**: `JITSignalBuilder::new` and `::box_builder` have zero callers
  in `shape-jit`, `shape-vm`, or anywhere in the workspace. The
  method-dispatch handler `is_heap_kind(receiver_bits, HK_JIT_SIGNAL_BUILDER)`
  can never fire. Likely orphaned from a finance-removal sweep.
- **Action**: delete after confirming no usage in extensions.

### `compiler::comptime_concrete::ConstantValue` — author-annotated dead

- **Path**: `crates/shape-vm/src/compiler/comptime_concrete.rs:90`
  (enum), `:188` (`type_name_constant`), `:197`
  (`type_annotation_to_constant_value`).
- **What**: Module carries `#![allow(dead_code)]` at module scope (line
  77). Two only references are TODO comments in
  `monomorphization/type_resolution.rs:514, :579`. Phase-4d migration is
  incomplete — `comptime.rs` still uses `ValueWord` internally, leaving
  `ConstantValue` exercised only by its own tests.
- **Action**: either land phase-4d wiring or delete. ADR-006's `var`
  inference may need `ConstantValue` as a building block — verify before
  deletion.

### `compiler::comptime_target::ComptimeTarget::for_expression`

- **Path**: `crates/shape-vm/src/compiler/comptime_target.rs:156`.
- **What**: Marked `#[allow(dead_code)]`. No callers found.
- **Action**: delete unless it's the entry point for a planned feature.

### `mir::lowering::helpers::collect_operands` / `collect_named_operands`

- **Path**: `crates/shape-vm/src/mir/lowering/helpers.rs:113, :123`.
- **What**: Both marked `#[allow(dead_code)]`. Doc-comment example calls
  them out as "consider using" patterns.
- **Action**: delete or wire up.

### `ValueSlot::from_heap` (transitional, not dead)

- **Path**: `crates/shape-value/src/slot.rs:59`.
- **What**: ADR-005 §3 / ADR-006 §2.4 mark as `#[deprecated]`-target.
  Per-FieldType constructors will replace it. ~10 caller sites still
  found (`type_schema/mod.rs:226, :234`, `stdlib/json.rs:126,138,151,
  226,233,239,246`, `state_builtins/core.rs:591`).
- **Action**: don't delete now; track as Phase 1.B migration target.
  When all callers migrated, delete.

## Medium-confidence (triage post-Phase-1)

These are likely transitional, may be dead, or are legacy paths whose
status is ambiguous.

### `MirConstant::StringId(u32)` variant

- **Path**: `crates/shape-vm/src/mir/types.rs:204`.
- **What**: Comment marks it "legacy — prefer Str for new code". Live
  uses are matchers/displays only; constructor sites only in tests
  (`return_ownership.rs:817`, `storage_planning.rs:1280`). Production
  lowering produces `MirConstant::Str(String)`.
- **Action**: investigate whether kept for back-compat reasons or truly
  dead.

### `OptimizationMetric::Custom` and `Item::Optimize`

- **Path**: `crates/shape-ast/src/ast/program.rs:166, :65`.
- **What**: Constructed by parser, visited by visitor, but compiler
  pattern reaches `Item::Optimize` only via span-only catch-all arms —
  no semantic codegen. Phase 3 placeholder.
- **Action**: keep if Phase 3 is real; delete if abandoned.

### `crates/shape-types/` empty crate skeleton

- **Path**: `crates/shape-types/` (only `data/` subdir, no `src/`).
- **What**: Referenced in CLAUDE.md "Crate Map" but has no `src/` dir.
  All type-system code lives at `crates/shape-runtime/src/type_system/`
  and `type_schema/`.
- **Action**: either populate the crate (planned move?) or remove from
  CLAUDE.md and the workspace. **CLAUDE.md will be corrected in this
  pass.**

### `numeric_compiler.rs` (legacy JIT path)

- **Path**: `crates/shape-jit/src/numeric_compiler.rs:1-105`; called only
  from `compiler/program.rs:146`.
- **What**: Module header calls itself "Legacy numeric-only JIT compiler"
  pre-MirToIR. The whole architecture moved to MirToIR.
- **Action**: candidate for deletion after confirming `compiler/program.rs:146`
  has no live caller.

### `crates/shape-jit/src/core.rs` (1418 lines, legacy)

- **Path**: `crates/shape-jit/src/core.rs`; private `mod core;` in
  `lib.rs:22`.
- **What**: Module doc-comment is the pre-v2 finance-strategy target.
  Many gated `test_jit_width_aware_*` tests live here. Per CLAUDE.md,
  described as "Legacy re-exports and tests".
- **Action**: candidate for deletion or restructuring. Check whether
  any non-test code still imports from it.

### `crates/shape-jit/src/REFACTOR_PLAN.md` and `V2_MIGRATION_STATUS.md`

- **Path**: `crates/shape-jit/src/REFACTOR_PLAN.md` (179 lines);
  `crates/shape-jit/src/compiler/V2_MIGRATION_STATUS.md`.
- **What**: In-tree refactor plans without enforcement. Historical, not
  active.
- **Action**: move to `docs/archive/` or delete.

### `crates/shape-jit/src/ffi/mod.rs:11-12, :42-43` DELETED comments

- **Path**: `crates/shape-jit/src/ffi/mod.rs:11, :12, :42, :43`.
- **What**: Tombstone comments referencing deleted finance-specific JIT
  modules. Pure breadcrumbs.
- **Action**: delete the comments.

### `HeapValue::BigInt(i64)` — placeholder name

- **Path**: `crates/shape-value/src/heap_variants.rs:104`.
- **What**: Variant payload is a plain `i64` despite the name "BigInt".
  No arbitrary-precision implementation. Either placeholder for future
  or fully redundant given `HeapValue::NativeScalar::I64` exists.
- **Action**: rename or remove. ADR-006 review when arbitrary-precision
  is needed.

### `TemporalData::TimeReference` / `DateTimeExpr` / `DataDateTimeRef`

- **Path**: `crates/shape-value/src/heap_value.rs:855-857`.
- **What**: Boxed AST nodes carried as runtime values. Single push site
  each in `stack_ops/mod.rs`, only Debug formatting in `printing.rs`,
  no method dispatch / arithmetic / equality. Niche opcode path.
- **Action**: investigate user-visible surface.

### `TypedArrayData::FloatSlice` — N7 deferred

- **Path**: `crates/shape-runtime/src/json_value.rs:219-220`.
- **What**: Serialization explicitly errors with "policy not yet decided
  (N7 architectural-choice deferral)". Variant in deferred-question
  state.
- **Action**: rule on policy or drop variant.

### `clone_heap` deep-clone path

- **Path**: `crates/shape-value/src/slot.rs:135`.
- **What**: Deep-clones via `(*ptr).clone()` then re-Boxes. Becomes a
  single Arc bump after ADR-006 §2.3 lands (typed `Arc<T>` payloads).
- **Action**: rewrite as part of Phase 1.A.

## Low-confidence (note and move on)

Includes "intentionally absent" gaps and items whose status is
uncertain without deeper investigation.

- **`type_system::environment::registry::BlanketImplEntry`** — annotated
  dead; struct used but multiple fields may be unread
  (`registry.rs:94`).
- **Pretty-printer / unparser** — not found in `crates/shape-ast/src`.
  Likely intentionally absent; flagged for completeness.
- **`HeapValue::Future(u64)` clone-into-typed-array path** — exists, but
  consumers are stubs/printing/JIT-FFI only. Likely live as
  `op_spawn_task` result-type.
- **`TableViewData::ColumnRef` / `IndexedTable`** — sparse method
  registry coverage; whether all 4 variants needed is unclear.
- **`HeapValue::NativeView` + `NativeViewData`** — pointer-backed
  zero-copy view; unclear if still active post-ADR-006 §4.4.
- **`DebugVMState`** — two-field stub at `executor/mod.rs:458-464`;
  may be unreferenced if integrated debugger moved on.
- **`LAZY_METHODS` / `MUTEX_METHODS`** — concurrency PHF maps;
  whether end-to-end exposed is unclear.
- **`IoResource::Custom`** — type-erased branch; whether any current
  call site constructs it is uncertain.
- **`HeapKind::Closure` ordinal-vs-name skew** — works (regression test
  exists), but rename candidate.
- **Width-aware JIT tests** (`test_jit_width_aware_*`) — ~23 ignored
  tests; may have drifted from current execution path.
- **Finance-specific FFI symbol tombstones** in
  `ffi_symbols/data_access/mod.rs:12-18`.
- **`bin/shape-cli/src/commands/wire_serve_cmd.rs`** vs `serve_cmd.rs`
  — both exist with different protocols; `serve` is the modern
  wire-msgpack-over-QUIC-with-auth path that the MCP spawns.
  `wire-serve` may be superseded.
- **`jit_matrix.rs` `JitMatrix`** — thin matrix bridge; v2
  `Arc<MatrixData>` direct path may make redundant.

## Recommended triage order

1. **Quick wins** (high-confidence, mechanical, ~1-2 hours total):
   `HeapKind::MAX_VARIANT` bug fix + `ffi/window.rs` tombstone deletion
   + `JITSignalBuilder` deletion + `comptime_target::for_expression`
   + `mir::lowering::helpers::collect_*` helpers. **Good first commits
   before Phase 1.A.**
2. **CLAUDE.md sync**: update stale path references (`shape-types/`
   crate map row, "Bytecode compiler" Key File Locations, "Type
   environment" path). Done as part of this index commit.
3. **Phase 1.A migration backlog**: `from_heap`, `clone_heap`, and the
   `HeapValue` payload layout work — these *are* Phase 1.A's scope, not
   side cleanup.
4. **Post-Phase 1**: medium-confidence triage. Run a dedicated cleanup
   sprint after Phase 1.A lands; the migration will resolve some
   ambiguities (e.g., `TypedArrayData::FloatSlice` policy).
5. **Low-confidence sweep**: only after Phase 1.A and the post-Phase-1
   cleanup. Some of these will resolve naturally; others may stay as
   "we don't know if it's used."

## How to extend this list

When working on Phase 1+ and you notice a function with no callers,
test-only callers, or a comment like "deleted" / "legacy" / "TODO":
add an entry here at the matching confidence tier. Keep it brief
(path + what + action). The triage pass collates and acts.
