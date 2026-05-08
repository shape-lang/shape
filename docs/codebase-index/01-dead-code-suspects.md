## Dead code suspects (compilation pipeline)

### `compiler::comptime_concrete::ConstantValue` and bridge fns

- **Path**: `crates/shape-vm/src/compiler/comptime_concrete.rs:90` (enum), `:188` (`type_name_constant`), `:197` (`type_annotation_to_constant_value`)
- **Why suspected**: The module carries `#![allow(dead_code)]` at module scope (line 77). The two only references in the codebase outside the file itself are TODO comments in `monomorphization/type_resolution.rs:514` / `:579`. The phase-4d migration is incomplete — `comptime.rs` still uses `ValueWord` internally, leaving `ConstantValue` exercised only by its own tests.
- **Confidence**: high (annotated dead by author until a future phase)

---

### `compiler::comptime_target::ComptimeTarget::for_expression`

- **Path**: `crates/shape-vm/src/compiler/comptime_target.rs:156`
- **Why suspected**: Marked `#[allow(dead_code)]`. No callers found via `rg for_expression\\(\\)` outside the definition.
- **Confidence**: high

---

### `mir::lowering::helpers::collect_operands` / `collect_named_operands`

- **Path**: `crates/shape-vm/src/mir/lowering/helpers.rs:113` and `:123`
- **Why suspected**: Both marked `#[allow(dead_code)]`. The doc-comment example calls them out as "consider using" patterns rather than active code paths.
- **Confidence**: high

---

### `MirConstant::StringId(u32)` variant

- **Path**: `crates/shape-vm/src/mir/types.rs:204`
- **Why suspected**: Source comment marks it as "legacy — prefer Str for new code" (line 203). Live uses are in matchers/displays only; the only `MirConstant::StringId(_)` constructor sites are inside tests (`return_ownership.rs:817`, `storage_planning.rs:1280`). The lowering pass produces `MirConstant::Str(String)` instead.
- **Confidence**: medium (variant kept for back-compat; production lowering does not emit it)

---

### `OptimizationMetric::Custom` and `Item::Optimize`

- **Path**: `crates/shape-ast/src/ast/program.rs:166` (enum), `:65` (`Item::Optimize`)
- **Why suspected**: Constructed only by parser (`parser/mod.rs:198`) and visited only by `visitor.rs:360`. The compiler pattern in `functions.rs:844` and `:890` reaches `Item::Optimize` only via span-only catch-all arms — no semantic codegen for it.
- **Confidence**: medium (Phase 3 placeholder per AST doc; not wired end-to-end)

---

### `type_system::environment::registry::BlanketImplEntry`

- **Path**: `crates/shape-runtime/src/type_system/environment/registry.rs:94`
- **Why suspected**: Already annotated `#[allow(dead_code)]` at line 93. The struct has only one read site (`:420`) and one push site (`:377`) — but several of its fields go unread by the consumer.
- **Confidence**: low (struct is used, but multiple fields may be dead)

---

### Pretty-printer / unparser surface

- **Path**: not found in `crates/shape-ast/src`
- **Why suspected**: No `PrettyPrinter`, `pretty_print`, `unparse`, or AST-to-source function exists in the AST crate. `Display` impls on individual MIR types (e.g. `Place`, `Operand`, `MirConstant`) cover diagnostics but no whole-AST round-trip.
- **Confidence**: low (this is "intentionally absent", not dead, but worth flagging to the user since the index template asked for it)

---

### `shape-types/` empty crate skeleton

- **Path**: `crates/shape-types/` (only `data/` subdir, no `src/`)
- **Why suspected**: The crate is referenced in CLAUDE.md as "Type system definitions, type inference types" but has no `src/` directory in the working tree. All type-system code actually lives under `crates/shape-runtime/src/type_system/` and `crates/shape-runtime/src/type_schema/`.
- **Confidence**: medium (may be a placeholder for a planned move; currently has no compilable Rust)
