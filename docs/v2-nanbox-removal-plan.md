# v2 NaN-Boxing Complete Removal Plan

**Status**: IN PROGRESS
**Goal**: Zero NaN-boxing anywhere in the runtime. Clean break per runtime-v2-spec.md.
**Spec contract**: "NO runtime type tags. NO NaN-boxing. NO dynamic dispatch on value type."

## Current State (2026-04-05)

| Component | NaN-boxing refs | Status |
|-----------|----------------|--------|
| `nan_boxing.rs` | 940 lines | EXISTS — must be deleted |
| `ValueWord` in shape-vm | 3,023 refs | EXISTS — must become raw u64 |
| NaN-boxing in shape-jit | 819 refs | EXISTS — FFI boundary + conversions |
| Generic opcodes emitted | 104 compiler sites | EXISTS — must all become typed |
| `push_vw`/`pop_vw` calls | 1,421 calls | EXISTS — must become push_raw/pop_raw |
| NaN-boxed FFI functions | 94 functions | EXISTS — must become v2 typed |

## Step 1: Compiler — Eliminate Generic Opcode Emission

**Owner**: Agent team (3 agents)
**Files**: `crates/shape-vm/src/compiler/expressions/`, `crates/shape-vm/src/compiler/statements.rs`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Replace all `OpCode::Add` emission with `OpCode::AddInt`/`OpCode::AddNumber` based on inferred types
- [ ] Replace all `OpCode::Sub`/`Mul`/`Div`/`Mod`/`Pow` similarly
- [ ] Replace all `OpCode::Gt`/`Lt`/`Gte`/`Lte`/`Eq`/`Neq` with typed variants
- [ ] Replace `OpCode::Neg` with `NegInt`/`NegNumber` (add opcodes if missing)
- [ ] Replace `OpCode::Not` with typed variant
- [ ] For pattern matching `Eq` on unknown types: use `EqAny` opcode that does polymorphic comparison without NaN-box tags (compare raw bits + type metadata)
- [ ] For string concatenation `Add`: use `StringConcat` opcode
- [ ] Verify: `grep -rn "OpCode::Add[^INDEF]" crates/shape-vm/src/compiler/` returns 0 results

### Acceptance
- Zero generic arithmetic/comparison opcodes emitted by the compiler
- All 1470 shape-vm tests pass

## Step 2: VM Stack — Replace Vec<ValueWord> with Vec<u64>

