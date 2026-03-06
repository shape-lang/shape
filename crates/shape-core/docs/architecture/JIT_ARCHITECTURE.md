# JIT Architecture: Current Limitations and Path to Full Support

## Current Design: Pure Numeric Model

**What we have:**
```rust
// JIT stack - pure f64
pub stack: [f64; 128]

// All operations assume numeric values
let a = stack.pop();  // f64
let b = stack.pop();  // f64
let result = a + b;   // f64
stack.push(result);
```

**What the VM has:**
```rust
// VM stack - tagged union
pub enum VMValue {
    Number(f64),
    String(Rc<String>),
    Array(Rc<Vec<VMValue>>),
    Object(Rc<HashMap<String, VMValue>>),
    Function(u16),
    Closure { function_id: u16, captures: Vec<VMValue> },
    // ... 20+ more variants
}
```

## The Gap: Type Information

The JIT **doesn't know** if a stack slot contains:
- A number (5.0)
- A pointer to a string (0x7f8a4c00)
- A pointer to an array (0x7f8a4c10)
- A function ID (42)

### Why This Matters

**For `NewArray`:**
```shape
let arr = [1, 2, 3];
```

**Bytecode:**
```
PushConst(1)    // Stack: [1.0]
PushConst(2)    // Stack: [1.0, 2.0]
PushConst(3)    // Stack: [1.0, 2.0, 3.0]
NewArray(3)     // Stack: [<array_pointer>]  ← How do we represent this as f64?
```

**For `GetProp`:**
```shape
let x = obj.field;
```

**Bytecode:**
```
LoadLocal(obj)   // Stack: [<object_pointer>]
GetProp("field") // Need to:
                 // 1. Dereference pointer
                 // 2. Lookup "field" in hash map
                 // 3. Return value (could be any type!)
```

---

## Solution: NaN-Boxing (Industry Standard)

### What is NaN-Boxing?

IEEE-754 `f64` has **many** NaN bit patterns we can use:

```
f64 bit layout:
┌────┬─────────────┬──────────────────────────────────────────────────┐
│Sign│  Exponent   │                   Mantissa                       │
│ 1  │     11      │                     52 bits                      │
└────┴─────────────┴──────────────────────────────────────────────────┘

NaN values: exponent = all 1s (0x7FF)

Canonical NaN:     0x7FF8000000000000
Quiet NaN range:   0x7FF8000000000000 to 0x7FFFFFFFFFFFFFFF  ← We can use these!

Available encodings:
0x7FF0000000000000 to 0x7FF7FFFFFFFFFFFF  = ~2^51 values
```

### Encoding Scheme

```rust
const TAG_NUMBER:   u64 = 0x0000_0000_0000_0000;  // Normal f64
const TAG_NULL:     u64 = 0x7FF0_0000_0000_0001;
const TAG_BOOL:     u64 = 0x7FF0_0000_0000_0002;  // + boolean value
const TAG_STRING:   u64 = 0x7FF1_0000_0000_0000;  // + pointer in lower 48 bits
const TAG_ARRAY:    u64 = 0x7FF2_0000_0000_0000;  // + pointer
const TAG_OBJECT:   u64 = 0x7FF3_0000_0000_0000;  // + pointer
const TAG_FUNCTION: u64 = 0x7FF4_0000_0000_0000;  // + function ID
const TAG_CLOSURE:  u64 = 0x7FF5_0000_0000_0000;  // + pointer
```

### Implementation

```rust
#[inline]
fn box_number(n: f64) -> u64 {
    n.to_bits()
}

#[inline]
fn box_pointer(ptr: *const u8, tag: u64) -> u64 {
    tag | (ptr as u64 & 0x0000_FFFF_FFFF_FFFF)
}

#[inline]
fn unbox(bits: u64) -> VMValue {
    if bits & 0x7FF0_0000_0000_0000 != 0x7FF0_0000_0000_0000 {
        // Normal number
        return VMValue::Number(f64::from_bits(bits));
    }

    let tag = bits & 0xFFFF_0000_0000_0000;
    let payload = bits & 0x0000_FFFF_FFFF_FFFF;

    match tag {
        TAG_NULL => VMValue::Null,
        TAG_BOOL => VMValue::Bool(payload != 0),
        TAG_STRING => {
            let ptr = payload as *const String;
            VMValue::String(unsafe { Rc::from_raw(ptr) })
        }
        TAG_ARRAY => {
            let ptr = payload as *const Vec<VMValue>;
            VMValue::Array(unsafe { Rc::from_raw(ptr) })
        }
        // ...
    }
}
```

---

## What Changes in JIT Code

### Before (Pure f64):
```rust
// Cranelift IR
let a = builder.ins().load(types::F64, ...);
let b = builder.ins().load(types::F64, ...);
let result = builder.ins().fadd(a, b);
```

### After (NaN-Boxed):
```rust
// Cranelift IR - now working with i64 (bit patterns)
let a_bits = builder.ins().load(types::I64, ...);
let b_bits = builder.ins().load(types::I64, ...);

// Check if both are numbers (tag check)
let a_is_num = builder.ins().icmp(IntCC::UnsignedLessThan, a_bits, TAG_FIRST_NAN);
let b_is_num = builder.ins().icmp(IntCC::UnsignedLessThan, b_bits, TAG_FIRST_NAN);
let both_num = builder.ins().band(a_is_num, b_is_num);

// If both numbers, do fast path
let then_block = builder.create_block();
let else_block = builder.create_block();
builder.ins().brif(both_num, then_block, &[], else_block, &[]);

builder.switch_to_block(then_block);
// Fast path: reinterpret as f64, do arithmetic
let a_f64 = builder.ins().bitcast(types::F64, a_bits);
let b_f64 = builder.ins().bitcast(types::F64, b_bits);
let result_f64 = builder.ins().fadd(a_f64, b_f64);
let result_bits = builder.ins().bitcast(types::I64, result_f64);

builder.switch_to_block(else_block);
// Slow path: call runtime function for polymorphic add
let runtime_add = // ... declare external function
let result_bits = builder.ins().call(runtime_add, &[a_bits, b_bits]);
```

