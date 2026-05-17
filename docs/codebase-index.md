# Shape codebase index

Concept-to-location map for Shape's runtime, compiler, and tooling. Use this
as the orientation layer when starting any work that touches more than one
file. The per-domain files are the source of truth; this file is the table
of contents.

## How to use this index

- **Looking up a concept?** Check the per-domain file matching the concept's
  area. Each entry has: file path (with line numbers where useful), one-line
  role, 1-3 invariant rules, and cross-references.
- **Editing a concept?** The "Key rules / invariants" lines are the
  binding constraints. If they reference an ADR, the ADR's text is
  authoritative.
- **Adding a new concept?** Add it to the appropriate per-domain file.
  Don't duplicate the entry here — this file is the navigation layer.
- **Finding old code?** See `00-dead-code-suspects.md` for the collated
  list of dead-code suspicions. Triage before deleting; some entries are
  transitional, not dead.

## Per-domain files

| Domain | File | Concepts (count) | Lines |
|---|---|---|---|
| Compilation pipeline | [`codebase-index/01-compilation.md`](codebase-index/01-compilation.md) | 36 | 606 |
| Runtime & values | [`codebase-index/02-runtime.md`](codebase-index/02-runtime.md) | 41 | 755 |
| Frontiers (JIT, FFI, polyglot, distribution, tooling) | [`codebase-index/03-runtime-frontiers.md`](codebase-index/03-runtime-frontiers.md) | 40 | 627 |
| Dead-code suspects (collated) | [`codebase-index/00-dead-code-suspects.md`](codebase-index/00-dead-code-suspects.md) | 29 | — |

## Quick-reference — common entry points

### Source-level (parser & AST)

| What | Where |
|---|---|
| Pest grammar | `crates/shape-ast/src/shape.pest` |
| `let` / `let mut` / `var` syntax | `shape.pest:760-771` (`variable_decl`, `var_mut_modifier`, `ownership_modifier`) |
| AST types | `crates/shape-ast/src/ast/` |
| Parser entry | `crates/shape-ast/src/parser/mod.rs` |

### Type system

| What | Where |
|---|---|
| Type / SemanticType | `crates/shape-runtime/src/type_system/types/` |
| Type environment | `crates/shape-runtime/src/type_system/environment/` |
| Type inference (bidirectional) | `crates/shape-runtime/src/type_system/checking/` |
| `FieldType` | `crates/shape-runtime/src/type_schema/field_types.rs:35` |
| `NativeKind` | `crates/shape-value/src/native_kind.rs:32` |
| `HeapKind` | `crates/shape-value/src/heap_variants.rs:56` |
| Schema definitions | `crates/shape-runtime/src/type_schema/` |

### Lifetime / ownership / storage

| What | Where |
|---|---|
| `BindingStorageClass` (lattice) | `crates/shape-vm/src/type_tracking.rs:286` |
| `BindingSemantics` | `crates/shape-vm/src/type_tracking.rs:299` |
| Borrow solver | `crates/shape-vm/src/mir/solver.rs` |
| Storage planning pass | `crates/shape-vm/src/mir/storage_planning.rs` |
| Ref escape analysis | `crates/shape-vm/src/mir/lowering/mod.rs` |
| Liveness analysis | `crates/shape-vm/src/mir/liveness.rs` |
| `B0013` / `B0014` errors | `crates/shape-vm/src/mir/solver.rs` |

### Value representation

| What | Where |
|---|---|
| `HeapValue` enum | `crates/shape-value/src/heap_variants.rs:87` |
| `ValueSlot` | `crates/shape-value/src/slot.rs:15` |
| `TypedArrayData` | `crates/shape-value/src/heap_value.rs:616` |
| `TypedObject` (current) | `crates/shape-value/src/heap_variants.rs` (struct variant of HeapValue) |
| ID newtypes (`StringId`, `FunctionId`, `SchemaId`) | `crates/shape-value/src/ids.rs` |

### Bytecode & blob format

| What | Where |
|---|---|
| Bytecode compiler | `crates/shape-vm/src/compiler/` |
| Opcodes | `crates/shape-vm/src/bytecode/opcodes/` (or similar — see 01-compilation.md) |
| `FunctionBlob` + content addressing | `crates/shape-vm/src/bytecode/content_addressed.rs` |
| Linker (transitive permissions) | `crates/shape-vm/src/linker.rs` |
| Comptime evaluator | `crates/shape-vm/src/compiler/comptime.rs` (and adjacent) |

### Runtime execution

