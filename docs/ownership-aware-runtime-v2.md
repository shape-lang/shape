# Ownership-Aware Runtime v2: Eliminate Unnecessary Reference Counting

## Problem Statement

Shape's runtime wraps **every heap-allocated value in `Arc<HeapValue>`** regardless of whether the compiler can prove unique ownership. The MIR borrow checker already computes `OwnershipDecision` (Move/Clone/Copy) and `BindingStorageClass` (Direct/UniqueHeap/SharedCow) per binding, but the executor ignores this — it unconditionally Arc-wraps at `vw_heap_box()` and Arc-bumps at `clone_raw_bits()`.

This means:
- Every string allocation pays atomic refcount overhead
- Every array clone does `Arc::increment_strong_count` even when the source is dead (Move)
- Every scope exit does `Arc::decrement_strong_count` even for uniquely-owned values
- Copy-on-write (`Arc::make_mut`) checks refcount even when the compiler proved single ownership

Rust itself only uses Arc when ownership is genuinely shared. Shape should do the same: **Move by default, Clone only when the compiler proves the value is still live at source, Arc only for `var` bindings or explicit sharing.**

## Current Architecture

### What the compiler knows (MIR layer)

```
MIR Analysis → OwnershipDecision per assignment point:
  Move  — source is dead after this, zero-cost transfer
  Clone — source is still live, need runtime clone
  Copy  — trivially copyable (inline scalars)

Storage Planning → BindingStorageClass per slot:
  Direct     — stack-only, no heap wrapping
  UniqueHeap — heap-allocated, single owner (closure capture w/ mutation)
  SharedCow  — heap-allocated, shared + mutable (var bindings, aliases)
  Reference  — holds &/&mut first-class reference
```

### What the executor does (ignores the above)

```
vw_heap_box(v) → Arc::new(v)           // ALWAYS Arc, even for unique
clone_raw_bits(b) → Arc::increment()   // ALWAYS bump, even for Move
scope exit → Arc::decrement()           // ALWAYS atomic decrement
Arc::make_mut() on mutation             // ALWAYS check refcount
```

### The gap

| What compiler proves | What executor does | What it should do |
|---------------------|-------------------|-------------------|
| `Move` (source dead) | Arc::increment + Arc::decrement | Pointer transfer (zero cost) |
| `Clone` (source live) | Arc::increment | Arc::increment (correct) |
| `Direct` (stack only) | N/A (already inline) | N/A (correct) |
| `UniqueHeap` (single owner) | Arc::new | Box::new (no atomic ops) |
| `SharedCow` (shared mutable) | Arc::new | Arc::new (correct — needs CoW) |

## Design: Three Ownership Modes

### Mode 1: Owned (default for `let`)

Values are uniquely owned. Heap allocation uses `Box<HeapValue>`, not `Arc`. Assignment is Move by default (pointer transfer, source invalidated). Clone only when the compiler proves the source is used after the assignment.

```shape
let arr = [1, 2, 3]     // Box<HeapValue::Array(...)>
let arr2 = arr           // Move: pointer transferred, arr is dead
// arr is invalid here — compile error to use it
```

**Encoding**: New `TAG_OWNED_HEAP` (use existing TAG_HEAP with bit 47 = 0 for owned, bit 47 = 1 for shared). The 48-bit payload already has room — the pointer is 48-bit aligned, so the LSB is free.

Or simpler: **new opcode `MoveHeap`** that transfers the pointer without refcount, vs `CloneHeap` that does `Arc::increment`. The compiler chooses based on `OwnershipDecision`.

### Mode 2: Shared (for `var` or explicit sharing)

Values are reference-counted via `Arc<HeapValue>`. This is today's behavior, but only for bindings declared with `var`:

```shape
var shared_arr = [1, 2, 3]   // Arc<HeapValue::Array(...)>
let alias = shared_arr        // Arc::clone — refcount bump
shared_arr.push(4)            // Arc::make_mut — CoW if refcount > 1
```

**Encoding**: `TAG_HEAP` with shared bit set. `clone_raw_bits` only bumps refcount for shared values.

### Mode 3: Borrowed (for `&` references)

References borrow without ownership transfer. Already implemented via `TAG_REF` for stack slots. Extend to heap borrowing (place-based borrows of fields, array elements).

```shape
let arr = [1, 2, 3]
let first = &arr[0]   // Borrow — no allocation, no refcount
```

## Implementation Plan

### Phase 1: Ownership-Aware Opcodes

**Goal**: The compiler emits different opcodes based on `OwnershipDecision`, and the executor respects them.

#### 1.1: Add Move/Clone/Drop opcodes

New opcodes in `opcode_defs.rs`:

```rust
MoveLocal    = 0xE0,  // Transfer ownership from local slot (source zeroed)
CloneLocal   = 0xE1,  // Clone value from local slot (refcount bump if shared)
DropLocal    = 0xE2,  // Drop value in local slot (decrement if shared, free if owned)
MoveHeap     = 0xE3,  // Transfer heap pointer (no refcount change)
CloneHeap    = 0xE4,  // Clone heap value (Arc::increment for shared, deep-clone for owned)
```