---

## Required Changes for Full Support

### 1. **Change Stack Type** (`jit.rs`)
```rust
// Before
pub stack: [f64; 128],

// After
pub stack: [u64; 128],  // NaN-boxed values
```

### 2. **Boxing/Unboxing Helpers**
```rust
// In JIT runtime
extern "C" fn jit_box_string(s: *const String) -> u64;
extern "C" fn jit_box_array(arr: *const Vec<VMValue>) -> u64;
extern "C" fn jit_unbox_string(bits: u64) -> *const String;
extern "C" fn jit_type_tag(bits: u64) -> u8;
```

### 3. **Heap Operations via FFI**
```rust
extern "C" fn jit_new_array(elements: *const u64, count: usize) -> u64;
extern "C" fn jit_get_prop(obj_bits: u64, key: *const String) -> u64;
extern "C" fn jit_call_function(fn_id: u16, args: *const u64, argc: usize) -> u64;
```

### 4. **Type Guards in Generated Code**
Every operation needs type checks:
```rust
OpCode::Add => {
    // Generate type check
    let both_numbers = check_both_numbers(a, b);
    brif(both_numbers, fast_add, slow_add);

    // Fast path: numeric addition
    fast_add: fadd(a, b)

    // Slow path: call runtime for string concat, series ops, etc.
    slow_add: call(runtime_add, a, b)
}
```

---

## Performance Impact

### Pure f64 Model (Current):
```assembly
; No type checks needed - everything is f64
movsd xmm0, [rsi]      ; Load a
movsd xmm1, [rsi+8]    ; Load b
addsd xmm0, xmm1       ; Add
movsd [rdi], xmm0      ; Store result
; ~4 instructions, ~1ns
```

### NaN-Boxed Model:
```assembly
; With type checks
mov rax, [rsi]         ; Load a (as bits)
mov rbx, [rsi+8]       ; Load b (as bits)
mov rcx, 0x7FF0000000000000
cmp rax, rcx           ; Check if a is number
jae slow_path
cmp rbx, rcx           ; Check if b is number
jae slow_path
movq xmm0, rax         ; Bitcast to f64
movq xmm1, rbx
addsd xmm0, xmm1       ; Add
movq rax, xmm0         ; Bitcast back
mov [rdi], rax
jmp done

slow_path:
; Call runtime function
call jit_runtime_add   ; ~50-100ns

done:
; ~12-15 instructions for fast path, ~1-2ns
; Slow path: ~50-100ns (still faster than VM's ~2000ns)
```

**Result:** Still **20-100x faster** than VM for numeric ops, and supports all types!

---

## Decision Point

### Option 1: Keep Pure f64 (Current)
- ✅ Simplest implementation
- ✅ Maximum performance for numeric strategies (~1µs/candle)
- ❌ Only supports ~40/60 opcodes
- ❌ Most strategies fall back to VM

### Option 2: Implement NaN-Boxing (Full Support)
- ✅ Supports ALL 60 opcodes
- ✅ Still 20-100x faster than VM for hot paths
- ✅ Production-ready for ALL strategies
- ⚠️ ~3-5x more complex implementation
- ⚠️ Slight performance cost for numeric ops (1µs → 2µs per candle)

### Option 3: Hybrid (Best of Both)
- ✅ Use pure f64 stack when `can_jit_compile()` returns true
- ✅ Use NaN-boxed stack when complex types needed
- ✅ Automatically choose best model per function
- ❌ Most complex - two separate code paths

---

## Recommendation

**Implement Option 2: NaN-Boxing for Full Support**

This is what production JITs do (V8, SpiderMonkey, LuaJIT). The ~2x slowdown on pure numeric code (1µs → 2µs) is negligible compared to the ~1000x speedup vs interpreter, and it unlocks:

- ✅ Function calls (enables modular strategies)
- ✅ Arrays/Objects (enables data aggregation)
- ✅ Closures (enables higher-order functions)
- ✅ **100% opcode coverage**
- ✅ **Production-ready for ALL strategies**

The feature tracking system will automatically verify full parity once implemented!

---

## Implementation Plan

1. **Phase 1: NaN-Boxing Runtime** (2-3 hours)
   - Define tag constants
   - Implement box/unbox helpers
   - Add type guard functions

2. **Phase 2: Update JIT Compiler** (3-4 hours)
   - Change stack from `[f64]` to `[u64]`
   - Add type checks to all operations
   - Generate guarded branches

3. **Phase 3: Heap Operations** (4-5 hours)
   - Implement `jit_new_array()`, `jit_new_object()`
   - Implement `jit_get_prop()`, `jit_set_prop()`
   - Integrate with existing GC

4. **Phase 4: Function Calls** (5-6 hours)
   - Build function table
   - Implement calling convention
   - Handle return values

5. **Phase 5: Validation** (2-3 hours)
   - Run parity matrix - should show 158/158 full parity
   - Benchmark performance
   - Update documentation

**Total: ~15-20 hours to production-ready full JIT**

Would you like me to implement NaN-boxing for full opcode support?
