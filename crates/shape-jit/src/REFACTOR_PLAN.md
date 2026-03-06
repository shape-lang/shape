# JIT Module Refactoring Plan

## Goal
Split `core.rs` (7,539 lines) into files of ~500 lines max.

## Current State
```
jit/
├── mod.rs           (30 lines)
├── nan_boxing.rs    (89 lines)  ✓
├── context.rs       (548 lines) ✓
└── core.rs          (7,539 lines) ← needs splitting
```

## Target Structure
```
jit/
├── mod.rs              (~80 lines)   - Public API, re-exports
├── nan_boxing.rs       (89 lines)    - NaN-boxing constants ✓
├── context.rs          (548 lines)   - JITContext, JITCandle ✓
├── compiler.rs         (~600 lines)  - JITCompiler struct + new/compile
├── ffi_refs.rs         (~200 lines)  - FFIFuncRefs struct, symbol registration
├── helpers.rs          (~300 lines)  - can_jit_compile, get_unsupported_opcodes, stubs
├── bytecode_ir/
│   ├── mod.rs          (~200 lines)  - BytecodeToIR struct, main compile loop
│   ├── stack_ops.rs    (~150 lines)  - PushConst, Pop, Dup, Swap
│   ├── arithmetic.rs   (~200 lines)  - Add, Sub, Mul, Div, Mod, Neg, Pow
│   ├── comparison.rs   (~200 lines)  - Gt, Lt, Eq, FuzzyEq, etc.
│   ├── control_flow.rs (~300 lines)  - Jump, JumpIf*, SetupTry, Throw, Return
│   ├── variables.rs    (~150 lines)  - LoadLocal, StoreLocal, LoadModuleBinding
│   ├── objects.rs      (~300 lines)  - NewArray, NewObject, GetProp, SetProp
│   ├── functions.rs    (~200 lines)  - Call, CallValue, CallMethod
│   └── builtins.rs     (~400 lines)  - BuiltinCall dispatch
└── ffi/
    ├── mod.rs          (~100 lines)  - Re-exports all FFI functions
    ├── array.rs        (~350 lines)  - jit_new_array, jit_array_*, jit_slice
    ├── object.rs       (~250 lines)  - jit_new_object, jit_get_prop, jit_set_prop
    ├── string.rs       (~200 lines)  - String method helpers (extracted from call_method)
    ├── candle.rs       (~250 lines)  - jit_get_*_at, jit_load_candle
    ├── indicator.rs    (~200 lines)  - jit_sma, jit_ema, jit_rsi
    ├── control.rs      (~350 lines)  - jit_control_fold/map/filter/reduce/foreach
    ├── iterator.rs     (~150 lines)  - jit_iter_done, jit_iter_next
    ├── math.rs         (~100 lines)  - jit_sin, jit_cos, jit_pow, etc.
    ├── conversion.rs   (~100 lines)  - jit_typeof, jit_to_string, jit_to_number
    ├── datetime.rs     (~150 lines)  - jit_eval_datetime_expr, jit_eval_time_reference
    ├── series.rs       (~200 lines)  - jit_series_method, jit_intrinsic_series
    └── method_dispatch.rs (~400 lines) - jit_call_method (dispatcher only)
        └── calls helper functions from array.rs, string.rs, object.rs
```

## Critical Refactoring: jit_call_method (1,000+ lines → ~400 lines)

**Problem:** One massive function with nested match statements.

**Solution:** Extract type-specific method handlers into separate functions:

```rust
// In ffi/method_dispatch.rs (~100 lines)
extern "C" fn jit_call_method(...) -> u64 {
    // Stack manipulation (~50 lines)
    // Type dispatch:
    match tag {
        TAG_ARRAY => call_array_method(arr, &method_name, &args),
        TAG_STRING => call_string_method(s, &method_name, &args),
        TAG_OBJECT => call_object_method(obj, &method_name, &args),
        TAG_DURATION => call_duration_method(dur, &method_name, &args),
        TAG_COLOR => call_color_method(color, &method_name, &args),
        TAG_SERIES => call_series_method(series, &method_name, &args),
        _ => TAG_NULL,
    }
}

// In ffi/array.rs - add helper (~200 lines)
pub fn call_array_method(arr: &[u64], method: &str, args: &[u64]) -> u64 {
    match method {
        "length" | "len" => ...,
        "first" => ...,
        "sum" => ...,
        // etc.
    }
}

// In ffi/string.rs - add helper (~150 lines)
pub fn call_string_method(s: &str, method: &str, args: &[u64]) -> u64 {
    match method {
        "length" | "len" => ...,
        "toUpperCase" => ...,
        // etc.
    }
}

// Similarly for object.rs, duration.rs, etc.
```

