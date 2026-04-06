# v2 Complete NaN-Boxing Removal: Monomorphization + Typed Collections Architecture

**Status**: Design document for multi-session execution
**Prerequisite**: All prior v2 work (native JIT locals, typed VM stack, BytecodeToIR deletion)
**Goal**: Zero ValueWord, zero NaN-boxing, zero type-tag dispatch anywhere in the runtime

## The Problem

The JIT compiler's MIR-to-IR path now operates with zero NaN-boxing tags. But the
rest of the runtime still uses `ValueWord` (NaN-boxed u64) as the universal value type:

| Component | ValueWord refs | What must change |
|-----------|---------------|-----------------|
| VM executor handlers | ~1,400 push_vw/pop_vw | All use raw typed push/pop |
| shape-value HeapValue | 30+ variants | Replaced by typed pointers |
| VMArray (`Arc<Vec<ValueWord>>`) | 549 refs | `TypedArray<T>` per element type |
| HashMapData | ValueWord values | `TypedMap<K,V>` per key/value type |
| EnumValue | `HashMap<String, ValueWord>` fields | Typed enum layout |
| Closures/Upvalues | ValueWord captures | Typed capture slots |
| Method dispatch | ValueWord args/return | Typed dispatch tables |

## Architecture Overview

```
Source → Parser → AST → Type Inference → Monomorphizer → Bytecode Compiler → Typed Bytecode
                              ↓                                                      ↓
                     Concrete type per          Typed opcodes + TypedArray/TypedStruct
                     variable/param/field       allocation with element types baked in
                              ↓                                                      ↓
                     MIR Lowering              VM Interpreter (raw u64 stack, typed dispatch)
                              ↓                         or
                     MirToIR (Cranelift)        JIT (native registers, inline access)
```

## Phase 1: Type Flow — Compile-Time Type Resolution (foundation)

### 1.1 Complete Type Resolution

Every variable, parameter, field, return value, and array element must have a
concrete resolved type at compile time. The compiler must produce a `TypeLayout`
for every local and every expression.

**New type**: `ConcreteType` (replaces `SlotKind` for richer type info)
```rust
pub enum ConcreteType {
    F64,
    I64,
    I32,
    I16,
    I8,
    U64, U32, U16, U8,
    Bool,
    String,                           // *const StringObj
    Struct(StructLayoutId),           // *const StructLayout, field types known
    Array(Box<ConcreteType>),         // *const TypedArray<T>, element type known
    HashMap(Box<ConcreteType>, Box<ConcreteType>), // typed map
    Option(Box<ConcreteType>),        // nullable pointer or tagged inline
    Result(Box<ConcreteType>, Box<ConcreteType>),
    Enum(EnumLayoutId),              // typed enum with variant layouts
    Closure(ClosureTypeId),          // typed closure with capture types
    Function(FunctionTypeId),        // function pointer with signature
    Pointer(Box<ConcreteType>),      // raw typed pointer
}
```

**File**: `crates/shape-vm/src/concrete_type.rs` (NEW)

**Implementation**:
1. After type inference resolves all `Type::Variable` to concrete types, traverse
   the AST and compute `ConcreteType` for every node
2. Store per-function: `Vec<ConcreteType>` for locals (indexed by slot)
3. Store in `FrameDescriptor`: full `ConcreteType` per slot (not just `SlotKind`)
4. If type inference leaves an unresolved variable → compile error (spec: "no escape hatch")

**Agent team**: 3 agents
- Agent 1: Define `ConcreteType` enum + conversion from Type/TypeAnnotation
- Agent 2: Emit `ConcreteType` per local in the bytecode compiler
- Agent 3: Store in FrameDescriptor, propagate to MIR and JIT

### 1.2 Collection Element Type Tracking

When the compiler sees `[1.0, 2.0, 3.0]` or `let arr: Array<number> = []`, it must
record the element type.

**Already done**: `v2_array_emission.rs` has `infer_array_element_type()` and
`typed_array_from_annotation()`. Extend to ALL collection creation sites.

**Agent team**: 2 agents
- Agent 1: Track element type through array literals, push, map, filter, slice
- Agent 2: Track key/value types through HashMap construction and mutation

## Phase 2: Monomorphization (the big one)

### 2.1 Generic Function Instantiation

Shape has generics: `fn map<T, U>(arr: Array<T>, f: (T) -> U) -> Array<U>`.
Currently compiled ONCE with dynamic dispatch. v2 requires: compiled once per
instantiation (`map<number, string>`, `map<int, bool>`, etc.).

**Strategy**: Lazy monomorphization at call sites.

