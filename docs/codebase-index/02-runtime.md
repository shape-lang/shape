# Codebase Index — Runtime & Value Model

Scope: HeapValue / ValueSlot / TypedObject / TypedArrayData, the VM executor,
stdlib + capabilities, method dispatch, async / concurrency.

ADR-005 (`docs/adr/005-typed-slot-construction.md`) and ADR-006
(`docs/adr/006-value-and-memory-model.md`) define the discipline these
types must conform to. Notes here distinguish "(current)" code shape from
"(ADR-006 target)" where they differ.

---

## 1. Value representation (`shape-value`)

### `HeapValue` enum

**Path**: `crates/shape-value/src/heap_variants.rs:99-148` (defined inside the
`define_heap_types!` macro, expanded from `crates/shape-value/src/heap_value.rs:966`).
**Role**: Single canonical sum type for all heap-resident value shapes after the
strict-typing bulldozer (option C, all `ValueWord`-bearing variants deleted).
**Key rules / invariants**:
- ADR-005 §1: HeapValue is the canonical heap discriminator. Layers above (ConcreteReturn, TypedFieldValue, marshal helpers, JIT FFI carriers) take `Arc<HeapValue>` and dispatch on `HeapValue::kind()`.
- (current) Variants `TypedObject { schema_id, slots, heap_mask }` and `TaskGroup { kind, task_ids }` carry inline payloads; (ADR-006 target) these become `Arc<TypedObjectStorage>`.
- 18 surviving variants: `String / Decimal / BigInt / Future / Char / DataTable / Content / Instant / IoHandle / NativeScalar / NativeView / TypedObject / ClosureRaw / TaskGroup / TypedArray / Temporal / TableView / HashMap`.

**Related**: `HeapKind discriminator`, `ValueSlot`, `NativeKind`, `ConcreteReturn`.

---

### `HeapKind` discriminator

**Path**: `crates/shape-value/src/heap_variants.rs:59-80`.
**Role**: `#[repr(u8)]` discriminant for fast dispatch without full pattern match; one variant per surviving HeapValue arm.
**Key rules / invariants**:
- `HeapKind::Closure` (ordinal 2) routes to `HeapValue::ClosureRaw` (regression-pinned, `heap_value.rs:1316`).
- 18 variants in 0..=17, contiguous (regression test in `heap_header.rs:319`).
- `HeapKind::MAX_VARIANT` (`heap_header.rs:160`) is `Char` — must update when adding variants. The actual max is `HashMap` (17). MAX_VARIANT is stale (see dead-code suspects).

**Related**: `HeapValue enum`, `NativeKind::Ptr(HeapKind)`.

---

### `ValueSlot` struct

**Path**: `crates/shape-value/src/slot.rs:13-150`.
**Role**: 8-byte raw container (`#[repr(transparent)] struct ValueSlot(u64)`) for TypedObject field storage and other typed slot uses.
**Key rules / invariants**:
- ADR-005 §3 / ADR-006 §2.4: typed pointers per FieldType. (current) `ValueSlot::from_heap` (`slot.rs:59`) is the transitional `Box<HeapValue>` wrapper; per-FieldType constructors (`from_string_arc`, `from_typed_array`, ...) are not yet implemented.
- (current) `drop_heap` (`slot.rs:112`) frees via `Box::from_raw`; (ADR-006 target) Drop dispatches on schema-derived `NativeKind`.
- `clone_heap` (`slot.rs:135`) deep-clones into a new Box (no Arc refcount yet).

**Related**: `TypedObject`, `HeapValue`, `NativeKind`, `from_heap_arc rejection (Q6)`.

---

### `NativeKind` enum

**Path**: `crates/shape-value/src/native_kind.rs:32-108`.
**Role**: Single discriminator for typed values at every ABI exit (compile-time proof, marshal layer, wire/snapshot, JIT FFI).
**Key rules / invariants**:
- Width-typed integer variants (Int8..Int64, IntSize) plus nullable mirrors, `Float64`/`NullableFloat64`, `Bool`, `String`, `Ptr(HeapKind)`.
- Watchlist (`native_kind.rs:88-96`): never add parametric `NativeKind::Result`, `Option`, `JsonValue` variants; the strict-typed answer is `HeapKind::TypedObject` plus a per-instantiation `schema_id`.
- `Dynamic` and `Unknown` deleted by the bulldozer; every slot has a proven kind at compile time or it is a compile error.

**Related**: `HeapKind`, `ValueSlot`, `prove_native_kind` (compile-time proof).

---

### `TypedObject` (current shape)

**Path**: `crates/shape-value/src/heap_variants.rs:115-120` (`HeapValue::TypedObject { schema_id: u64, slots: Box<[ValueSlot]>, heap_mask: u64 }`).
**Role**: Object value with schema-defined typed slots; each slot is 8 raw bytes; `heap_mask` bit per slot (1 = heap pointer needing drop).
**Key rules / invariants**:
- Schema-driven layout — slots have no self-description, look-up via `schema_id`.
- Equality compares `schema_id`, `heap_mask`, then raw bits (`heap_value.rs:1213`).
- `heap_mask: u64` caps schemas at ≤64 fields. Reset constraint of the redesign.

**Related**: `ValueSlot`, `TypedObjectStorage (post-ADR-006)`, `TypeSchema` registry.

---

### `TypedObject` (post-ADR-006: `TypedObjectStorage` struct)

