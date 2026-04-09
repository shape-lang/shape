# Stage 6 Gate: ValueWord Deletion Feasibility Audit

**Date**: 2026-04-09
**Branch**: jit-v2-phase1
**Scope**: Full codebase audit of ValueWord, NanTag, and nan_boxing usage to
determine if/when the NaN-boxing layer can be deleted entirely.

---

## 1. Current Reference Counts

### 1a. ValueWord by crate

| Crate | Total refs | Production refs | Test/comment refs |
|-------|-----------|-----------------|-------------------|
| shape-value | 656 | 486 | 170 |
| shape-vm | 4,232 | 3,373 | 859 |
| shape-runtime | 2,435 | 2,232 | 203 |
| shape-jit | 143 | 115 | 28 |
| shape-gc | 3 | 0 | 3 |
| **Total** | **7,469** | **6,206** | **1,263** |

### 1b. Related NaN-boxing types

| Symbol | Total refs across crates |
|--------|------------------------|
| `NanTag` | 346 |
| `NanBoxed` | 34 |
| `nan_boxing` (module) | 96 |

### 1c. Stack API comparison (push/pop patterns)

| API | Call sites (production) |
|-----|----------------------|
| `push_vw` / `pop_vw` (ValueWord) | 950 |
| `push_raw_*` / `pop_raw_*` (raw u64/i64/f64/bool) | 446 |

The raw API is at ~32% adoption. The ValueWord API still dominates.

### 1d. File-level deletion targets

| File | Lines | ValueWord refs | Deletable? |
|------|-------|---------------|------------|
| `shape-value/src/value_word.rs` | 4,032 | 236 (self-refs) | Not yet |
| `shape-jit/src/nan_boxing.rs` | 940 | 0 (uses NanTag) | Not yet |

### 1e. Top 20 files by ValueWord reference count (production only)

| # | File | Refs | Category |
|---|------|------|----------|
| 1 | value_word.rs (definition) | 236 | Structural |
| 2 | arithmetic/mod.rs | 226 | Typed+Generic handlers |
| 3 | state_diff.rs | 155 | Runtime stdlib |
| 4 | wire_conversion.rs | 135 | FFI/boundary |
| 5 | builtins/type_ops.rs | 131 | Builtin ops |
| 6 | objects/typed_array_methods.rs | 129 | Method dispatch |
| 7 | objects/property_access.rs | 110 | Generic handler |
| 8 | objects/datetime_methods.rs | 110 | Method dispatch |
| 9 | objects/mod.rs | 108 | Method dispatch |
| 10 | control_flow/native_abi.rs | 99 | FFI boundary |
| 11 | builtins/math.rs | 89 | Builtin ops |
| 12 | stdlib_io/file_ops.rs | 88 | Runtime stdlib |
| 13 | executor/printing.rs | 78 | Formatting |
| 14 | stdlib/csv_module.rs | 76 | Runtime stdlib |
| 15 | objects/hashmap_methods.rs | 75 | Method dispatch |
| 16 | objects/iterator_methods.rs | 72 | Method dispatch |
| 17 | snapshot.rs | 71 | FFI/boundary |
| 18 | shape_array.rs | 66 | Structural |
| 19 | stdlib/json.rs | 66 | Runtime stdlib |
| 20 | objects/column_methods.rs | 65 | Method dispatch |

---

## 2. Categorization by Removability

### Category A: Now removable (mechanical replacement)

**~870 refs (~14% of production)**

Sites where the type is statically known at handler entry. The pattern is:
```rust
let x = self.pop_vw()?.as_i64().unwrap();  // type already proven
// ... compute ...
self.push_vw(ValueWord::from_i64(result))?;
```
Can be replaced with:
```rust
let x = self.pop_raw_i64()?;
// ... compute ...
self.push_raw_i64(result)?;
```

Key files:
- `arithmetic/mod.rs`: `exec_typed_arithmetic`, `exec_compact_typed_arithmetic` (~100 refs)
- `comparison/mod.rs`: `exec_typed_comparison` (~20 refs)
- `logical/mod.rs`: `exec_typed_logical` (~5 refs)
- `stack_ops/mod.rs`: Dup/Swap/PushConst for known types (~18 refs)
- `v2_handlers/*.rs`: Bridge values between v1 and v2 paths (~140 refs)
- `variables/mod.rs`: Typed variable load/store (~20 refs)
- `loops/mod.rs`: Typed loop counter ops (~10 refs)
- Remaining scatter across executor (~557 refs in typed paths)

**Blocked by**: Nothing. Raw stack ops already exist. This is pure mechanical work.