```rust
// When compiler encounters: arr.map(|x| x.toString())
// where arr: Array<number>
//
// 1. Resolve T = number, U = string from argument types
// 2. Check if map<number, string> is already compiled → reuse
// 3. If not: clone the FunctionDef, substitute T→number, U→string
// 4. Compile the specialized version with concrete types
// 5. Emit call to the specialized version (not the generic template)
```

**New compiler field**: `monomorphized_functions: HashMap<String, u16>`
(key = `"map::number_string"`, value = function index)

**Implementation steps**:
1. At each call site with generic callee: resolve type params from argument types
2. Generate a specialization key: `"fn_name::type1_type2_..."`
3. If key exists in cache → emit call to cached function index
4. If not: clone FunctionDef, substitute type params, compile, cache
5. The specialized function uses `ConcreteType` for all locals → typed opcodes

**Already partially exists**: `const_specializations` in `function_calls.rs` does
this for comptime const args. Extend to TYPE args.

**Agent team**: 4 agents
- Agent 1: Type parameter resolution at call sites (resolve T from argument types)
- Agent 2: FunctionDef cloning + type substitution
- Agent 3: Specialization key + cache management
- Agent 4: Integration tests for monomorphized stdlib (map, filter, reduce)

### 2.2 Stdlib Monomorphization

The stdlib defines generic functions like `map`, `filter`, `reduce`, `sort`.
These must be monomorphized for each element type used in the program.

**Key insight**: Most stdlib methods are SIMPLE — they iterate, compare, or
transform elements. With concrete element types, the compiler emits typed opcodes:
- `Array<number>.map(f)` → `TypedArrayGetF64` + call f + `TypedArrayPushF64`
- `Array<int>.filter(f)` → `TypedArrayGetI64` + call f + `TypedArrayPushI64`

The stdlib source code doesn't change — monomorphization happens at the bytecode level.

**Agent team**: 2 agents
- Agent 1: Monomorphize array methods (map, filter, reduce, find, forEach, etc.)
- Agent 2: Monomorphize string methods (already concrete) + HashMap methods

## Phase 3: Typed Collections (replace VMArray, HashMapData, etc.)

### 3.1 Replace VMArray with TypedArray<T>

**Current**: `VMArray = Arc<Vec<ValueWord>>` — heterogeneous NaN-boxed elements
**Target**: `TypedArray<T>` with contiguous native-typed buffer

**Already built**: `v2_typed_array.rs` in shape-value has the TypedArrayHeader
with alloc/get/push for f64/i64/i32/bool.

**Integration steps**:
1. When compiler emits `NewArray` for a known element type → emit `NewTypedArrayF64`
2. When compiler emits `ArrayPush` for a known element type → emit `TypedArrayPushF64`
3. When compiler emits `ArrayGet` for a known element type → emit `TypedArrayGetF64`
4. VM handlers for typed array opcodes use `push_raw_f64`/`pop_raw_f64`
5. JIT MirToIR uses `v2_array_get`/`v2_array_set` (inline Cranelift loads)

**For heterogeneous arrays** (rare, only from explicit `[1, "hello", true]`):
Keep `Vec<u64>` with a type tag array alongside: `elem_types: Vec<ConcreteType>`.
Each element stored as raw u64 bits. The type tag tells how to interpret.

**Agent team**: 3 agents
- Agent 1: Compiler emits typed array opcodes when element type known
- Agent 2: VM handlers use TypedArrayHeader + raw push/pop
- Agent 3: JIT MirToIR uses inline v2_array_get/set

### 3.2 Replace HashMapData with TypedMap<K,V>

**Current**: `HashMapData { map: HashMap<u64, ValueWord> }` — NaN-boxed keys/values
**Target**: `TypedMap<K,V>` with native-typed storage

**Implementation**:
```rust
pub struct TypedMapHeader {
    header: HeapHeader,     // 8 bytes
    key_type: u8,           // ConcreteType tag for keys
    value_type: u8,         // ConcreteType tag for values
    // ... hashbrown::RawTable with typed slots
}
```

For `HashMap<string, number>`: keys are interned StringObj pointers, values are raw f64.
For `HashMap<string, int>`: keys are StringObj pointers, values are raw i64.

**Agent team**: 2 agents
- Agent 1: TypedMapHeader layout + alloc/get/set for common type combos
- Agent 2: Compiler + VM integration

### 3.3 Replace EnumValue with Typed Enum Layout

**Current**: `EnumValue::Struct(HashMap<String, ValueWord>)` — dynamic field lookup
**Target**: Typed enum with C-compatible variant layouts