#### 1.2: Compiler emits Move vs Clone

In `crates/shape-vm/src/mir/lowering/`:
- When `OwnershipDecision::Move`: emit `MoveLocal` (zero-cost transfer)
- When `OwnershipDecision::Clone`: emit `CloneLocal` (runtime clone)
- When `OwnershipDecision::Copy`: emit existing `Dup` (trivial copy)

#### 1.3: Executor handles Move vs Clone

In `crates/shape-vm/src/executor/`:
- `MoveLocal`: Read slot, zero the source slot (preventing double-free)
- `CloneLocal`: Read slot, if heap-tagged check shared bit, if shared do `Arc::increment`, if owned do deep clone
- `DropLocal`: If heap-tagged, if shared do `Arc::decrement`, if owned do immediate free

**Verification**: All existing tests pass. Move should be the common case — measure what % of assignments become zero-cost.

### Phase 2: Owned Heap Allocation

**Goal**: Values proven uniquely owned use `Box<HeapValue>` instead of `Arc<HeapValue>`.

#### 2.1: Dual allocation in vw_heap_box

```rust
pub fn vw_heap_box_owned(v: HeapValue) -> ValueWord {
    let ptr = Box::into_raw(Box::new(v));
    make_tagged(TAG_HEAP, (ptr as u64) & PAYLOAD_MASK)  // bit 47 = 0 (owned)
}

pub fn vw_heap_box_shared(v: HeapValue) -> ValueWord {
    let arc = Arc::new(v);
    let ptr = Arc::into_raw(arc) as u64;
    make_tagged(TAG_HEAP, (ptr & PAYLOAD_MASK) | SHARED_BIT)  // bit 47 = 1 (shared)
}
```

#### 2.2: Storage plan drives allocation mode

The compiler's `BindingStorageClass` determines which boxing function to call:
- `Direct` → inline (no heap)
- `UniqueHeap` → `vw_heap_box_owned` (Box)
- `SharedCow` → `vw_heap_box_shared` (Arc)

#### 2.3: Clone behavior depends on ownership

```rust
pub fn clone_value(bits: u64) -> u64 {
    if !is_heap(bits) { return bits; }  // Inline: trivial copy
    if is_shared(bits) {
        // Shared: Arc refcount bump (cheap)
        Arc::increment_strong_count(ptr);
        bits
    } else {
        // Owned: deep clone (heap allocate new copy)
        let hv = unsafe { &*ptr };
        vw_heap_box_owned(hv.clone())
    }
}
```

#### 2.4: Drop behavior depends on ownership

```rust
pub fn drop_value(bits: u64) {
    if !is_heap(bits) { return; }  // Inline: no-op
    if is_shared(bits) {
        // Shared: Arc decrement (may free)
        unsafe { Arc::from_raw(ptr); }  // Drop decrements
    } else {
        // Owned: immediate free (no atomic ops)
        unsafe { Box::from_raw(ptr as *mut HeapValue); }  // Drop frees
    }
}
```

**Verification**: Benchmark — owned values should show zero atomic operations in the hot path.

### Phase 3: Promote Owned → Shared on Demand

**Goal**: When a uniquely-owned value needs to be shared (e.g., captured by closure, stored in `var`), promote it from Box to Arc at that point — not at allocation time.

#### 3.1: Promote opcode

```rust
PromoteToShared = 0xE5,  // Convert owned Box<HeapValue> to Arc<HeapValue>
```

The compiler emits this when:
- A `let` binding is captured by a closure that escapes
- A value is assigned to a `var` binding
- A value is passed to a function that stores it in a shared collection

#### 3.2: Runtime promotion

```rust
pub fn promote_to_shared(bits: u64) -> u64 {
    if !is_heap(bits) || is_shared(bits) { return bits; }
    let ptr = get_payload(bits) as *mut HeapValue;
    let owned = unsafe { Box::from_raw(ptr) };
    let arc = Arc::new(*owned);  // Move value into Arc
    let new_ptr = Arc::into_raw(arc) as u64;
    make_tagged(TAG_HEAP, (new_ptr & PAYLOAD_MASK) | SHARED_BIT)
}
```

#### 3.3: Closure capture integration

When the compiler detects a closure captures a uniquely-owned value:
1. Emit `PromoteToShared` before creating the closure
2. Store the shared pointer as the upvalue
3. The original binding now holds a shared reference too

**Verification**: Closures that don't escape should NOT trigger promotion. Test with `arr.map(|x| x + 1)` — the closure is local, so `arr` stays owned.

### Phase 4: var Semantics

**Goal**: The `var` keyword is the ONLY way to get shared mutable state. Everything else is owned.

#### 4.1: var = SharedCow by default

```shape
var counter = 0        // SharedCow: Arc<HeapValue> (for heap types)
counter = counter + 1  // Arc::make_mut (CoW if shared)

let x = counter        // Clone: Arc refcount bump
```