### Category B: After method dispatch migration

**~3,450 refs (~56% of production)**

The dominant category. All method handler functions use `Vec<ValueWord>` or
`&[ValueWord]` as their argument/return type. This includes:

**VM executor method handlers (55+ files, ~1,600 refs):**
- `objects/array_*.rs` (8 files): map, filter, reduce, sort, join, etc.
- `objects/datatable_methods/*.rs` (7 files): DataTable operations
- `objects/hashmap_methods.rs`, `set_methods.rs`, `deque_methods.rs`
- `objects/datetime_methods.rs`, `instant_methods.rs`
- `objects/typed_array_methods.rs`, `column_methods.rs`
- `objects/iterator_methods.rs`, `matrix_methods.rs`
- `objects/string_methods.rs`, `channel_methods.rs`
- `objects/concurrency_methods.rs`, `priority_queue_methods.rs`
- `objects/object_creation.rs`, `content_methods.rs`

**VM builtin function handlers (15+ files, ~580 refs):**
- `builtins/math.rs`: abs, sqrt, floor, ceil, sin, cos, etc.
- `builtins/type_ops.rs`: isNumber, toString, typeOf, `__into_*`
- `builtins/special_ops.rs`: print, snapshot, fold
- `builtins/array_comprehension.rs`, `json_helpers.rs`
- `builtins/remote_builtins.rs`, `transport_builtins.rs`
- `builtins/intrinsics/*.rs`: statistical, signal, math

**Runtime stdlib handlers (72 functions, ~2,232 refs in shape-runtime):**
- `stdlib/` (json, yaml, xml, csv, toml, crypto, regex, etc.)
- `stdlib_io/` (file_ops, network_ops, process_ops, path_ops)
- `intrinsics/` (math, statistical, random, matrix, vector, fft, etc.)
- `content_*.rs`, `simulation/`, `join_executor.rs`
- `state_diff.rs`, `const_eval.rs`, `context/`

**Blocked by**: The method dispatch ABI. All of these share the
`fn handler(args: &[ValueWord]) -> Result<ValueWord>` signature. Migration
requires either:
1. Monomorphizing all method calls (type-specialized handlers)
2. Changing the signature to `&[(u64, SlotKind)]`
3. Keeping ValueWord as the universal method boundary type (hybrid approach)

### Category C: Structural (heap, GC, stack frame)

**~830 refs (~13% of production)**

ValueWord is embedded in data structures that cannot be trivially rewritten:

**HeapValue variants (36 fields use ValueWord):**
- `LazyIterator` stages: `Map(ValueWord)`, `Filter(ValueWord)`, `FlatMap(ValueWord)`
- `CoroutineState`: `locals: Box<[ValueWord]>`, `result: Option<Box<ValueWord>>`
- `GeneratorFrame`: `params: HashMap<String, ValueWord>`
- `HashMap`: `keys: Vec<ValueWord>`, `values: Vec<ValueWord>`
- `Set`: `items: Vec<ValueWord>`
- `PriorityQueue`: `items: Vec<ValueWord>`
- `Deque`: `items: VecDeque<ValueWord>`
- `MutexObj`: `inner: Arc<Mutex<ValueWord>>`
- `LazyObj`: `initializer/value: Arc<Mutex<Option<ValueWord>>>`
- `ChannelSender/Receiver`: `Arc<mpsc::Sender/Receiver<ValueWord>>`
- `IndexedValue`: `base: ValueWord`, `index: ValueWord`

**Stack/VM state:**
- `VirtualMachine::stack` (logically `Vec<u64>`, but Drop semantics via ValueWord)
- `CallFrame::locals: Vec<ValueWord>`
- `last_uncaught_exception: Option<ValueWord>`
- `module_bindings: Vec<ValueWord>`

**shape-value public API (212 pub fn/const on ValueWord):**
- Constructors: `from_i64`, `from_f64`, `from_string`, `from_bool`, `none`, etc.
- Extractors: `as_i64`, `as_f64`, `as_str`, `as_bool`, `tag()`, etc.
- NanTag dispatch: `tag() -> NanTag` used in 82 sites

**`HostCallable` trait:**
```rust
pub struct HostCallable {
    inner: Arc<dyn Fn(&[ValueWord]) -> Result<ValueWord, String> + Send + Sync>,
}
```
This is the extension module ABI. All extensions use it.

**GC/memory:**
- `write_barrier_vw()` in `memory.rs`
- `trace_nanboxed_bits()` in shape-gc
- Stack Drop cleanup via `ValueWord::from_raw_bits()`