**Path**: Not yet implemented. Specified in `docs/adr/006-value-and-memory-model.md:198-222`.
**Role**: Extracted struct holding `{schema_id, slots, heap_mask}`, wrapped as `Arc<TypedObjectStorage>` and stored in `HeapValue::TypedObject`. (ADR-006 target.)
**Key rules / invariants**:
- Drop discipline (ADR-006 §2.5): walks `heap_mask`, dispatches on each field's `NativeKind` to call matching `Arc::decrement_strong_count` or no-op.
- Schema-id lookup at drop time (Q2 default) until profiling justifies promotion to `Arc<TypeSchema>`.
- Slot stores raw `Arc::into_raw(...)` pointer bits, not `Box<HeapValue>`.

**Related**: `TypedObject (current)`, `TypeSchema` (`shape-runtime/type_schema/`), `Q6 from_heap_arc rejection`.

---

### `TypedArrayData` enum

**Path**: `crates/shape-value/src/heap_value.rs:616-636`.
**Role**: Consolidated typed array storage (one HeapValue variant covers all element widths).
**Key rules / invariants**:
- Per-element-type buffers: `I64/F64/Bool/Matrix/I8/I16/I32/U8/U16/U32/U64/F32` plus `String(Vec<Arc<String>>)` and `HeapValue(Vec<Arc<HeapValue>>)` Phase-2d additions.
- Element-kind homogeneity at the `HeapValue` arm is body-side type contract (option β/ε); panicking on mismatch is spec-permitted.
- `FloatSlice { parent, offset, len }` is a non-owning view over a `MatrixData` row.

**Related**: `TypedBuffer<T>`, `HeapValue::TypedArray`, `MatrixData`.

---

### `TypedBuffer<T>`

**Path**: `crates/shape-value/src/typed_buffer.rs:14-19`.
**Role**: Width-specific `Vec<T>` plus optional bit-packed `validity: Option<Vec<u64>>` for nullability.
**Key rules / invariants**:
- One bit per element; `None` validity ⇒ all valid.
- Used by every primitive `TypedArrayData::*` arm. `AlignedTypedBuffer` is the SIMD-aligned f64 specialization.

**Related**: `TypedArrayData`, `AlignedVec`.

---

### `HashMapData`

**Path**: `crates/shape-value/src/heap_value.rs:489-499`.
**Role**: Two-buffer `HashMap` storage — parallel `keys: Arc<TypedBuffer<Arc<String>>>` and `values: Arc<TypedBuffer<Arc<HeapValue>>>`, plus an FNV-1a `index: HashMap<u64, Vec<u32>>` for O(1) lookup.
**Key rules / invariants**:
- Insertion-ordered keys; eager bucket index built at construction (`from_pairs`, `heap_value.rs:514`).
- "shape_id" hidden-class fast-path deferred — refused as architectural-bundling per supervisor.
- Element-type discrimination is body-side: `FromSlot` impls for `Vec<(Arc<String>, Arc<String>)>` and `Vec<(Arc<String>, Arc<HeapValue>)>` decode the same slot.

**Related**: `TypedBuffer`, `HeapValue::HashMap`, `ConcreteReturn::HashMapStringHeapValue`.

---

### `IoHandleData`

