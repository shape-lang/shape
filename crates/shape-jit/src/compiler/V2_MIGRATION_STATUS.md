# V2 Migration Status: NaN-Boxing Audit

**Date**: 2026-04-01
**Scope**: All NaN-boxing usage in `shape-jit/src/compiler/` (MIR-to-IR) and
its transitive dependencies (`translator/`, `ffi/`, `ffi_symbols/`, runtime layer).

## Executive Summary

The `compiler/` directory (MIR-to-IR compiler) has **ZERO NaN-boxing references**.
It is already fully v2-clean. All NaN-boxing lives in the `translator/` (BytecodeToIR)
which the compiler delegates to, plus the FFI/runtime layers that the translator calls.

Total NaN-boxing references across shape-jit:
- `compiler/` (6 files): **0 references** -- CLEAN
- `translator/` (28 files): **242 references**
- `ffi/` (runtime FFI helpers): **~919 references**
- `ffi_symbols/` (symbol registration): **~166 references**
- Root files (`context.rs`, `executor.rs`, `core.rs`, `jit_array.rs`, etc.): **~91 references**
- `nan_boxing.rs` itself: definition file, **~775 lines**

## File-by-File Audit

### compiler/ -- CLEAN (0 references)

| File | NaN-boxing refs | Status |
|------|----------------|--------|
| `mod.rs` | 0 | CLEAN |
| `setup.rs` | 0 | CLEAN |
| `strategy.rs` | 0 | CLEAN |
| `program.rs` | 0 | CLEAN |
| `ffi_builder.rs` | 0 | CLEAN |
| `accessors.rs` | 0 | CLEAN |

The compiler itself constructs `BytecodeToIR` (translator) and calls `.compile()`.
It never touches NaN-boxing directly. The v2 migration boundary is at the
`BytecodeToIR` interface.

### translator/ -- DELETE WITH TRANSLATOR (242 references)

All 242 NaN-boxing references in the translator will be eliminated when the
translator is replaced with a v2 MIR-based code generator.

**Category: DELETE WITH TRANSLATOR**

| File | Refs | Usage |
|------|------|-------|
| `opcodes/builtins/array_builtins.rs` | 26 | TAG_NULL as FFI error sentinel |
| `opcodes/functions.rs` | 18 | TAG_NULL defaults, TAG_BOOL for returns, box_number for FFI args |
| `opcodes/builtins/array.rs` | 18 | TAG_NULL as FFI error sentinel |
| `opcodes/collections.rs` | 17 | TAG_NULL for array init, sentinels, error values |
| `opcodes/hof_inline.rs` | 16 | TAG_NULL/TAG_BOOL for filter/find/some/every results |
| `compiler_tests.rs` | 16 | SlotKind::NanBoxed in deopt metadata tests |
| `opcodes/control_flow.rs` | 14 | TAG_NULL/TAG_BOOL for if/else/null-coalescing |
| `helpers.rs` | 13 | TAG_NULL defaults, TAG_BOOL for comparisons, NanBoxed tracking |
| `opcodes/builtins/math.rs` | 12 | TAG_NULL as FFI error fallback |
| `opcodes/control_flow_loops.rs` | 10 | box_number/TAG_BOOL/TAG_NULL for loop constant folding |
| `storage.rs` | 9 | CraneliftRepr::NanBoxed enum variant and mappings |
| `opcodes/stack.rs` | 9 | TAG_NULL/TAG_BOOL for PushNull/PushBool/literal encoding |
| `compiler.rs` | 9 | TAG_NULL defaults, SlotKind::NanBoxed for deopt |
| `opcodes/control_flow_extras.rs` | 8 | TAG_NULL/TAG_BOOL for match/guard/optional paths |
| `opcodes/control_flow_result_ops.rs` | 7 | TAG_NULL/TAG_BOOL for Result/Option unwrap |
| `inline_ops.rs` | 7 | TAG_NULL/TAG_BOOL for inline operations |
| `opcodes/data.rs` | 6 | TAG_NULL for out-of-bounds/error returns |
| `typed.rs` | 6 | TAG_BOOL boxing/unboxing, CraneliftRepr::NanBoxed |
| `helpers_numeric_ops.rs` | 4 | TAG_BOOL for comparison results |
| `opcodes/variables.rs` | 3 | CraneliftRepr::NanBoxed fallback |
| `opcodes/builtins/types.rs` | 3 | TAG_BOOL for type check results |
| `opcodes/arithmetic.rs` | 3 | TAG_BOOL for comparison results |
| `osr_compiler.rs` | 2 | TAG_NULL for OSR local init |
| `opcodes/builtins/control.rs` | 2 | TAG_NULL for error fallback |
| `opcodes/builtins/mod.rs` | 1 | box_number for arg count |
| `opcodes/typed_objects.rs` | 1 | use crate::nan_boxing::* |
| `opcodes/speculative.rs` | 1 | use crate::nan_boxing::* |
| `opcodes/shape_guards.rs` | 1 | use crate::nan_boxing::* |

### ffi/ -- BOUNDARY ONLY (919 references)

These are Rust `extern "C"` functions that the JIT calls via Cranelift `call` instructions.
They accept and return NaN-boxed `u64` values because the translator ABI requires it.

**Category: BOUNDARY ONLY / DELETE WITH TRANSLATOR**

When the v2 compiler uses typed FFI (passing f64/i64/bool directly), these functions
will need typed alternatives. Many already have partial typed variants.

