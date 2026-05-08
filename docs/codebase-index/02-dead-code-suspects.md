# Dead code suspects (runtime + values)

Best-effort scan from the runtime/value-model concept index pass. Each entry
is a *suspect* — confirmation requires running `cargo check` and a
cross-reference pass before deleting anything.

## Dead code suspects (runtime + values)

### `HeapKind::MAX_VARIANT`

- **Path**: `crates/shape-value/src/heap_header.rs:160`
- **Why suspected**: Constant set to `HeapKind::Char` but the actual maximum after the Stage C `HashMap` addition (2026-05-07) is `HeapKind::HashMap` (ordinal 17, vs. `Char` at 16). The roundtrip test (`heap_header.rs:319`) only iterates `0..=MAX_VARIANT`, so it silently skips the `HashMap` ordinal. `from_u16(17)` returns `None` for what is in fact a valid variant.
- **Confidence**: high (factual wrong-value bug — comment "IMPORTANT: Update this when adding new HeapKind variants" at `:159` was not honored).

### `HeapValue::Future(u64)` clone-into-typed-array path

- **Path**: `crates/shape-value/src/heap_value.rs:981`, `:1167`, `:1273`, `:1095`
- **Why suspected**: `HeapValue::Future` exists, but the only consumers found are: serialization stubs (`json_value.rs:114` errors out, `wire_conversion.rs:117` formats as a string), printing (`printing.rs:265`), structural eq, and the JIT FFI conversion (`shape-jit/.../conversion.rs:599`). The actual async machinery uses `WaitType::Future { id }` directly. The `HeapValue::Future` carrier may be load-bearing only as the result-type of `op_spawn_task` — verifying whether the rest of the runtime relies on it being a `HeapValue` (as opposed to a tagged scalar) would be useful.
- **Confidence**: low (likely live; called out for re-audit).

### `HeapValue::BigInt(i64)` placeholder

- **Path**: `crates/shape-value/src/heap_variants.rs:104`
- **Why suspected**: Variant payload is a plain `i64` despite the name "BigInt". No arbitrary-precision implementation; treated as a scalar everywhere. Either a placeholder for future work or fully redundant given `HeapValue::NativeScalar::I64` already covers the i64 case.
- **Confidence**: medium.

### `TemporalData::TimeReference` / `DateTimeExpr` / `DataDateTimeRef`

- **Path**: `crates/shape-value/src/heap_value.rs:855-857`
- **Why suspected**: Boxed AST nodes (`shape_ast::ast::TimeReference`, `DateTimeExpr`, `DataDateTimeRef`) carried as runtime values. The only consumers found are: `stack_ops/mod.rs:291-297` (single push site each), `printing.rs:285-287` (Debug formatting), and `window_join.rs:23` (DateTimeExpr only). No method dispatch, no arithmetic, no equality coverage. Suggests these wrap AST that never actually flows to user-visible methods.
- **Confidence**: medium — likely a niche opcode path, not user-callable surface.

### `TableViewData::ColumnRef` / `IndexedTable` (consumer surface gap)

- **Path**: `crates/shape-value/src/heap_value.rs:908-918`
- **Why suspected**: Variants exist, are constructed by some opcode, but consumer methods (`COLUMN_METHODS`, `INDEXED_TABLE_METHODS`) are sparse (10 + 6 entries respectively in `method_registry.rs`). Whether all four `TableViewData` variants are still needed end-to-end (or `TypedTable + RowView` cover the active surface) is worth a pass.
- **Confidence**: low.

### `HeapValue::NativeView` + `NativeViewData`

- **Path**: `crates/shape-value/src/heap_value.rs:268-273`, `:1101-1107`
- **Why suspected**: Pointer-backed zero-copy view into native memory, used by the C ABI native-views path. With ADR-006's `extern C` design (§4.4) using `repr(C)` discipline at the boundary, it is unclear whether `NativeView` is still on the active path or only used by an early prototype. No method registry entry (`method_registry.rs` has no `NATIVE_VIEW_METHODS`).
- **Confidence**: low.

### `TypedArrayData::FloatSlice` serialization

- **Path**: `crates/shape-runtime/src/json_value.rs:219-220`
- **Why suspected**: `FloatSlice` serialization explicitly errors with "policy not yet decided (N7 architectural-choice deferral)". If the decision is "drop entirely", removing `FloatSlice` would simplify Display, equality, the typed-array ops, and the wire path. If the decision is "include", code needs to land. Either way the variant is in deferred-question state.
- **Confidence**: medium.

### `ValueSlot::from_heap` (transitional API)

- **Path**: `crates/shape-value/src/slot.rs:59`
- **Why suspected**: ADR-005 §3 / ADR-006 §2.4 mark this as `#[deprecated]`-target; per-FieldType constructors (`from_string_arc`, `from_typed_array`, `from_typed_object`) should replace it. ~10 caller sites still found (`type_schema/mod.rs:226, :234`, `stdlib/json.rs:126, :138, :151, :226, :233, :239, :246`, `state_builtins/core.rs:591`). It's a known transitional, not strictly dead, but the migration is open and the old constructor is the gravity well.
- **Confidence**: high (known migration backlog, not pure dead code).

### `clone_heap` deep-clone path

- **Path**: `crates/shape-value/src/slot.rs:135`
- **Why suspected**: Deep-clones via `(*ptr).clone()` then re-Boxes. Once ADR-006 §2.3 lands (typed `Arc<T>` payloads), this becomes a single Arc refcount bump — the deep-clone branch is short-lived.
- **Confidence**: medium (will be replaced; lifecycle, not dead now).

### `DebugVMState`

- **Path**: `crates/shape-vm/src/executor/mod.rs:458-464`
- **Why suspected**: Two-field struct (`ip`, `call_stack_depth`). Easy to grep for usage; if the integrated debugger has moved on to richer state representations, this stub may be unreferenced.
- **Confidence**: low.

### `LAZY_METHODS` / `MUTEX_METHODS` (concurrency PHF maps)

- **Path**: `crates/shape-vm/src/executor/objects/method_registry.rs:596-625`
- **Why suspected**: Concurrency primitive method maps for `Mutex<T>`, `Atomic<T>`, `Lazy<T>`, `Channel<T>`. Whether all are exposed end-to-end through the user-facing type surface (or only Channel is) — given ADR-006's `var` smart-default does the heavy lifting on storage class — is worth checking.
- **Confidence**: low.

### `IoResource::Custom` type-erased branch

- **Path**: `crates/shape-value/src/heap_value.rs:301`
- **Why suspected**: `Custom(Box<dyn Any + Send>)` for memoized transports / future I/O kinds. Whether any current call site actually constructs it (vs. it being reserve-for-future) — easy grep.
- **Confidence**: low.

### `HeapKind::Closure` ordinal-vs-name skew

- **Path**: `crates/shape-value/src/heap_variants.rs:64`
- **Why suspected**: Comment-only marker — ordinal moved from 3 to 2 in the Phase-2b trim while the discriminator name stayed `Closure` (canonical variant is `HeapValue::ClosureRaw`). The mapping is preserved by a dedicated regression test (`heap_value.rs:1325`), but the disconnect between name (`Closure`) and the actual variant (`ClosureRaw`) is a footgun.
- **Confidence**: low (works; rename candidate, not dead code).