For inline types (int, float, bool), `var` is just a mutable stack slot — no Arc needed.

#### 4.2: let mut = owned mutable

```shape
let mut arr = [1, 2, 3]  // Owned: Box<HeapValue>
arr.push(4)               // Direct mutation (no CoW check, unique owner)
let arr2 = arr            // Move: arr is dead
```

`let mut` gives single-owner mutability — no sharing, no refcount. This is the equivalent of Rust's `let mut`.

#### 4.3: Migration path

Currently Shape treats `let mut` and `var` identically. The migration:
1. Audit all `var` usage in stdlib and packages
2. Convert `var` → `let mut` where sharing isn't needed
3. Emit warnings for `var` on types that don't need sharing
4. Eventually: `var` becomes the explicit opt-in for shared state

### Phase 5: Escape Analysis for Automatic Ownership

**Goal**: The compiler automatically determines whether a value needs to be shared, without requiring `var`.

#### 5.1: Enhanced escape analysis in MIR

Extend `storage_planning.rs`:
- Track all consumers of each binding
- If no consumer escapes the function → Owned
- If consumer is a local closure that doesn't escape → Owned
- If consumer escapes (returned, stored in collection, captured by escaping closure) → check if mutation needed → SharedCow or Shared

#### 5.2: Interprocedural analysis (future)

Function signatures annotated with ownership requirements:
```shape
fn take(arr: own Array<int>) -> int { ... }      // Takes ownership
fn borrow(arr: &Array<int>) -> int { ... }        // Borrows
fn share(arr: shared Array<int>) -> int { ... }   // Needs Arc
```

Default: `own` for consumed parameters, `&` for read-only parameters.

### Phase 6: GC Integration (Optional)

If Phase 1-5 aren't sufficient for cycle-handling or ergonomics:

#### 6.1: Tracing GC for shared values

Replace `Arc` with a tracing GC for `SharedCow` values:
- Owned values (Box) stay as-is — deterministic drop
- Shared values use GC heap — no refcount, handles cycles
- The `shape-gc` crate already has the infrastructure (currently no-op)

#### 6.2: Generational collection

- Young gen: bump allocator for short-lived shared values
- Old gen: mark-sweep for long-lived shared values
- Write barriers on shared mutation

## Metrics to Track

| Metric | Current | Phase 1 | Phase 2 | Phase 4 |
|--------|---------|---------|---------|---------|
| Atomic ops per assignment | 1-2 | 0 for Move | 0 for Owned | 0 for let/let mut |
| Heap alloc per string | Arc::new | Arc::new | Box::new (if let) | Box::new (default) |
| Clone cost (source live) | Arc::increment | Arc::increment | Deep clone (owned) or Arc::increment (var) | Same |
| Drop cost | Arc::decrement | Arc::decrement | free() (owned) or Arc::decrement (var) | Same |
| % zero-cost assignments | 0% (all Arc) | ~60% (Move) | ~80% (Move + Owned) | ~90% |

## Execution Order

1. **Phase 1** (2-3 weeks): Ownership-aware opcodes. Biggest bang — makes Move zero-cost.
2. **Phase 2** (2-3 weeks): Dual allocation. Eliminates Arc for uniquely-owned values.
3. **Phase 4** (1-2 weeks): var semantics. Clarifies the language model.
4. **Phase 3** (1-2 weeks): Promotion on demand. Handles edge cases.
5. **Phase 5** (3-4 weeks): Escape analysis. Makes ownership automatic.
6. **Phase 6** (optional): GC integration. Only if cycles become a problem.

## Key Files

| File | Role |
|------|------|
| `crates/shape-vm/src/mir/analysis.rs` | Produces OwnershipDecision |
| `crates/shape-vm/src/mir/storage_planning.rs` | Produces BindingStorageClass |
| `crates/shape-vm/src/mir/lowering/` | Lowers MIR to bytecode (emit Move vs Clone) |
| `crates/shape-value/src/value_word.rs` | vw_heap_box, clone_from_bits, TAG_HEAP encoding |
| `crates/shape-vm/src/executor/objects/raw_helpers.rs` | clone_raw_bits (runtime refcount) |
| `crates/shape-vm/src/bytecode/opcode_defs.rs` | Add new Move/Clone/Drop opcodes |
| `crates/shape-vm/src/executor/dispatch.rs` | Execute new opcodes |
| `crates/shape-value/src/value.rs` | Upvalue enum (closure captures) |
| `crates/shape-vm/src/type_tracking.rs` | SlotKind, BindingOwnershipClass |

## Non-Goals

- **Not changing the bit layout**: Values are still 8-byte u64 with NaN-boxing tags
- **Not removing Arc entirely**: `var` bindings and concurrent primitives still need it
- **Not adding Rust-style lifetime annotations to the language**: Shape's borrow checker works automatically
- **Not breaking existing code**: All existing Shape programs continue to work — they just run faster