**Blocked by**: Fundamental architecture. HeapValue variants store ValueWord as
their element type. The extension ABI (`HostCallable`) is a public contract.
The stack Drop semantics need a replacement (slot-type bitmap or conservative scanning).

### Category D: FFI/serialization boundary

**~680 refs (~11% of production)**

Cross-system boundaries where ValueWord is the interchange format:

- `native_abi.rs` (~99 refs): C FFI marshaling
- `wire_conversion.rs` (~135 refs): Wire protocol
- `foreign_marshal.rs` (~40 refs): MessagePack marshaling
- `jit_abi.rs` (~31 refs): JIT <-> interpreter boundary
- `snapshot.rs` (~71 refs): VM state serialization
- `module_bindings.rs` + `module_exports.rs` (~62 refs): Module protocol
- `remote.rs` (~10 refs): Remote execution
- shape-jit FFI layer (~115 refs): JIT runtime callbacks

**Blocked by**: Each boundary protocol needs a v2 format design. The JIT ABI
is closest to v2 (already uses raw u64 with SlotKind). Others need new
type-metadata-carrying formats.

### Category E: Formatting/printing (read-only)

**~86 refs (~1% of production)**

- `printing.rs`: ValueFormatter dispatches on NanTag and HeapValue variant

**Blocked by**: Nothing critical. Can reconstruct display info from slot+type.
Low priority, migrate last.

---

## 3. What Agents 26-29 Would Eliminate

Based on the Wave 4 task structure (parallel agents working on v2 migration):

### Agent 26: Typed arithmetic raw-op migration
**Target**: `arithmetic/mod.rs` typed paths (~100 refs), `comparison/mod.rs` (~20),
`logical/mod.rs` (~5)
**Estimated elimination**: ~125 ValueWord refs
**Category**: A (now removable)

### Agent 27: V2 method dispatch prototype
**Target**: Array method handlers, initial signature migration for hot-path
methods (map, filter, reduce, sort)
**Estimated elimination**: ~200-300 ValueWord refs (in the migrated methods)
**Category**: B (method dispatch)
**Note**: This establishes the pattern but covers <10% of method dispatch volume

### Agent 28: Runtime stdlib migration (hot paths)
**Target**: `stdlib/json.rs`, `stdlib_io/file_ops.rs`, `intrinsics/math.rs`
(highest-traffic stdlib modules)
**Estimated elimination**: ~150-200 ValueWord refs
**Category**: B (method dispatch)

### Agent 29: FFI boundary v2 prototype
**Target**: `jit_abi.rs` (already nearly v2), initial `native_abi.rs` typed paths
**Estimated elimination**: ~50-80 ValueWord refs
**Category**: D (FFI boundary)

### Combined Wave 4 impact estimate

| Metric | Before | After Wave 4 | Reduction |
|--------|--------|-------------|-----------|
| Production ValueWord refs | 6,206 | ~5,500-5,650 | ~9-11% |
| push_vw call sites | 678 | ~550-600 | ~12-19% |
| pop_vw call sites | 272 | ~220-240 | ~12-19% |

Wave 4 is a proof-of-concept wave. It establishes patterns and migrates
hot paths but does not achieve bulk deletion.

---

## 4. Can `value_word.rs` and `nan_boxing.rs` Be Deleted?

### `value_word.rs` (4,032 lines): NO

Cannot be deleted. It is the foundational type used by:
- 276 files across 4 crates (238 non-test files)
- 36 HeapValue struct/enum fields
- The `HostCallable` public extension ABI
- Stack Drop semantics (refcount cleanup for heap values)
- 212 public methods that the entire VM and runtime depend on

Even after all Wave 4 work completes, >90% of these dependencies remain.

### `nan_boxing.rs` (shape-jit, 940 lines): NO

Cannot be deleted. The JIT translator uses NaN-boxing constants (TAG_NULL,
TAG_BOOL, etc.) for encoding values in the bytecode-to-IR path. The translator
itself is scheduled for replacement (with a MIR-based code generator), but
that is a post-Wave-4 project. The MIR compiler (`compiler/` directory) is
already NaN-box-free.

### Deletion timeline estimate

| Milestone | ValueWord refs remaining | When |
|-----------|------------------------|------|
| Today | 6,206 | Now |
| After Wave 4 | ~5,500 | +1-2 weeks |
| After method dispatch ABI migration | ~2,000 | +2-3 months |
| After HeapValue refactor | ~800 | +4-6 months |
| After extension ABI v2 | ~400 | +6-9 months |
| Theoretical full deletion | 0 | +9-12 months |
| Practical hybrid equilibrium | ~200-400 | +6 months |

