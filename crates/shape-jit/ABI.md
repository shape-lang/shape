# VM-JIT Call ABI v1

## Convention

1. Caller (VM) checks `TierManager::get_native_code(func_id)` for a compiled function pointer.
2. Caller creates a `JITContext` on the stack.
3. Caller reads arg count from `program.functions[func_id].arity` (canonical source).
4. Caller writes arguments to `ctx.locals[0..arity]` using typed marshaling
   when a `FrameDescriptor` is available (see Typed Argument Marshaling below),
   falling back to NaN-boxed `u64` passthrough for unknown-type slots.
5. Caller invokes `jit_fn(ctx_ptr as *mut u8, null_ptr)` via `extern "C"`.
6. Callee returns a `u64`:
   - If `result != u64::MAX`: success. The return value is the NaN-boxed result.
   - If `result == u64::MAX`: deopt requested (fall back to interpreter).
7. On success, caller converts the returned `u64` back to a `ValueWord` and pushes it.
8. On deopt, caller restores args to the VM stack and falls through to the
   bytecode interpreter path.

## Function Pointer Type

```rust
pub type JitFnPtr = unsafe extern "C" fn(*mut u8, *const u8) -> u64;
```

Defined in `shape-vm/src/executor/mod.rs` behind `#[cfg(feature = "jit")]`.

## Deopt Sentinel

The deopt sentinel is `u64::MAX` (`0xFFFF_FFFF_FFFF_FFFF`). This bit pattern
is not a valid NaN-boxed value (it falls in the unused NaN-box tag space), so
it is unambiguous.

## Type Encoding

Arguments and return values use NaN-boxed `u64` encoding identical to the
interpreter's `ValueWord` format (`#[repr(transparent)] struct ValueWord(u64)`).

`JITContext.locals` and `JITContext.stack` use raw `u64` with the same encoding.

### Typed Argument Marshaling (Pillar 4)

When a callee function has a `FrameDescriptor` with known `SlotKind`s, the VM
marshals arguments using typed encoding instead of raw NaN-boxed passthrough:

| SlotKind           | Ingress (VM -> JIT)                | Egress (JIT -> VM)             |
|--------------------|------------------------------------|---------------------------------|
| Int64/IntSize      | Extract i64, store as raw bits     | Raw bits -> `ValueWord::from_f64(i as f64)` |
| Float64            | NaN-boxed passthrough              | NaN-boxed passthrough          |
| Bool               | Extract bool, store as 0/1         | 0/1 -> `ValueWord::from_bool`  |
| Unknown / other    | NaN-boxed passthrough              | NaN-boxed passthrough          |

The fallback is **always** NaN-boxed passthrough, never synthetic None/null.

Return values use the `FrameDescriptor.return_kind` field. When `Unknown`, the
existing NaN-boxed passthrough behavior is preserved (zero overhead).

Implementation: `shape-vm/src/executor/control_flow/jit_abi.rs`.

## Tiered Promotion

| Tier | Threshold | Description |
|------|-----------|-------------|
| 0 (Interpreted) | 0 | All functions start here |
| 1 (BaselineJit) | 100 calls | Per-function compilation, no cross-function opt |
| 2 (OptimizingJit) | 10,000 calls | Inlining, constant propagation, devirt |

Promotion is managed by `TierManager` in `shape-vm/src/tier.rs`. The VM polls
for completed compilations every 1024 instructions (piggy-backing on the
existing interrupt check cadence).

## OSR (On-Stack Replacement) Entry

OSR allows the VM to transfer execution from the interpreter into JIT-compiled
code mid-function, specifically at hot loop headers.

1. VM detects a hot loop via back-edge counter exceeding threshold.
2. VM sends `CompilationRequest { osr: true, loop_header_ip: Some(ip), .. }`
   to the background compilation thread.
3. Background thread compiles only the loop body via `compile_osr_loop()`,
   producing an `OsrCompilationResult` with native code, `OsrEntryPoint`
   metadata, and `DeoptInfo` for any guard points within the loop.
4. At the next loop back-edge, the VM:
   a. Snapshots live locals from the interpreter frame.
   b. Marshals them to `JITContext.locals` using `OsrEntryPoint.local_kinds`
      and the same typed marshaling rules as function entry (see Typed
      Argument Marshaling above).
   c. Invokes the OSR entry function (signature: `OsrEntryFn`, same as
      `JittedStrategyFn`).
   d. On success (return 0): reads modified locals back from `JITContext.locals`,
      unmarshals them to interpreter `ValueWord`s, and continues
      interpretation at `OsrEntryPoint.exit_ip`.
   e. On deopt (return `i32::MIN + 1`): reads locals from `JITContext.locals`,
      looks up the `DeoptInfo` for the failing guard, and resumes
      interpretation at `DeoptInfo.resume_ip`.

### OSR Data Structures

- `OsrEntryPoint` (in `shape-vm/src/bytecode/core_types.rs`): metadata for
  one loop header -- bytecode IP, live locals, slot kinds, exit IP.
- `OsrCompilationResult` (in `shape-jit/src/translator/compiler.rs`): native
  code pointer + entry point + deopt points.
- `CompilationRequest.osr` / `CompilationRequest.loop_header_ip`: flag and
  target IP for OSR compilation requests.
- `CompilationResult.osr_entry` / `CompilationResult.deopt_points`: metadata
  returned alongside compiled native code.

## Full Deoptimization

When a compiled function must fall back to the interpreter mid-execution
(e.g., a type guard fails), the following protocol applies:

1. JIT code detects the guard failure (e.g., value is not an integer when
   the optimized path assumed integer).
2. JIT code stores current locals to `JITContext.locals`.
3. JIT code returns the deopt signal: `i32::MIN + 1` (for `JittedStrategyFn`
   return type) or `u64::MAX` (for raw `JitFnPtr` return type).
4. VM reads the `DeoptInfo` for the specific guard point (identified by
   the deopt point index or resume IP).
5. VM reconstructs the interpreter frame from JIT locals using
   `DeoptInfo.local_mapping` (JIT index -> bytecode index) and
   `DeoptInfo.local_kinds` (for unmarshaling).
6. VM resumes bytecode interpretation at `DeoptInfo.resume_ip`.

### DeoptInfo Structure

```
DeoptInfo {
    resume_ip: usize,              // Bytecode IP to resume at
    local_mapping: Vec<(u16, u16)>, // (jit_local_idx, bytecode_local_idx)
    local_kinds: Vec<SlotKind>,     // Parallel to local_mapping
    stack_depth: u16,              // Operand stack depth at this point
}
```

### Guard Emission

During compilation, `BytecodeToIR::emit_deopt_point()` records a `DeoptInfo`
for each type guard. The accumulated deopt points are returned via
`take_deopt_points()` and attached to the `CompilationResult`.

## Safety

- `JitFnPtr` raw pointers are valid for the lifetime of the JIT compilation
  and are only accessed from the VM thread.
- `TierManager` is `Send` but not `Sync` — it is owned by a single
  `VirtualMachine` instance.
- The background compilation thread communicates via `mpsc` channels.
- `OsrCompilationResult` is `Send` but not `Sync` -- native code pointers
  are only accessed from the VM thread.
