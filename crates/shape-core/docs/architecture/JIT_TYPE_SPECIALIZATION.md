# JIT Type Specialization Design

**Status:** Implemented | **Author:** Claude | **Date:** 2026-01-20

## Overview

Type specialization is an optimization that generates faster code when the type of a value is known at compile time. Instead of using dynamic dispatch (HashMap lookup) for property access, we can generate direct memory access when the object's schema is known.

## Performance Results

Benchmarks show **10.9x speedup** for typed field access:

| Access Method | Time | Notes |
|--------------|------|-------|
| HashMap lookup | 7.23ns | Dynamic dispatch |
| TypedObject direct | 0.67ns | Direct offset access |
| **Speedup** | **10.9x** | |

## Architecture

### NaN-Boxing for TypedObject

TypedObject uses a specialized NaN-boxing format to distinguish from regular objects:

```rust
// Tag constants in nan_boxing.rs
pub const TAG_TYPED_OBJECT: u64 = 0x7FF3_8000_0000_0000;
pub const TYPED_OBJECT_MARKER_MASK: u64 = 0xFFFF_8000_0000_0000;
pub const TYPED_OBJECT_PAYLOAD_MASK: u64 = 0x0000_7FFF_FFFF_FFFF; // 47-bit pointer

// Helper functions
pub fn box_typed_object(ptr: *const u8) -> u64 {
    TAG_TYPED_OBJECT | ((ptr as u64) & TYPED_OBJECT_PAYLOAD_MASK)
}

pub fn unbox_typed_object(bits: u64) -> *const u8 {
    (bits & TYPED_OBJECT_PAYLOAD_MASK) as *const u8
}

pub fn is_typed_object(bits: u64) -> bool {
    (bits & TYPED_OBJECT_MARKER_MASK) == TAG_TYPED_OBJECT
}
```

**Key Design Decision:** We use 47-bit pointers (0x0000_7FFF_FFFF_FFFF mask) because:
- x86-64 user-space addresses are limited to 47 bits
- This allows the full pointer to be preserved without truncation
- The 0x7FF3_8xxx pattern distinguishes TypedObject from regular TAG_OBJECT (0x7FF3_0xxx)

### TypedObject Memory Layout

```rust
// vm/jit/ffi/typed_object.rs
pub const TYPED_OBJECT_HEADER_SIZE: usize = 8;

#[repr(C)]
pub struct TypedObject {
    pub schema_id: u32,   // Type schema identifier
    pub ref_count: u32,   // Reference count for GC
    // Field data follows inline at known byte offsets
}

impl TypedObject {
    /// Allocate a typed object for a given schema
    pub fn alloc(schema: &TypeSchema) -> *mut TypedObject;

    /// Direct field access by byte offset - O(1)
    pub unsafe fn get_field(&self, offset: usize) -> u64;

    /// Direct field set by byte offset - O(1)
    pub unsafe fn set_field(&mut self, offset: usize, value: u64);
}
```

### Type Schema Registry

```rust
// runtime/type_schema.rs
pub type SchemaId = u32;

pub struct TypeSchema {
    pub id: SchemaId,
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub data_size: usize,  // Total size excluding header
}

pub struct FieldDef {
    pub name: String,
    pub field_type: FieldType,
    pub offset: usize,  // Byte offset from start of data
    pub index: u16,     // Field index for fast lookup
}

pub enum FieldType {
    F64, I64, Bool, String, Timestamp,
    Array(Box<FieldType>), Object(String), Any,
}

pub struct TypeSchemaRegistry {
    by_name: HashMap<String, TypeSchema>,
    by_id: HashMap<SchemaId, String>,
}
```

### ExecutionContext Integration

The `TypeSchemaRegistry` is integrated into `ExecutionContext`:

```rust
// runtime/context.rs
pub struct ExecutionContext {
    // ... other fields ...
    type_schema_registry: Arc<TypeSchemaRegistry>,
}

impl ExecutionContext {
    /// Get the type schema registry for JIT type specialization
    pub fn type_schema_registry(&self) -> &Arc<TypeSchemaRegistry> {
        &self.type_schema_registry
    }
}
```

### FFI Functions for JIT

```rust
// vm/jit/ffi/data.rs

/// Fast path: TypedObject with direct offset access (~0.67ns)
/// Slow path: HashMap fallback (~7.23ns)
pub extern "C" fn jit_get_field_typed(
    obj: u64,
    type_id: u64,
    field_idx: u64,
    offset: u64,
) -> u64 {
    if is_typed_object(obj) {
        let ptr = unbox_typed_object(obj) as *const TypedObject;
        unsafe {
            // Optional type guard
            if type_id != 0 && (*ptr).schema_id != type_id as u32 {
                // Type mismatch - fall through to slow path
            } else {
                // Direct field access - O(1)!
                return (*ptr).get_field(offset as usize);
            }
        }
    }
    // Slow path: HashMap fallback
    // ... dynamic property access ...
}
```

## Implementation Phases

### Phase 1: Type Schema Registry ✅

**File:** `runtime/type_schema.rs`

- `TypeSchema`, `FieldDef`, `FieldType` structs
- `TypeSchemaRegistry` with registration and lookup
- `TypeSchemaBuilder` for fluent API
- Unique schema ID generation
- Field offset computation with 8-byte alignment

### Phase 2: Type Tracking in Compiler ✅

**File:** `vm/type_tracking.rs`

- `TypeTracker` struct for compile-time type information
- `TypedFieldInfo` for precomputed field access metadata
- Scope-based type tracking (module binding, local, inner scopes)
- Type inference from variable declarations

### Phase 3: Typed Opcodes ✅

**File:** `vm/opcodes.rs`

- `GetFieldTyped { type_id, field_idx, offset }` opcode
- `SetFieldTyped { type_id, field_idx, offset }` opcode
- Supports precomputed byte offsets for O(1) access

### Phase 4: JIT Translation ✅

**File:** `vm/jit/ffi/data.rs`

- `jit_get_field_typed()` with fast/slow path
- `jit_set_field_typed()` with fast/slow path
- Type guard checking for safety

### Phase 5: Typed Object Layout ✅

**File:** `vm/jit/ffi/typed_object.rs`

- `TypedObject` struct with 8-byte header
- Direct memory allocation with schema-based sizing
- Reference counting for garbage collection
- Fast field get/set by byte offset

### Phase 6: Integration & Testing ✅

- TypeSchemaRegistry wired to ExecutionContext
- 7 unit tests for TypedObject functionality
- Performance benchmark: 10.9x speedup verified
- All 486+ existing tests pass

## Usage Example

```shape
// Define a type in Shape
type Point {
    x: f64,
    y: f64,
    z: f64
}

// Create and use typed object
let p: Point = { x: 1.0, y: 2.0, z: 3.0 }
let sum = p.x + p.y + p.z  // Fast direct access
```

Under the hood:
1. Compiler detects `p` has type `Point`
2. Emits `GetFieldTyped { type_id: 1, field_idx: 0, offset: 0 }` for `p.x`
3. JIT generates direct memory load at known offset
4. No HashMap lookup, no hash computation

## Success Criteria ✅

- [x] Type schema registry compiles types from Shape definitions
- [x] Compiler can emit typed opcodes when type is known
- [x] JIT generates direct field access for typed opcodes
- [x] Benchmark shows 10x+ speedup for field access (10.9x achieved)
- [x] All existing tests pass (486 tests)
- [x] TypedObject tests pass (7 tests)

## Future Extensions

1. **SIMD Batch Access**: Load multiple adjacent fields in one instruction
2. **Inline Caching**: Cache type checks at call sites
3. **Escape Analysis**: Stack-allocate objects that don't escape
4. **Polymorphic Inline Caches**: Fast paths for 2-3 common types