**Path**: `crates/shape-value/src/heap_value.rs:333-339`.
**Role**: Shared OS resource handle wrapper; `Arc<Mutex<Option<IoResource>>>` so the resource can be closed and shared.
**Key rules / invariants**:
- Stored as `HeapValue::IoHandle(Arc<IoHandleData>)` (cluster #2 option γ, marshal-layer typed handle).
- `IoResource` (`heap_value.rs:292-303`) covers File / TcpStream / TcpListener / UdpSocket / ChildProcess / PipeReader / PipeWriter / PipeReaderErr / Custom (type-erased `Box<dyn Any + Send>`).
- `close()` takes the inner `Option`, returning whether it was open.

**Related**: `IoResource`, `stdlib_io` module exports, `ConcreteType::IoHandle`.

---

### `NativeScalar`

**Path**: `crates/shape-value/src/heap_value.rs:94-107`.
**Role**: Width-preserving native ABI scalar carrier for `extern C fn` boundaries (avoids lossy `i64` normalization).
**Key rules / invariants**:
- 12 variants: I8/U8/I16/U16/I32/I64/U32/U64/Isize/Usize/Ptr/F32. (Notably no `F64` — `Float64` is a regular `HeapValue::TypedArray::F64` element type, not boxed here.)
- `as_i64` / `as_u64` / `as_i128` / `as_f64` accessors.

**Related**: `HeapValue::NativeScalar`, `extern C fn` ABI bridge (`shape-vm/executor/control_flow/native_abi.rs`).

---

### `MatrixData`

**Path**: `crates/shape-value/src/heap_value.rs:31-86`.
**Role**: Flat, SIMD-aligned `AlignedVec<f64>` plus `rows` / `cols`; row-major storage.
**Key rules / invariants**:
- `Arc<MatrixData>` lives inside `TypedArrayData::Matrix(...)`.
- `FloatSlice` (`TypedArrayData::FloatSlice`) is a non-owning row/column view (parent Arc + offset + len).

**Related**: `TypedArrayData::Matrix`, `TypedArrayData::FloatSlice`, `AlignedVec`.

---

### `TemporalData`

**Path**: `crates/shape-value/src/heap_value.rs:850-858`.
**Role**: Temporal-value sum: `DateTime / Duration / TimeSpan / Timeframe / TimeReference / DateTimeExpr / DataDateTimeRef`.
**Key rules / invariants**:
- Wraps `chrono::DateTime<FixedOffset>` plus AST temporal nodes (`shape_ast::ast::Duration`, etc.).
- Boxed AST forms (`TimeReference`, `DateTimeExpr`, `DataDateTimeRef`) are rare-path AST holdovers used only by `stack_ops` and `printing` (see dead-code suspects).

**Related**: `HeapValue::Temporal`, `stdlib_time`.

---

### `TableViewData`

**Path**: `crates/shape-value/src/heap_value.rs:898-918`.
**Role**: Sum over four DataTable view shapes — `TypedTable / RowView / ColumnRef / IndexedTable` — each carries `schema_id` plus `Arc<DataTable>`.
**Key rules / invariants**:
- All variants share `Arc<DataTable>` so views are zero-copy.
- Row/column indexes stored alongside.

**Related**: `DataTable`, `HeapValue::TableView`, method registry `INDEXED_TABLE_METHODS` / `COLUMN_METHODS`.

---

### `ContentNode`

**Path**: `crates/shape-value/src/content.rs:13-29`.
**Role**: Structured content output — `Text / Table / Code / Chart / KeyValue / Fragment` for rich rendering (carried as `HeapValue::Content(Box<ContentNode>)`).
**Key rules / invariants**:
- Serde-tagged for wire transport.
- Used by Shape's content rendering pipeline (`shape-runtime/src/content_*.rs`).

**Related**: `HeapValue::Content`, `shape-runtime/src/content_*`.

---

### String representation today

**Path**: `crates/shape-value/src/heap_variants.rs:102` — `HeapValue::String(Arc<String>)`.
**Role**: Refcounted UTF-8 buffer; cheap to clone (one atomic).
**Key rules / invariants**:
- ADR-005 §2 named exception: `Arc<String>` allowed as a top-level carrier alongside `Arc<HeapValue>` to avoid extra `Arc::new` per parsed-document field.
- (ADR-006 target) §5: 16-byte tagged carrier with 15-byte SSO inline OR `Arc<[u8]>` pointer; CoW on mutation. Not implemented.
- Compile-time interning via `StringId(u32)` in opcodes (`crates/shape-value/src/ids.rs:60-79`); runtime InternPool deferred.

**Related**: `Arc<String>` carrier exception (ADR-005 §2), `StringId`, `string_intern.rs`, V2StringObj.

---

### `Decimal` representation

**Path**: `crates/shape-value/src/heap_variants.rs:103` — `HeapValue::Decimal(rust_decimal::Decimal)`.
**Role**: Fixed-point decimal arithmetic via `rust_decimal::Decimal` (16 bytes inline; not Arc-wrapped despite ADR-006 §2.3 specifying `Arc<rust_decimal::Decimal>`).

**Related**: `HeapValue::Decimal`, native cross-type comparisons (`bigint_decimal_eq`).

---

### `BigInt` representation

**Path**: `crates/shape-value/src/heap_variants.rs:104` — `HeapValue::BigInt(i64)`.
**Role**: (current) BigInt is just an `i64` payload. A real arbitrary-precision implementation is not present.
**Key rules / invariants**:
- Variant kept on the stage as a placeholder; equals/structural_eq treat it as scalar.
- Targeted ADR-006 §2.3 shape is `Arc<i64>` ("... etc per Kind" example); currently inline.

**Related**: `HeapValue::BigInt`.

---

### `Char` representation

**Path**: `crates/shape-value/src/heap_variants.rs:106` — `HeapValue::Char(char)`.
**Role**: Single Rust `char`; cross-type equality with single-char `String` (`heap_value.rs:1207`).

**Related**: `HeapValue::String`, string-indexing path.

---

### `ClosureRaw` + `OwnedClosureBlock`

**Path**: `crates/shape-value/src/heap_variants.rs:130` (`HeapValue::ClosureRaw(OwnedClosureBlock)`); `crates/shape-value/src/v2/closure_raw.rs:63` (struct), `:265` (`retain_typed_closure`), `:312` (`release_typed_closure`).
**Role**: The canonical (Track A.5) closure representation — owning handle to a raw `*const TypedClosureHeader` block laid out for both VM and JIT.
**Key rules / invariants**:
- Block layout matches JIT Phase H1: `HeapHeader (8) | function_id (4) | type_id (4) | captures...`.
- Cloning `HeapValue::ClosureRaw` increments the block's refcount; dropping decrements via `release_typed_closure`.
- `VmClosureHandle` is the stable read API (`crates/shape-value/src/vm_closure_handle.rs`).

**Related**: `closure_layout::TypedClosureHeader`, `VmClosureHandle`, `op_make_closure_heap`.

---

### `TypedScalar` + `ScalarKind`

**Path**: `crates/shape-value/src/scalar.rs:15-31`.
**Role**: Type-preserving scalar boundary contract for VM↔JIT exchange — `ScalarKind` discriminator plus payload bits in `ValueSlot`.
**Key rules / invariants**:
- 15 kinds (I8..I128, U8..U128, F32/F64, Bool, None, Unit). Discriminants are part of the ABI; do not reorder.

**Related**: `ValueSlot`, `NativeKind` (the proven kind path).

---

### `HeapHeader` (v2)

**Path**: `crates/shape-value/src/heap_header.rs:43-54`.
**Role**: 8-byte `repr(C)` prefix on every v2 heap object — `AtomicU32 refcount @ 0`, `u16 kind @ 4`, `u8 flags @ 6`, `u8 _pad @ 7`. Data starts at `DATA_OFFSET = 8`.
**Key rules / invariants**:
- `retain` = `fetch_add(1, Relaxed)`; `release` = `fetch_sub(1, Release)` plus Acquire fence on last drop.
- `FLAG_MARKED` (GC), `FLAG_PINNED` (GC), `FLAG_READONLY`. (`heap_header.rs:28-32`).
- Static asserts ensure header is exactly 8 bytes (`heap_header.rs:57-61`).

**Related**: `v2_retain` / `v2_release`, `TypedClosureHeader`, `V2TypedArray<T>`.

---

### `vw_clone` / `vw_drop` (current refcount ops)

**Path**: Removed at `HeapValue` level — `heap_value.rs:973` notes the manual `Clone` impl no longer needs `vw_clone`/`vw_drop` bookkeeping after the bulldozer. Stack-side helpers still exist in shape-vm.
**Key rules / invariants**:
- `shape_value::v2::refcount::v2_retain` / `v2_release` (`crates/shape-value/src/v2/refcount.rs:15`, `:30`) are the v2 atomic refcount ops on `HeapHeader`-prefixed allocations.
- VM stack uses `vw_drop_slice` / `vw_drop` over module-bindings + stack drops on VM Drop (`executor/mod.rs:480-490`).
- Pre-existing v2-raw-heap aliasing class (CLAUDE.md note) still needs paired `vw_clone`/`vw_drop` audit on push/pop opcodes.

**Related**: `HeapHeader`, `OwnedClosureBlock`, VirtualMachine `Drop`.

---

## 2. VM executor (`shape-vm/src/executor/`)

### `VirtualMachine` root type

**Path**: `crates/shape-vm/src/executor/mod.rs:213-427`.
**Role**: Top-level VM struct holding program, stack, call/loop/exception/timeframe stacks, GC, scheduler, schemas, JIT dispatch table, metrics, ShapeTableHandle.
**Key rules / invariants**:
- `!Sync` by design; cooperative single-threaded async model.
- `stack: Vec<u64>` (raw bits, `ValueWord` is `repr(transparent)` over `u64`); `sp` = logical top.
- `module_bindings: Vec<u64>` plus `shared_module_bindings: HashSet<usize>` for `Arc<Mutex<ValueWord>>`-promoted bindings reclaimed first in Drop (`mod.rs:489-499`).

**Related**: `CallFrame`, `LoopContext`, `ExceptionHandler`, `TaskScheduler`.

---

### VM frame / call frame

**Path**: `crates/shape-vm/src/executor/mod.rs:163-189` (`CallFrame`).
**Role**: Per-call activation record: `return_ip`, `base_pointer` (window into unified stack), `locals_count`, `function_id`, `upvalues`, `blob_hash`, `closure_heap_bits`.
**Key rules / invariants**:
- Locals live in register windows on the unified stack; no separate locals Vec.
- `closure_heap_bits` (WB2.3 retain-on-read) keeps the closure's allocation alive while the frame's `upvalues` raw pointers are dereferenced; released via `vw_drop` on frame-pop.
- `blob_hash: Option<FunctionHash>` enables content-addressed snapshot frames.

**Related**: `VirtualMachine.stack`, `CaptureKind` (`OwnedMutable` / `Shared`), `op_return`.

---

### Local slots

**Path**: Same unified stack — locals occupy `stack[base_pointer..base_pointer+locals_count]`. No standalone "Locals" struct.
**Key rules / invariants**:
- Each slot is 8 bytes (`u64`). Interpretation supplied by `NativeKind` via the schema/type-tracker, not per-slot tag.
- Loads/stores via `stack_read_raw` / `stack_write_raw` helpers (`executor/vm_impl/stack.rs`).

**Related**: `CallFrame.base_pointer`, `NativeKind`, opcode handlers under `variables/`.

---

### Opcode dispatch loop

**Path**: `crates/shape-vm/src/executor/dispatch.rs` — `execute_with_suspend` (top entry, `:80`); `execute_fast_with_exceptions` is the streamlined fast path when no debugger is attached (`:96-97`).
**Role**: Main interpreter loop; reads `Instruction`, calls per-category handlers (arithmetic, comparison, async, control flow, etc.).
**Key rules / invariants**:
- Installs the VM's `ShapeTableHandle` as the ambient `current_shape_table` for the duration of execution (`dispatch.rs:88-90`).
- `execute()` returns a `ValueWord` synthesized from raw bits per the program's `top_level_frame.return_kind` (`dispatch.rs:34-37`).
- Resource limits ticked once per instruction via `tick_instruction()` when `resource_usage` is `Some` (`mod.rs:373`).

**Related**: `Instruction`, `OpCode`, `ResourceUsage`, `AsyncExecutionResult`.

---

### `drop_heap` (slot drop)

**Path**: `crates/shape-value/src/slot.rs:112-125`.
**Role**: Free a heap slot. (current) `Box::from_raw(ptr as *mut HeapValue)`; under `gc` feature, no-op (GC handles).
**Key rules / invariants**:
- Caller must verify the corresponding `heap_mask` bit is set — slot does not self-describe.
- (ADR-006 target) consults schema-derived `NativeKind` and dispatches per-FieldType.
- Sets bits to 0 after free.

**Related**: `ValueSlot`, `TypedObject.heap_mask`, `Drop` impl on TypedObject (Phase 1.A target).

---

### `clone_heap` (slot clone)

**Path**: `crates/shape-value/src/slot.rs:135-149`.
**Role**: Deep-clone a heap slot. (current) `(*ptr).clone()` then `from_heap`. Under `gc` feature, bitwise copy (GC traces).
**Key rules / invariants**:
- (ADR-006 target) becomes Arc refcount bump (one atomic), not deep clone.

**Related**: `drop_heap`, ADR-006 §2.3.

---

### Resource limits

**Path**: `crates/shape-vm/src/resource_limits.rs:11-48` (`ResourceLimits`); `:52-61` (`ResourceUsage`); `:65-70` (`ResourceLimitExceeded`).
**Role**: Caps for instruction count, memory bytes, wall-clock time, output bytes. Defaults: `unlimited()`; `sandboxed()` = 10M instructions, 256 MB, 30s, 1 MB output.
**Key rules / invariants**:
- Wall-time check amortized every 1024 instructions (`wall_time_check_interval`).
- `tick_instruction()` returns `Err(ResourceLimitExceeded::*)` when a limit is exceeded.

**Related**: `VirtualMachine.resource_usage`, sandbox-control permissions (`MemLimited`, `TimeLimited`, `OutputLimited`).

---

### Exception handling

**Path**: `crates/shape-vm/src/executor/exceptions/mod.rs`; `crates/shape-vm/src/executor/mod.rs:438-446` (`ExceptionHandler` struct with `catch_ip`, `stack_size`, `call_depth`).
**Role**: Try/catch unwinding; opcodes `SetupTry`, `PopHandler`, `Throw`, `TryUnwrap`, `UnwrapOption`, `ErrorContext`, `IsOk`, `IsErr`, `UnwrapOk`, `UnwrapErr`, `TypeCheck`.
**Key rules / invariants**:
- `last_uncaught_exception: Option<ValueWord>` (`mod.rs:282`) captures uncaught exceptions for host structured error rendering.
- AnyError / TraceFrame schemas fixed-layout via `BuiltinSchemaIds` (`mod.rs:268`).

**Related**: `AnyError` schema, `Result<T,E>` opcodes.

---

### Snapshot capture

**Path**: `crates/shape-vm/src/executor/vm_state_snapshot.rs:21-30` (`VmStateSnapshot`); `crates/shape-vm/src/executor/snapshot.rs` (resume/restore).
**Role**: Captures call stack, locals, module bindings at suspension; supports resumable distributed execution.
**Key rules / invariants**:
- `SerializableCallFrame` / `SerializableExceptionHandler` / `SerializableLoopContext` types in `shape-runtime/src/snapshot.rs` are wire-side mirrors.
- Hash-first function identity resolution via `function_id_by_hash` (`snapshot.rs:18-40`).

**Related**: `state_builtins.rs::resume`, `pending_resume` / `pending_frame_resume` fields.

---

## 3. Stdlib + capabilities

### `ModuleExports` + `register_typed_fn_N`

**Path**: `crates/shape-runtime/src/module_exports.rs` (`ModuleExports`); `crates/shape-runtime/src/marshal.rs:888-1330+` (`register_typed_fn_0` … `register_typed_fn_6`, plus `_full` variants).
**Role**: Per-module function registry; typed-return ABI registration helpers derive `arg_kinds` from `FromSlot::NATIVE_KIND` so kinds cannot drift from the body.
**Key rules / invariants**:
- Single registry post-Phase-4c.4 (`typed_module_exports.rs:18-27`): all native bodies live in `TypedModuleExports`, dispatched via `ModuleFnEntry::Typed` / `ModuleFnEntry::TypedAsync`.
- `register_test_function*` wraps a legacy `Fn(...) -> Result<ValueWord, String>` body as `TypedReturn::ValueWord` passthrough.

**Related**: `TypedReturn`, `ConcreteReturn`, `ConcreteType`, `ModuleContext`.

---

### `ConcreteReturn` enum

**Path**: `crates/shape-runtime/src/typed_module_exports.rs:55-175`.
**Role**: Strictly-typed leaf value returned by a native function body. Two-tier with `TypedReturn` (wrappers like `Ok`/`Err`/`Some`/`ObjectPairs`).
**Key rules / invariants**:
- ADR-005 cluster-#7 cleanup target: heap-arm variants (`ArrayHeapValue`, `HashMapStringHeapValue`, `JsonValue`, `OpaqueTypedObject`, `IoHandle`) are scheduled to fold into a single `Heap(Arc<HeapValue>)` arm.
- 19 leaf variants; no recursion (recursion only at `TypedReturn` wrapper layer or inside `JsonValue`).

**Related**: `TypedReturn`, `ConcreteType`, ADR-005.

---

### `TypedReturn` enum

**Path**: `crates/shape-runtime/src/typed_module_exports.rs:189-233`.
**Role**: Wrapper over `ConcreteReturn` for `Ok / Err / Some / None / ObjectPairs / TypedObject / ArrayObjectPairs / SomeObjectPairs / OkObjectPairs / ErrObjectPairs`.
**Key rules / invariants**:
- The leaf-only invariant of `ConcreteReturn` is unrepresentably-violated by Rust's type system — no `TypedReturn::Ok(TypedReturn::Ok(...))` is constructible.
- `TypedReturn::ValueWord` (legacy escape hatch) is removed — strict-typed marshal projects each variant directly into a typed slot.

**Related**: `ConcreteReturn`, `ConcreteType`, marshal layer.

---

### `ConcreteType` enum

**Path**: `crates/shape-runtime/src/typed_module_exports.rs:253-331`.
**Role**: Payload-less mirror of `TypedReturn`/`ConcreteReturn` recorded at registration time; LSP and content-addressed schema can read return shape without invoking the function.
**Key rules / invariants**:
- `shape_type_name()` (`:337-369`) maps each variant back to the user-facing type name.
- `Result(Box<ConcreteType>)` / `Result2(Box, Box)` / `Option(Box)` are recursive at this layer (mirroring `Option<Vec<X>>` etc.).

**Related**: `TypedReturn`, `register_typed_fn_N`.

---

### `JsonValue` (parser intermediate)

**Path**: `crates/shape-runtime/src/json_value.rs:29-37`.
**Role**: Strict-typed sum for parsed-data trees (`Null/Bool/Int/Number/String/Bytes/Array/Object`). Replaces the deleted `ValueWord`-tree return that pre-bulldozer json/yaml/toml/msgpack/xml parsers used.
**Key rules / invariants**:
- ADR-005: parser-intermediate / wire-form translation layer, NOT a runtime storage type for user objects.
- `__parse_typed` projects `JsonValue` to `HeapValue::TypedObject` before reaching user code; only `json.parse` surfaces it as the user-facing `Json` enum.
- `Object` keeps insertion order via `Vec<(String, JsonValue)>` (not HashMap).

**Related**: `ConcreteReturn::JsonValue`, `ConcreteType::JsonValue`, `stdlib/json.rs`.

---

### Capability tags per stdlib function

**Path**: `crates/shape-runtime/src/stdlib/capability_tags.rs:14-100`.
**Role**: Static `(module, function) -> PermissionSet` map consulted at compile time.
**Key rules / invariants**:
- Pure-computation modules (`json / crypto / testing / regex / math`) require no permissions.
- I/O modules' per-function mappings: `io.read_file → [FsRead]`, `io.spawn → [Process]`, `http.get → [NetConnect]`, `env.get → [Env]`, `csv.read_file → [FsRead]`, etc.
- `module_permissions(module)` returns the full union (e.g., `std::core::io → {FsRead, FsWrite, NetConnect, NetListen, Process}`).

**Related**: `Permission enum`, `FunctionBlob.required_permissions`, linker.

---

### `Permission` enum (16 permissions)

**Path**: `crates/shape-abi-v1/src/lib.rs:1001-1041`.
**Role**: Capability vocabulary for static + runtime permission gating.
**Key rules / invariants**:
- Filesystem: `FsRead`, `FsWrite`, `FsScoped`. Network: `NetConnect`, `NetListen`, `NetScoped`. System: `Process`, `Env`, `Time`, `Random`. Sandbox controls: `Vfs`, `Deterministic`, `Capture`, `MemLimited`, `TimeLimited`, `OutputLimited`.
- `name()` returns stable machine-readable strings (`"fs.read"`, `"net.connect"`, ...).
- `category()` partitions into `Filesystem / Network / System / Sandbox`.

**Related**: `PermissionSet`, `ScopeConstraints`, `capability_tags`.

---

### `PermissionSet`

**Path**: `crates/shape-abi-v1/src/lib.rs:1138-1150+`.
**Role**: Set algebra over `Permission`, backed by `BTreeSet` for deterministic iteration / stable serialization.
**Key rules / invariants**:
- Constructors: `pure()` (empty), `readonly()` (`{FsRead, Env, Time}`), `full()` (all 16).
- `is_subset` / `is_superset` / `intersection` / `union` set ops.

**Related**: `FunctionBlob.required_permissions`, host-granted set vs. required set check.

---

### `ScopeConstraints`

**Path**: `crates/shape-abi-v1/src/lib.rs:1312+`.
**Role**: Narrows scoped permissions (`FsScoped`, `NetScoped`) to specific filesystem paths (glob) and network hosts/ports.

**Related**: `Permission::FsScoped`, `Permission::NetScoped`, `PermissionSet`.

---

### Permission baked into FunctionBlob hash

**Path**: `crates/shape-vm/src/bytecode/content_addressed.rs` (FunctionBlob), linker at `crates/shape-vm/src/linker.rs`.
**Role**: `FunctionBlob.required_permissions` is part of the content hash; two functions with identical code but different permissions hash differently. Linker computes transitive union at link time.

**Related**: `PermissionSet`, content-addressed bytecode, `linker_tests.rs`.

---

### IO module (`std::core::io`)

**Path**: `crates/shape-runtime/src/stdlib_io/mod.rs:38-`. Submodules: `file_ops`, `network_ops`, `process_ops`, `path_ops`, `async_file_ops`.
**Role**: File system + network + process operations.
**Key rules / invariants**:
- Migrated (cluster #2 group 1): file-handle ops (`open`, `read_to_string`, `read`, `read_bytes`, `write`, `close`, `flush`), file-path ops (`exists`, `stat`, `is_file`, `is_dir`, `mkdir`, `remove`, `rename`, `read_dir`, `read_gzip`, `write_gzip`).
- Deferred: `path_ops` (`io.join` blocked on varargs marshal), async file ops, network ops, process ops.

**Related**: `IoHandleData`, `Permission::{FsRead, FsWrite, NetConnect, NetListen, Process}`.

---

### HTTP module

**Path**: `crates/shape-runtime/src/stdlib/http.rs`.
**Role**: HTTP client; `get` / `post` / `put` / `delete` require `NetConnect`.

**Related**: `capability_tags.rs::http_permissions`.

---

### JSON / YAML / TOML / MsgPack / XML / CSV modules

**Path**: `crates/shape-runtime/src/stdlib/json.rs` / `yaml.rs` / `toml_module.rs` / `msgpack_module.rs` / `xml.rs` / `csv_module.rs`.
**Role**: Parsers/encoders. Output projected through `JsonValue` (parser intermediate) into typed objects via `__parse_typed`.
**Key rules / invariants**:
- Pure computation — no permissions required (csv read_file is the lone exception).

**Related**: `JsonValue`, `ConcreteReturn::JsonValue`, `__parse_typed`.

---

### Time / DateTime (`stdlib_time.rs`)

**Path**: `crates/shape-runtime/src/stdlib_time.rs` (whole file).
**Role**: Time module — `now`, parsing, formatting, `benchmark` (returns anonymous typed object).

**Related**: `Permission::Time`, `TemporalData::DateTime`.

---

### File ops

**Path**: `crates/shape-runtime/src/stdlib/file.rs`; `stdlib_io/file_ops.rs`.
**Role**: Path-based file read/write APIs. `read_text` / `read_lines` / `read_bytes` / `write_text` / `write_bytes` / `append`.

**Related**: `Permission::{FsRead, FsWrite}`, `IoHandleData`.

---

### Network ops

**Path**: `crates/shape-runtime/src/stdlib_io/network_ops.rs`.
**Role**: TCP / UDP socket primitives — `tcp_connect / tcp_listen / tcp_accept / tcp_read / tcp_write / tcp_close`, `udp_bind / udp_send / udp_recv`. Marked deferred — IoHandle returns share cluster #2 option γ shape.

**Related**: `IoHandleData`, `Permission::{NetConnect, NetListen}`.

---

### Process ops

**Path**: `crates/shape-runtime/src/stdlib_io/process_ops.rs`.
**Role**: `spawn / exec / shell / process_*` returning `IoHandle` for child stdio. Blocked on `Array<string>`-marshal sub-cluster for `args` plus cluster #2 IoHandle returns.

**Related**: `Permission::Process`, `IoResource::ChildProcess`.

---

### Path ops

**Path**: `crates/shape-runtime/src/stdlib_io/path_ops.rs`.
**Role**: `io.join` (varargs), `dirname`, `basename`, `extension`, `resolve`. Mostly deferred; `io.join` blocked on varargs-marshal sub-cluster.

**Related**: `stdlib_io` deferred work.

---

## 4. Method dispatch

### Method registry PHF maps

**Path**: `crates/shape-vm/src/executor/objects/method_registry.rs:44-` (28 PHF maps).
**Role**: Compile-time perfect-hash maps for builtin type methods, O(1) lookup.
**Key rules / invariants**:
- Maps: `ARRAY_METHODS / DATATABLE_METHODS / COLUMN_METHODS / HASHMAP_METHODS / SET_METHODS / DEQUE_METHODS / PRIORITY_QUEUE_METHODS / DATETIME_METHODS / TIMESPAN_METHODS / INSTANT_METHODS / ITERATOR_METHODS / MATRIX_METHODS / INDEXED_TABLE_METHODS / FLOAT_ARRAY_METHODS / INT_ARRAY_METHODS / TYPED_INT_ARRAY_METHODS / TYPED_NUMBER_ARRAY_METHODS / BOOL_ARRAY_METHODS / MUTEX_METHODS / ATOMIC_METHODS / LAZY_METHODS / CHANNEL_METHODS / NUMBER_METHODS / STRING_METHODS / ...`.
- `MethodFnV2` signature: `fn(&mut VirtualMachine, args: &mut [u64], Option<&mut ExecutionContext>) -> Result<u64, VMError>` — operates on raw u64 stack slots, no `Vec<ValueWord>` on hot paths (`method_registry.rs:24-28`).
- `args[0]` is receiver; mutable so handlers can update pointers after in-place mutation (e.g., `Arc::make_mut` realloc).

**Related**: `HeapKind dispatch entry`, `ARRAY_METHODS` (47 methods), `STRING_METHODS`.

---

### Generic method signatures (`GenericMethodSignature`)

**Path**: `crates/shape-runtime/src/type_system/checking/method_table.rs:79-93`.
**Role**: Compile-time type-checking signatures for generic methods (`Vec<T>`, `HashMap<K,V>`, `Option<T>`, `Result<T,E>`).
**Key rules / invariants**:
- `method_type_params: usize` — count of method-introduced type vars (e.g., `U` in `.map<U>`).
- `param_types: Vec<TypeParamExpr>`, `return_type: TypeParamExpr`.
- `receiver_param_bounds`: trait bounds on receiver type params (e.g., `Vec<T: Numeric>.sum()` → `[(0, ["Numeric"])]`).

**Related**: `TypeParamExpr`, `MethodTable`, bidirectional closure inference.

---

### `TypeParamExpr` resolution

**Path**: `crates/shape-runtime/src/type_system/checking/method_table.rs:52-74`.
**Role**: Type-level expression referencing receiver/method type parameters; resolved at call site.
**Key rules / invariants**:
- Variants: `Concrete(Type) / ReceiverParam(usize) / MethodParam(usize) / Function { params, returns } / GenericContainer { name, args } / SelfType`.
- `SelfType` = full receiver type (used for `filter`, `sort` returning the same shape).
- Resolution: extract receiver type args, allocate fresh type vars for `MethodParam`s, walk the tree.

**Related**: `GenericMethodSignature`, `MethodSignature`, `MethodTable`.

---

### `HeapKind`-based dispatch entry

**Path**: `crates/shape-vm/src/executor/objects/mod.rs:429-` (`handle_object_method`-style routing).
**Role**: Pattern match on `HeapValue` variant to pick the right method registry — no `VMValue` materialization on hot paths.
**Key rules / invariants**:
- Dispatch routes by `HeapKind` (e.g., `HeapKind::Array → ARRAY_METHODS`, `HeapKind::HashMap → HASHMAP_METHODS`).
- Falls through to extension methods (registered via `ModuleExports.method_intrinsics`) and UFCS lookup if no PHF match.

**Related**: `method_registry`, `extension_methods`, `function_name_index`.

---

### Method receiver type resolution

**Path**: `crates/shape-runtime/src/type_system/checking/method_table.rs:108-` (`ReceiverType`).
**Role**: Resolves the static receiver type for a method call to look up applicable signatures.
**Key rules / invariants**:
- Universal receiver `__Any__` (`UNIVERSAL_RECEIVER`, `:46`) for methods available on every value (e.g., `toString`, `toJSON`).

**Related**: `MethodTable`, `register_user_method`.

---

## 5. Async / concurrency

### `TaskGroup`

**Path**: `crates/shape-value/src/heap_variants.rs:131-134` (`HeapValue::TaskGroup { kind: u8, task_ids: Vec<u64> }`).
**Role**: Aggregation of multiple in-flight task IDs for `join all|race|any|settle`.
**Key rules / invariants**:
- `kind` discriminates the join strategy (all=0, race=1, any=2, settle=3 — encoded in `op_join_init`).
- Created by `JoinInit` (`async_ops/mod.rs:317-350`); consumed by `JoinAwait`.

**Related**: `WaitType::TaskGroup`, `op_join_init`, `op_join_await`, `TaskScheduler`.

---

### Future representation

**Path**: `crates/shape-value/src/heap_variants.rs:105` — `HeapValue::Future(u64)`.
**Role**: Opaque task ID; the runtime keeps the actual future state in `TaskScheduler`.
**Key rules / invariants**:
- ID is a monotonic `u64` allocated from `VirtualMachine.future_id_counter` (`mod.rs:331`).
- `Future` is structurally just an integer — no closures or completion state stored on it.
- Reserved sentinel `SNAPSHOT_FUTURE_ID = u64::MAX` (`executor/mod.rs:50`).

**Related**: `TaskScheduler`, `WaitType::Future`, `op_spawn_task`.

---

### `async let` / `async scope` / `for await` / `join`

**Path**: `crates/shape-vm/src/executor/async_ops/mod.rs:88-`. Opcodes: `Yield`, `Suspend`, `Resume`, `Poll`, `AwaitBar`, `AwaitTick`, `EmitAlert`, `EmitEvent`, `Await`, `SpawnTask`, `JoinInit`, `JoinAwait`, `CancelTask`, `AsyncScopeEnter`, `AsyncScopeExit`.
**Role**: Cooperative single-threaded async on the VM thread.
**Key rules / invariants**:
- `op_spawn_task`: pops callable, allocates ID, registers with scheduler, pushes `HeapValue::Future(id)`.
- `op_await`: tries inline resolution, suspends with `VMError::Suspended { future_id, resume_ip }` otherwise.
- `AsyncScopeEnter` pushes a new tracked-IDs vector onto `async_scope_stack`; `AsyncScopeExit` cancels still-pending tasks in LIFO order, guaranteeing structured concurrency.

**Related**: `AsyncExecutionResult`, `SuspensionInfo`, `WaitType`.

---

### Join family (`JoinInit` / `JoinAwait`)

**Path**: `async_ops/mod.rs:317-393`.
**Role**: Joins multiple futures with strategy `all|race|any|settle`. `JoinInit` builds `TaskGroup`, `JoinAwait` resolves it.
**Key rules / invariants**:
- `JoinAwait` consumes a `TaskGroup` and pushes a result; net stack effect 0.
- Suspends with `WaitType::TaskGroup { kind, task_ids }` if not synchronously resolvable.

**Related**: `TaskGroup`, `WaitType::TaskGroup`, host runtime resume.

---

### `B0014 NonSendableAcrossTaskBoundary` runtime-side enforcement

**Path**: Compile-time only. `crates/shape-vm/src/mir/solver.rs:1195` raises `BorrowErrorKind::NonSendableAcrossTaskBoundary`; `crates/shape-vm/src/mir/analysis.rs:156, :187, :241` define the kind / code; `crates/shape-vm/src/compiler/functions.rs:597, :630` surface the diagnostic.
**Role**: Detection occurs in the borrow solver / MIR analysis. There is no runtime-side enforcement in the VM executor.
**Key rules / invariants**:
- Decision per ADR-006 Q1: B0014 fires as an error for `let`/`let mut`; for `var` the same condition triggers a class upgrade to `SharedAtomicMut` (or `SharedAtomic` if read-only) — not yet implemented since the `var` smart-default isn't landed.

**Related**: `BorrowErrorKind`, `BorrowErrorCode`, MIR solver, ADR-006 §3.2.

---

### `TaskScheduler`

**Path**: `crates/shape-vm/src/executor/task_scheduler.rs:33-44`.
**Role**: Per-VM scheduler — stores callables by task ID, tracks completion (`TaskStatus::Pending|Completed|Cancelled`), bridges externally-completed Tokio tasks via oneshot channels.
**Key rules / invariants**:
- Inline execution model: tasks run synchronously at await-time. Tokio path layered for remote calls.
- `external_receivers: HashMap<u64, oneshot::Receiver<Result<ValueWord, String>>>` for externally-completed tasks.
- `cancel(id)` only effective if status is `Pending`.

**Related**: `op_spawn_task`, `op_cancel_task`, remote call path.

---

### Channel / queue

**Path**: `crates/shape-vm/src/executor/objects/channel_methods.rs` (per-method handlers); `CHANNEL_METHODS` PHF map (`method_registry.rs:618-625`) — `send / recv / try_recv / close / is_closed / is_sender`.
**Role**: User-level channel primitive (v2 typed). Receiver structurally identifies a channel via the method registry.
**Key rules / invariants**:
- Internally backed by a `SharedAtomicMut`-shaped allocation (deque + condvar pattern; implementation in `channel_methods.rs`).

**Related**: `MUTEX_METHODS`, `ATOMIC_METHODS`, `LAZY_METHODS`, ADR-006 `SharedAtomicMut` storage class.

---