## BytecodeToIR Splitting Strategy

The BytecodeToIR has one giant match statement. We can split using a trait or by grouping related opcodes into methods:

```rust
// bytecode_ir/mod.rs
impl BytecodeToIR {
    pub fn compile_instruction(&mut self, instr: &Instruction) {
        match instr.opcode {
            // Stack ops
            OpCode::PushConst | OpCode::Pop | OpCode::Dup | OpCode::Swap
                => self.compile_stack_op(instr),

            // Arithmetic
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | ...
                => self.compile_arithmetic(instr),

            // Comparisons
            OpCode::Gt | OpCode::Lt | OpCode::Eq | ...
                => self.compile_comparison(instr),

            // etc.
        }
    }
}

// bytecode_ir/stack_ops.rs
impl BytecodeToIR {
    pub fn compile_stack_op(&mut self, instr: &Instruction) { ... }
}

// bytecode_ir/arithmetic.rs
impl BytecodeToIR {
    pub fn compile_arithmetic(&mut self, instr: &Instruction) { ... }
}
```

## Execution Order

### Phase 1: FFI Extraction -- DONE
1. [x] Create `ffi/mod.rs`
2. [x] Extract `ffi/array.rs` (jit_new_array, jit_array_get, etc.)
3. [x] Extract `ffi/object.rs` (jit_new_object, jit_get_prop, etc.)
4. [x] Extract `ffi/control.rs` (jit_control_fold/map/filter)
5. [x] Extract `ffi/iterator.rs` (jit_iter_done, jit_iter_next)
6. [x] Extract `ffi/math.rs` (jit_sin, jit_cos, etc.)
7. [x] Extract `ffi/conversion.rs` (jit_typeof, jit_to_string)
8. [x] Extract `ffi/data.rs`, `ffi/window.rs`, `ffi/simd.rs`, `ffi/result.rs`
9. [x] Extract `ffi/call_method/` (dispatcher + per-type handlers)
10. [x] Extract `ffi/typed_object/` (allocation, field access, merge, FFI exports)
11. [x] Extract `ffi/gc.rs`, `ffi/references.rs`, `ffi/async_ops.rs`, `ffi/join.rs`

### Phase 2: Refactor jit_call_method -- DONE
12. [x] Create type-specific helper functions (call_array_method, call_string_method, etc.)
13. [x] Simplify jit_call_method to dispatcher only (`ffi/call_method/mod.rs`)
14. [x] Move helpers to `ffi/call_method/{array,string,object,duration,number,result,time,signal_builder}.rs`

### Phase 3: BytecodeToIR Extraction -- DONE
15. [x] Create `translator/mod.rs` with BytecodeToIR struct in `translator/types.rs`
16. [x] Extract opcode groups: `opcodes/{stack,arithmetic,control_flow,variables,data,functions,references,builtins/}`
17. [x] Extract `translator/compiler.rs`, `translator/helpers.rs`, `translator/storage.rs`, `translator/typed.rs`
18. [x] Further split `opcodes/data.rs` into `collections.rs` + `typed_objects.rs` + `data.rs`
19. [x] Extract `translator/inline_ops.rs` from helpers.rs (inline array/data access)

### Phase 4: JITCompiler & Helpers -- DONE
20. [x] Extract `compiler/setup.rs` (JITCompiler struct)
21. [x] Extract `compiler/ffi_builder.rs` (FFI func ref building)
22. [x] Extract `compiler/accessors.rs` (can_jit_compile, get_unsupported_opcodes)
23. [x] Extract `compiler/strategy.rs`, `compiler/program.rs`

### Phase 5: Verification -- DONE
24. [x] `cargo check -p shape-jit` passes
25. [x] `cargo test -p shape-jit` passes (66 tests)
26. [x] No new warnings introduced

## Current State (Post-Refactor)
- `core.rs`: 698 LOC (tests + re-exports only, down from 7,539)
- Largest file: `context.rs` at 767 LOC (JITContext struct, single cohesive unit)
- All opcode translation files < 620 LOC
- All FFI files < 650 LOC

## Notes
- Each extraction should be one commit
- Test after each extraction
- No functionality changes, just reorganization