Key files: `ffi/array.rs` (~100 refs), `ffi/call_method/` (~300 refs),
`ffi/typed_object/` (~150 refs), `ffi/iterator.rs` (~30 refs), `ffi/join.rs` (~10 refs).

### ffi_symbols/ -- BOUNDARY ONLY (166 references)

Symbol registration and dispatch for JIT-callable functions.
Same boundary story as `ffi/`.

Key files: `ffi_symbols/intrinsics/mod.rs` (~50 refs), `ffi_symbols/vector/mod.rs` (~40 refs),
`ffi_symbols/helpers/mod.rs` (~25 refs), `ffi_symbols/simulation/mod.rs` (~15 refs),
`ffi_symbols/data_access/mod.rs` (~15 refs), `ffi_symbols/series/mod.rs` (~15 refs).

### Runtime layer files -- NEEDS V2 INFRASTRUCTURE

| File | Refs | Category |
|------|------|----------|
| `context.rs` | ~25 | NEEDS V2 INFRASTRUCTURE -- JITContext locals/stack use TAG_NULL as zero-init |
| `executor.rs` | ~10 | NEEDS V2 INFRASTRUCTURE -- result unmarshaling from JIT uses TAG_NULL/TAG_BOOL |
| `core.rs` | ~15 | BOUNDARY ONLY -- integration tests, arc_retain/release |
| `jit_array.rs` | ~12 | NEEDS V2 INFRASTRUCTURE -- array helpers using TAG_BOOL for bool detection |
| `foreign_bridge.rs` | ~10 | BOUNDARY ONLY -- ValueWord in foreign function bridge |
| `async_symbols.rs` | ~1 | BOUNDARY ONLY -- TAG_NULL as suspension sentinel |
| `nan_boxing.rs` | entire file | DEFINITION FILE -- keeps as long as translator exists |

### SlotKind::NanBoxed in shape-vm

`SlotKind::NanBoxed` (defined in `shape-vm/src/type_tracking.rs`) is used in
deopt metadata to tell the interpreter "these bits are a raw NaN-boxed value,
transmute directly." This is part of the JIT-to-interpreter deopt ABI and cannot
be eliminated until the interpreter also moves to v2 value representation.

**Category: NEEDS V2 INFRASTRUCTURE** (interpreter + JIT deopt ABI change)

## What is "ELIMINABLE NOW"?

Given that the `compiler/` directory is already clean and the translator is
the legacy path, there is nothing to eliminate in the MIR-to-IR compiler itself.

However, within the translator, the following patterns are **already handled
by v2 typed paths** when type info is available:

1. **TAG_BOOL_TRUE/FALSE for typed comparisons**: The translator already has
   `CraneliftRepr::I8` for bools and `typed.rs` has boxing/unboxing code.
   When types are proven, the I8 path is used. The NaN-boxed path is the
   fallback for dynamic types.

2. **TAG_NULL as default value**: Used when the translator doesn't know the type.
   When types are known, `CraneliftRepr::F64` uses `0.0` and `CraneliftRepr::I64`
   uses `0`. This is already correct.

3. **box_number() for constant encoding**: Used in the dynamic (untyped) path.
   Typed paths use raw f64/i64 constants directly.

In short: the translator already generates optimal code when types are known.
The NaN-boxing references are the fallback paths for dynamic/unknown types,
which are correct behavior for the legacy translator.

## Steps to Achieve Zero NaN-Boxing in MIR-to-IR

The `compiler/` directory is already at zero. The full path to zero NaN-boxing
across all of shape-jit is:

### Phase 1: Already Done
- [x] `compiler/` directory is NaN-boxing-free
- [x] Typed paths in translator use native F64/I64/I8 representations
- [x] `CraneliftRepr` enum supports F64, I64, I8, Series alongside NanBoxed

### Phase 2: Replace Translator with v2 MIR Codegen
- [ ] Implement MIR-based code generator that works on typed MIR (not bytecode)
- [ ] MIR already has type info -- no need for NaN-boxed fallback paths
- [ ] This deletes the entire `translator/` directory (~28 files, 242 NaN-boxing refs)

### Phase 3: Typed FFI Layer
- [ ] Create typed FFI functions that accept/return native types (f64, i64, bool, *const T)
- [ ] Wire v2 MIR codegen to call typed FFI instead of NaN-boxed FFI
- [ ] This eliminates `ffi/` and `ffi_symbols/` NaN-boxing (~1085 refs)

### Phase 4: Runtime Layer
- [ ] Migrate `JITContext` to use typed slots instead of `[TAG_NULL; 256]`
- [ ] Migrate `executor.rs` result unmarshaling to typed returns
- [ ] Migrate `jit_array.rs` to work with TypedArray<T> instead of NaN-boxed arrays
- [ ] This eliminates runtime layer NaN-boxing (~91 refs)

### Phase 5: Deopt ABI
- [ ] Migrate interpreter to v2 value representation
- [ ] Replace `SlotKind::NanBoxed` in deopt metadata with typed alternatives
- [ ] Delete `nan_boxing.rs` definition file

## Risk Assessment

**No changes needed in `compiler/` -- it's already clean.**

The translator's NaN-boxing usage is correct for its role as the legacy
bytecode-to-IR path. Attempting to eliminate NaN-boxing from the translator
without replacing it with a typed MIR codegen would break the dynamic type
fallback paths and is not recommended.

The safe path forward is: build the v2 MIR codegen (Phase 2), then delete
the translator wholesale.