**Implementation**: Each enum gets a compile-time layout:
```rust
// enum Shape { Circle(number), Rectangle(number, number) }
// →
// tag: u8 (0 = Circle, 1 = Rectangle)
// payload: union { circle: f64, rectangle: (f64, f64) }
// Total: 1 + max(8, 16) = 17 bytes, padded to 24
```

**Agent team**: 2 agents
- Agent 1: Enum layout computation + alloc
- Agent 2: Match dispatch using tag byte (no string comparison)

## Phase 4: Delete ValueWord (the final deletion)

Once ALL of the above is in place:

### 4.1 Replace push_vw/pop_vw with push_raw/pop_raw
- Every remaining `push_vw(vw)` → `push_raw_u64(val)` with typed interpretation
- Every remaining `pop_vw()` → typed `pop_raw_f64()` / `pop_raw_i64()` etc.
- Delete `push_vw`, `pop_vw`, `stack_read_vw`, `stack_write_vw` etc.
- 1,425 call sites to change

### 4.2 Delete HeapValue enum
- Replace with typed pointers: `*const TypedArray<f64>`, `*const StringObj`, etc.
- The `kind` field in HeapHeader tells the GC/debug system what type it is
- Compiled code NEVER reads `kind` — it knows the concrete type

### 4.3 Delete ValueWord
- `ValueWord` becomes `pub type ValueWord = u64` (no methods)
- Then delete the type alias entirely
- Delete `value_word.rs` (currently 4,000+ lines)
- Delete `nan_boxing.rs` (currently 940 lines)
- Delete `tags.rs` NaN-boxing constants
- Delete `NanTag` enum

### 4.4 Delete generic opcode handlers
- `exec_arithmetic()` with `(tag_a, tag_b)` dispatch → deleted
- `exec_comparison()` with tag dispatch → deleted
- Generic `Add`/`Sub`/`Mul` opcodes → removed from opcode enum
- Only typed opcodes remain

**Agent team**: 5 agents (one per sub-step + verification)

## Phase 5: Comptime Integration

### 5.1 Comptime Type Resolution
Comptime evaluation must produce typed values, not ValueWord:
- `comptime { type_info(T) }` returns a typed struct, not a NaN-boxed object
- `comptime for field in fields { ... }` iterates typed field descriptors
- Comptime blocks produce compile-time constants with known types

### 5.2 Const Generics
Extend monomorphization to support const generic params:
- `fn repeat<const N: int>(x: number) -> Array<number>` → compile once per N
- Const params become literal constants in the specialized code

**Agent team**: 2 agents

## Execution Order (Dependency Graph)

```
Phase 1.1 (ConcreteType)     ─┐
Phase 1.2 (Element tracking)  ─┼── Foundation (parallel)
                               │
Phase 2.1 (Monomorphization)  ─── Depends on Phase 1
Phase 2.2 (Stdlib mono)       ─── Depends on Phase 2.1
                               │
Phase 3.1 (TypedArray)        ─── Depends on Phase 1 + 2
Phase 3.2 (TypedMap)          ─── Depends on Phase 1 + 2
Phase 3.3 (TypedEnum)         ─── Depends on Phase 1
                               │
Phase 4 (Delete ValueWord)    ─── Depends on ALL Phase 3
Phase 5 (Comptime)            ─── Depends on Phase 1 + 2
```

## Agent Team Sizing

| Phase | Agents | Estimated Scope |
|-------|--------|----------------|
| 1.1 ConcreteType | 3 | New type + conversion + propagation |
| 1.2 Element tracking | 2 | Array/Map element type flow |
| 2.1 Monomorphization | 4 | Core monomorphizer engine |
| 2.2 Stdlib mono | 2 | Stdlib method specialization |
| 3.1 TypedArray | 3 | Replace VMArray end-to-end |
| 3.2 TypedMap | 2 | Replace HashMapData end-to-end |
| 3.3 TypedEnum | 2 | Typed enum layouts |
| 4.x Delete ValueWord | 5 | Mass deletion + verification |
| 5.x Comptime | 2 | Typed comptime values |
| **Total** | **25** | |

## Verification Gates

After each phase:
- `cargo check --workspace` passes
- `cargo test -p shape-vm --lib` passes (1,400+ tests)
- `cargo test -p shape-jit --lib` passes (200+ tests)
- Suessco solver produces correct results
- `grep -rn "ValueWord" crates/` count decreases monotonically

After Phase 4:
- `ValueWord` has ZERO references outside of type alias
- `nan_boxing.rs` is DELETED
- `NanTag` has ZERO references
- `HeapValue` enum is DELETED
- All collection types are monomorphized
- `grep -rn "NanBox\|nan_box\|TAG_NULL\|TAG_BOOL\|tag()" crates/` returns ZERO