| What | Where |
|---|---|
| VM executor | `crates/shape-vm/src/executor/` |
| Drop discipline (`drop_heap`) | `crates/shape-value/src/slot.rs:107` |
| Resource limits | `crates/shape-vm/src/resource_limits.rs` |
| Method registry (PHF) | `crates/shape-vm/src/executor/objects/method_registry.rs` |
| Snapshot capture | `crates/shape-vm/src/executor/snapshot.rs:80` |

### Stdlib & capabilities

| What | Where |
|---|---|
| Stdlib modules | `crates/shape-runtime/src/stdlib/` and `stdlib_io/` |
| Capability tags | `crates/shape-runtime/src/stdlib/capability_tags.rs` |
| Marshal layer (typed exports) | `crates/shape-runtime/src/typed_module_exports.rs` |
| `ConcreteReturn` enum | `crates/shape-runtime/src/typed_module_exports.rs:49` |
| `Permission` enum (16 permissions) | `crates/shape-abi-v1/src/lib.rs:996` |
| `LanguageRuntimeVTable` | `crates/shape-abi-v1/src/lib.rs:722` |

### JIT (Cranelift)

| What | Where |
|---|---|
| JIT executor entry | `crates/shape-jit/src/` (see `lib.rs`) |
| MirToIR translator | `crates/shape-jit/src/mir_compiler/` |
| FFI value conversion | `crates/shape-jit/src/ffi/` |
| Tier thresholds (T1@100, T2@10k) | `crates/shape-vm/src/tier.rs:17-87` |
| OSR ABI (`JIT_LOCALS_CAP=256`) | `crates/shape-jit/src/osr_compiler.rs` |
| Inline cache state machine | `crates/shape-vm/src/feedback.rs:9-128` |

### Distribution & tooling

| What | Where |
|---|---|
| Wire protocol v1 | `crates/shape-wire/src/lib.rs:51` |
| Ed25519 signing | `crates/shape-runtime/src/crypto/signing.rs` |
| CLI entry | `bin/shape-cli/` |
| LSP server | `tools/shape-lsp/` |
| Test framework | `tools/shape-test/` |
| MCP server (out-of-workspace) | `../shape-mcp/` |
| `xtask` automation | `tools/xtask/` |

## ADR cross-references

Rules in this index that reference ADRs:

- **ADR-005** (`docs/adr/005-typed-slot-construction.md`): single-discriminator
  discipline, `String` exception, typed slot storage, uniform VM↔JIT slot ABI.
  ADR-005 §3 is partially superseded by ADR-006 (corrected layout).
- **ADR-006** (`docs/adr/006-value-and-memory-model.md`): canonical value &
  memory model. `let` / `let mut` / `var`, refcount-on-escape,
  `HeapValue::TypedArray(Arc<TypedArrayData>)`, LSDS error system, PVL audit,
  PES. **All Phase 1 implementation work derives from ADR-006.**

When an entry's "Key rules" cite an ADR section, the ADR text is binding;
the index summary is for orientation.

## What's known stale or in-progress

The codebase has known migration backlog. Where the current code differs
from ADR-006 or ADR-005 targets, entries note this with `(current)` vs
`(ADR-006 target)`. Major in-flight items:

- **`HeapValue` payload layout** — currently inline payloads in some
  variants; ADR-006 §2.3 target is `Arc<TypedT>` per variant. Phase 1.A.
- **`ValueSlot::from_heap`** — `#[deprecated]`-target; per-FieldType
  constructors land in Phase 1.A.
- **`var` smart-default inference** — `BindingStorageClass` exists but
  `var`-specific inference and two new variants (`SharedAtomic`,
  `SharedAtomicMut`) are Phase 1.C work.
- **B0014 runtime enforcement** — currently compile-time only; Phase 1.C
  adds the runtime upgrade path for `var`.

## Coverage gaps

The index is best-effort. Things known not (yet) covered:

- `comptime` builtins surface (partially listed; the full set is in
  `01-compilation.md` but may not be exhaustive).
- Trait-system internals at the inference level (`01-compilation.md`
  has the entry but depth varies).
- Test infrastructure and test fixtures (`tools/shape-test/` only at the
  surface level).
- `shape-viz` (visualization) — not indexed; thin wrapper around
  visualization libraries.
- `packages/` — only `xgboost/` exists; CLAUDE.md memory references
  `packages/duckdb/` and `packages/ai/` that may be stale (verify before
  acting on memory references).

When you find a missing concept, add it to the appropriate per-domain
file and update the relevant quick-reference row here.