The "practical hybrid" outcome is more realistic than full deletion:
- Scalars use raw stack ops (no ValueWord)
- Heap objects keep thin NaN-box tags for refcount management
- Method dispatch uses typed signatures for monomorphic calls,
  ValueWord for polymorphic fallback
- Extensions keep HostCallable with ValueWord (stable ABI)

---

## 5. Percentage of Usage That Is Removable

| Category | Refs | % of total | Removable now? | Removable after migration? |
|----------|------|-----------|----------------|---------------------------|
| A: Raw-op replaceable | 870 | 14% | YES | YES |
| B: Method dispatch | 3,450 | 56% | No | After ABI change |
| C: Structural | 830 | 13% | No | After HeapValue refactor |
| D: FFI/boundary | 680 | 11% | No | After protocol v2 |
| E: Formatting | 86 | 1% | No | Trivially, last |
| Test/comment | 290 | 5% | N/A | Follows production |
| **Total** | **6,206** | **100%** | **14%** | **~70% with major work** |

The remaining ~30% (structural + extension ABI) may never be fully removed.
The hybrid approach keeps ValueWord as an internal interchange format for
polymorphic paths while all typed/monomorphic paths use raw values.

---

## 6. Remaining Blockers for Full Deletion

### Hard blockers (must solve)

1. **Stack Drop semantics**: Without NaN-box tags, the VM cannot determine
   which stack slots hold heap pointers for refcount cleanup. Needs
   per-frame `slot_kinds` bitmap from the MIR storage planner.

2. **HeapValue field types**: 36 struct fields in HeapValue use ValueWord.
   HashMap keys/values, Set items, Deque items, coroutine locals, etc.
   These need a replacement type (e.g., raw u64 + per-collection type tag,
   or monomorphized container variants).

3. **HostCallable ABI**: The `Fn(&[ValueWord]) -> Result<ValueWord, String>`
   signature is the extension module contract. Changing it breaks all
   extensions. Needs an ABI versioning story.

4. **Method dispatch volume**: 55+ files, 72+ stdlib functions, ~3,450 refs.
   Even with macros, this is 2-4 weeks of migration work.

### Soft blockers (can defer with hybrid)

5. **Generic opcode fallbacks**: Truly polymorphic code paths (pattern
   matching on heterogeneous values) need some tagged dispatch. Could
   use a lighter tag scheme than full NaN-boxing.

6. **Comptime mini-VM**: The compile-time evaluator executes arbitrary code
   and needs a universal value type. ValueWord is the natural choice.

7. **Wire protocol / snapshot format**: Needs v2 format design alongside
   type metadata. Design task, not just code migration.

---

## 7. Recommended Next Steps

### Immediate (this wave)

1. **Complete Wave 4 agent work** -- establishes patterns for all categories
2. **Merge raw-op migration for typed handlers** (Category A, ~125 refs)
3. **Document the method dispatch ABI decision** (monomorphize vs. typed
   signature vs. hybrid)

### Next wave (Wave 5)

1. **Bulk method dispatch migration** -- use the pattern from Agent 27 to
   migrate remaining method handlers in executor/objects/
2. **Stack slot-type bitmap** -- plumb MIR storage planner data to the
   executor for heap-pointer tracking without NaN-box tags
3. **Start HeapValue field migration** for hot-path containers (HashMap,
   Array, Set)

### Future (Wave 6+)

1. **Extension ABI v2** with `HostCallable` signature change
2. **Wire protocol v2** with typed serialization
3. **Delete JIT translator** (replaced by MIR compiler) which eliminates
   `nan_boxing.rs`
4. **Assess hybrid equilibrium** -- determine if full deletion is worth
   the remaining effort or if ~200-400 refs in structural positions is
   acceptable

---

## 8. Verdict

**Full deletion is not feasible in this wave or the next.** ValueWord is
too deeply embedded in the heap value system, method dispatch ABI, extension
contract, and serialization protocols.

The realistic target is **hybrid equilibrium at ~200-400 refs** within 6
months, where:
- All hot-path scalar operations bypass ValueWord entirely (raw stack ops)
- Method dispatch uses typed signatures for monomorphic calls
- ValueWord survives as an internal interchange format for polymorphic
  paths and heap-backed container elements
- Extensions keep the current HostCallable ABI (or a v2 version that
  still uses a universal value type)

The files `value_word.rs` and `nan_boxing.rs` cannot be deleted. They can
be progressively hollowed out (removing unused constructors/extractors as
callers migrate) but the core type and its Drop/Clone/tag infrastructure
will remain for the foreseeable future.