**Owner**: Agent team (2 agents)
**Files**: `crates/shape-vm/src/executor/mod.rs`, `executor/vm_impl/stack.rs`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Change `stack: Vec<ValueWord>` to `stack: Vec<u64>` in VirtualMachine
- [ ] Change `module_bindings: Vec<ValueWord>` to `Vec<u64>`
- [ ] Delete `push_vw()` and `pop_vw()` — only `push_raw_u64`/`pop_raw_u64` remain
- [ ] Rename `push_raw_u64`/`pop_raw_u64` to `push`/`pop` (they're the only API now)
- [ ] Update all 1,421 push_vw/pop_vw call sites
- [ ] Update CallFrame to use u64 stack
- [ ] Update upvalue/closure capture to use u64

### Acceptance
- Zero `ValueWord` in stack operations
- Zero `push_vw`/`pop_vw` calls
- All tests pass

## Step 3: VM Handlers — Delete Generic Dispatch

**Owner**: Agent team (4 agents)
**Files**: `executor/arithmetic/`, `executor/comparison/`, `executor/dispatch.rs`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Delete `exec_arithmetic()` (generic tag-checking dispatch)
- [ ] Delete `exec_comparison()` (generic tag-checking dispatch)
- [ ] Remove generic opcode arms from `dispatch.rs` match
- [ ] All arithmetic handlers use only `pop_raw_i64`/`push_raw_i64` or `pop_raw_f64`/`push_raw_f64`
- [ ] All comparison handlers push raw bool (0 or 1), not TAG_BOOL_TRUE/FALSE
- [ ] Delete `NanTag` enum and all tag-checking logic
- [ ] Delete feedback vector type profiling for generic opcodes

### Acceptance
- Zero `vw.tag()` calls in executor
- Zero `NanTag` references
- All tests pass

## Step 4: FFI — Replace 94 NaN-Boxed Functions with v2 Typed

**Owner**: Agent team (5 agents)
**Files**: `crates/shape-jit/src/ffi/`, `ffi_symbols/`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Delete `ffi/math.rs` generic functions, keep only `ffi/v2_math.rs`
- [ ] Delete `ffi/array.rs` NaN-boxed functions, keep only `ffi/v2_array.rs`
- [ ] Rewrite `ffi/control/mod.rs` — jit_call_value/jit_call_method use raw u64 stack slots
- [ ] Rewrite `ffi/conversion.rs` — jit_print takes `*const StringObj`, not u64
- [ ] Rewrite `ffi/object/property_access.rs` — get_prop/set_prop use typed pointers
- [ ] Rewrite `ffi/typed_object/` — alloc/field_access use v2 struct layout
- [ ] Rewrite `ffi/arc.rs` — arc_retain/release take raw pointer, not u64
- [ ] Delete all `ffi_symbols/*_symbols.rs` that register NaN-boxed function signatures
- [ ] Create new `ffi_symbols/v2_*.rs` with native Cranelift type signatures
- [ ] Update `ffi_builder.rs` to build only v2 typed FuncRefs

### Acceptance
- Zero `u64` NaN-boxed function signatures in FFI
- All FFI functions take/return native types (f64, i64, *const T, etc.)

## Step 5: MirToIR — Eliminate Boxing/Unboxing

**Owner**: Agent team (2 agents)
**Files**: `crates/shape-jit/src/mir_compiler/`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Delete `conversions.rs` entirely (ensure_nanboxed, unbox_from_nanboxed, box_to_nanboxed)
- [ ] All MirToIR FFI calls use v2 typed FuncRefs with native Cranelift types
- [ ] Return terminator writes raw native value to ctx.stack[0] (already done via return_type_tag)
- [ ] Call terminator passes native-typed args (no boxing)
- [ ] SwitchBool uses native I8 bool (no TAG_BOOL_TRUE comparison for I64 path)
- [ ] Delete all `crate::nan_boxing::*` imports from mir_compiler/

### Acceptance
- Zero `nan_boxing` imports in `mir_compiler/`
- Zero `ensure_nanboxed`/`box_to_nanboxed`/`unbox_from_nanboxed` calls

## Step 6: Delete nan_boxing.rs, ValueWord, Tags

**Owner**: Agent team (3 agents)
**Files**: `shape-jit/src/nan_boxing.rs`, `shape-value/src/value_word.rs`, `shape-value/src/tags.rs`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Delete `crates/shape-jit/src/nan_boxing.rs` (940 lines)
- [ ] Delete or gut `crates/shape-value/src/value_word.rs` — replace with `pub type ValueWord = u64`
- [ ] Delete NaN-boxing tag constants from `shape-value/src/tags.rs`
- [ ] Delete `NanBoxed` type alias
- [ ] Delete `NanTag` enum
- [ ] Update `shape-value/src/lib.rs` exports
- [ ] Fix all compilation errors across the workspace

### Acceptance
- `nan_boxing.rs` does not exist
- `ValueWord` is just `u64` (no methods)
- Zero NaN-boxing tag constants

## Step 7: Delete FFIFuncRefs and NaN-Boxed Symbol Registration

**Owner**: Agent team (1 agent)
**Files**: `shape-jit/src/ffi_refs.rs`, `shape-jit/src/compiler/ffi_builder.rs`, `ffi_symbols/`
**Tracking**: [ ] Complete

### Sub-tasks
- [ ] Delete `ffi_refs.rs` (current ~200 field struct of NaN-boxed FuncRefs)
- [ ] Create new `v2_ffi_refs.rs` with only v2 typed FuncRefs (~25 fields)
- [ ] Rewrite `ffi_builder.rs` to build only v2 typed refs
- [ ] Delete all `*_symbols.rs` files that register NaN-boxed functions
- [ ] Keep only `v2_*_symbols.rs` files

### Acceptance
- FFIFuncRefs has ~25 fields (down from ~185)
- All Cranelift function signatures use native types
- Zero I64-typed NaN-boxed function declarations

## Execution Order

```
Step 1 (compiler)     ─┐
Step 2 (VM stack)     ─┼── Parallel, independent
Step 4 (FFI rewrite)  ─┘
         │
Step 3 (VM handlers)  ── Depends on Step 1 + 2
Step 5 (MirToIR)      ── Depends on Step 4
         │
Step 6 (delete)       ── Depends on ALL above
Step 7 (cleanup)      ── Depends on Step 6
```

## Verification Gate

After ALL steps:
- `cargo check --workspace` passes
- `cargo test -p shape-vm --lib` passes (1470+ tests)
- `cargo test -p shape-jit --lib` passes (200+ tests)
- `cargo test -p shape-value --lib` passes (390+ tests)
- Suessco solver runs correctly in JIT mode
- `grep -rn "NanBox\|nan_box\|TAG_NULL\|TAG_BOOL\|ValueWord" crates/` returns ZERO results (excluding comments/docs)
