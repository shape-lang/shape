// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     jit_box(HK_ARRAY, ...) — jit_control_map, jit_control_filter
//   Category B (intermediate/consumed): 3 sites
//     Vec::with_capacity for args in jit_call_value, jit_call_foreign_impl,
//       jit_call_foreign_native_args_fixed (consumed within call, not escaped)
//     Arc::new in error path of jit_call_foreign_impl (returned as ValueWord)
//   Category C (heap islands): 0 sites (jit_control_map results — fixed via write barrier)
//!
//! Control Flow FFI Functions for JIT
//!
//! Higher-order functions (fold, reduce, map, filter, forEach) and function call helpers
//! for JIT-compiled code.

use crate::context::{JITClosure, JITContext};
use crate::ffi::object::conversion::{jit_bits_to_nanboxed_with_ctx, nanboxed_to_jit_bits};
use crate::jit_array::JitArray;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use shape_runtime::module_exports::RawCallableInvoker;
use shape_value::{ValueWord, ValueWordExt};
use std::ffi::c_void;
use std::sync::Arc;

// ============================================================================
// Trampoline VM — thread-local VirtualMachine for JIT-to-VM fallback
// ============================================================================

use std::cell::Cell;

thread_local! {
    /// Pointer to a fully-initialized VirtualMachine for executing bytecode
    /// functions that weren't JIT-compiled. Set by `execute_with_jit()` before
    /// JIT execution and cleared after. Valid only on the executor thread.
    static TRAMPOLINE_VM: Cell<*mut shape_vm::VirtualMachine> = const { Cell::new(std::ptr::null_mut()) };
}

/// Register the trampoline VM for use during JIT execution.
///
/// # Safety
/// The pointer must remain valid for the entire duration of JIT execution.
/// Caller must clear it with `unset_trampoline_vm()` after execution.
pub unsafe fn set_trampoline_vm(vm: *mut shape_vm::VirtualMachine) {
    TRAMPOLINE_VM.with(|cell| cell.set(vm));
}

/// Clear the trampoline VM pointer after JIT execution.
pub fn unset_trampoline_vm() {
    TRAMPOLINE_VM.with(|cell| cell.set(std::ptr::null_mut()));
}

/// Access the trampoline VM for read-only queries (schema lookups, etc.)
pub fn with_trampoline_vm<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&shape_vm::VirtualMachine) -> R,
{
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            None
        } else {
            Some(f(unsafe { &*vm_ptr }))
        }
    })
}

/// Execute a closure with mutable access to the trampoline VM.
pub fn with_trampoline_vm_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut shape_vm::VirtualMachine) -> R,
{
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            None
        } else {
            Some(f(unsafe { &mut *vm_ptr }))
        }
    })
}

/// Dispatch a function call through the trampoline VM for functions that
/// aren't JIT-compiled (null entries in the function table).
///
/// `upvalue_bits` carries the closure's captures when the callee is a
/// closure (either VM-format heap or unified-heap `JITClosure`). When the
/// callee is a bare function (TAG_FUNCTION inline), pass `None` to dispatch
/// through `call_value_immediate_nb` with a plain function ValueWord.
///
/// When captures are present we route through `jit_trampoline_call_closure`
/// on the interpreter side, which binds them to the callee frame's
/// upvalues exactly as the `op_call_closure` path does. Without this
/// path, a closure that fails JIT compilation (null entry in the function
/// table) would be reconstructed as a bare function, losing its captures
/// and producing `Null` on return.
fn dispatch_call_via_trampoline_vm(
    function_id: u32,
    upvalue_bits: Option<&[u64]>,
    jit_args: &[u64],
    jit_ctx: *const JITContext,
) -> u64 {
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                eprintln!(
                    "[jit-trampoline] fn_id={} BAIL: trampoline VM not registered",
                    function_id
                );
            }
            return TAG_NULL;
        }
        let vm = unsafe { &mut *vm_ptr };

        // Convert JIT NaN-boxed args to ValueWord
        let args: Vec<shape_value::ValueWord> = jit_args
            .iter()
            .map(|&bits| jit_bits_to_nanboxed_with_ctx(bits, jit_ctx))
            .collect();

        if let Some(caps) = upvalue_bits {
            // Closure dispatch: bind captures as the callee frame's
            // upvalues. The raw bits are passed through verbatim —
            // Immutable captures carry widened ValueWord bits, while
            // OwnedMutable / Shared captures carry raw cell pointer bits
            // that the interpreter's Load/Store*Capture opcodes recover.
            let debug = std::env::var_os("SHAPE_JIT_DEBUG").is_some();
            match vm.jit_trampoline_call_closure(function_id as u16, caps, &args, None) {
                Ok(bits) => {
                    if debug {
                        eprintln!(
                            "[jit-trampoline-closure] fn_id={} returned {:#x}",
                            function_id, bits
                        );
                    }
                    // `pop_raw_u64` transfers ownership of any Arc refcount
                    // to us. Wrap in a `ValueWord` so its Drop releases
                    // that refcount after `nanboxed_to_jit_bits` has
                    // performed its own ref-management (unified-heap path
                    // bumps the refcount; VM-format path clones fields
                    // it wants to retain).
                    let vw = shape_value::ValueWord::from_raw_bits(bits);
                    nanboxed_to_jit_bits(&vw)
                }
                Err(e) => {
                    if debug {
                        eprintln!("[jit-trampoline-closure] fn_id={} ERROR: {}", function_id, e);
                    }
                    TAG_NULL
                }
            }
        } else {
            // Bare function (TAG_FUNCTION inline, no captures).
            // Use call_value_immediate_nb instead of execute_function_by_id
            // because execute_function_by_id calls reset() which wipes VM
            // state. call_value_immediate_nb preserves existing state for
            // nested calls.
            let callee = shape_value::ValueWord::from_function(function_id as u16);
            match vm.call_value_immediate_nb(&callee, &args, None) {
                Ok(result) => nanboxed_to_jit_bits(&result),
                Err(_) => TAG_NULL,
            }
        }
    })
}

/// Dispatch a native module function call through the trampoline VM.
fn dispatch_module_fn_call(
    module_fn_id: u32,
    jit_args: &[u64],
    ctx: *mut JITContext,
) -> u64 {
    TRAMPOLINE_VM.with(|cell| {
        let vm_ptr = cell.get();
        if vm_ptr.is_null() {
            return TAG_NULL;
        }
        let vm = unsafe { &mut *vm_ptr };
        let debug = std::env::var_os("SHAPE_JIT_DEBUG").is_some();

        // Convert JIT args to ValueWord
        let args: Vec<shape_value::ValueWord> = jit_args
            .iter()
            .map(|&bits| jit_bits_to_nanboxed_with_ctx(bits, ctx as *const JITContext))
            .collect();

        // Build a ValueWord::ModuleFunction callee and call through the VM
        let callee = shape_value::ValueWord::from_module_function(module_fn_id);
        match vm.call_value_immediate_nb(&callee, &args, None) {
            Ok(result) => {
                if debug {
                    eprintln!(
                        "[jit-module-fn] id={} returned {:#x}",
                        module_fn_id,
                        result.raw_bits()
                    );
                }
                // Convert VM-format ValueWord to JIT-format using
                // nanboxed_to_jit_bits which handles Ok/Err wrappers,
                // TypedObjects, HashMaps etc. by converting to JitAlloc format.
                nanboxed_to_jit_bits(&result)
            }
            Err(e) => {
                if debug {
                    eprintln!("[jit-module-fn] id={} ERROR: {}", module_fn_id, e);
                }
                // Wrap errors as Result::Err so `?` operator works correctly
                let err_msg = format!("{}", e);
                let err_vw = shape_value::ValueWord::from_err(
                    shape_value::ValueWord::from_string(std::sync::Arc::new(err_msg)),
                );
                nanboxed_to_jit_bits(&err_vw)
            }
        }
    })
}

/// Call a function by function_id
/// Stack reads args from ctx.stack before the call
pub extern "C" fn jit_call_function(
    ctx: *mut JITContext,
    function_id: u16,
    _args: *const u64, // deprecated, pass null
    _arg_count: usize,
) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Check if we have a function table
        if ctx_ref.function_table.is_null() || (function_id as usize) >= ctx_ref.function_table_len
        {
            return TAG_NULL;
        }

        // Get the function pointer
        let fn_ptr = *ctx_ref.function_table.add(function_id as usize);

        // The function reads its args from the stack (already pushed by caller)
        // and returns result on the stack
        let _result_code = fn_ptr(ctx);

        // Pop result from stack
        if ctx_ref.stack_ptr > 0 {
            ctx_ref.stack_ptr -= 1;
            ctx_ref.stack[ctx_ref.stack_ptr]
        } else {
            TAG_NULL
        }
    }
}

/// Call a closure or function value
/// Stack layout: [callee, arg1, ..., argN, arg_count]
/// Returns the result of the call
pub extern "C" fn jit_call_value(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;
        let debug = std::env::var_os("SHAPE_JIT_DEBUG").is_some();

        if debug {
            eprintln!(
                "[jit-call-value] entry: stack_ptr={}, stack[0..4]=[{:#x}, {:#x}, {:#x}, {:#x}]",
                ctx_ref.stack_ptr,
                if ctx_ref.stack_ptr > 0 { ctx_ref.stack[0] } else { 0 },
                if ctx_ref.stack_ptr > 1 { ctx_ref.stack[1] } else { 0 },
                if ctx_ref.stack_ptr > 2 { ctx_ref.stack[2] } else { 0 },
                if ctx_ref.stack_ptr > 3 { ctx_ref.stack[3] } else { 0 },
            );
        }

        // Pop arg_count.
        // ABI: the MIR producer stores arg_count as a raw i64 (see
        // mir_compiler/terminators.rs CallValue lowering). We decode it directly
        // as usize — do NOT attempt NaN-box decode.
        if ctx_ref.stack_ptr == 0 {
            if debug { eprintln!("[jit-call-value] BAIL: stack_ptr=0 at arg_count pop"); }
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count = ctx_ref.stack[ctx_ref.stack_ptr] as usize;

        // Pop args (in reverse order, then we'll reverse)
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if ctx_ref.stack_ptr == 0 {
                return TAG_NULL;
            }
            ctx_ref.stack_ptr -= 1;
            args.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        args.reverse();

        // Pop callee
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let callee_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        // Check for TAG_MODULE_FN (native module functions like read_text).
        // These are dispatched through the trampoline VM's invoke_module_fn.
        if shape_value::tag_bits::is_tagged(callee_bits)
            && shape_value::tag_bits::get_tag(callee_bits) == shape_value::tag_bits::TAG_MODULE_FN
        {
            let module_fn_id = shape_value::tag_bits::get_payload(callee_bits) as u32;
            return dispatch_module_fn_call(module_fn_id, &args, ctx);
        }

        // Resolve the function_id and any VM-format closure captures.
        //
        // Three callee shapes are supported here:
        //  1. `TAG_FUNCTION` inline: bare function value (no captures).
        //  2. Unified heap `HK_CLOSURE`: v2 `JITClosure` block allocated by
        //     the JIT itself (captures are raw bits in an inline array).
        //  3. VM-format heap: `HeapValue::Closure` / `HeapValue::ClosureRaw`
        //     produced by the interpreter's `op_make_closure`. The JIT sees
        //     these when a bytecode-emitted closure flows into a JIT-compiled
        //     call site — that's the A.1D.2 / A.1E scenario where the
        //     closure body runs natively but was allocated by the VM.
        //
        // For shape 3 we go through `VmClosureHandle` to read function_id
        // and capture_execution_bits, mirroring the interpreter's
        // `op_call_closure`. `capture_execution_bits` returns raw pointer
        // bits for OwnedMutable / Shared captures — identical to what the
        // bytecode's Load/StoreOwnedMutableCapture opcodes expect to see in
        // `frame.upvalues[i]`.
        let mut vm_captures: Option<Vec<u64>> = None;
        let function_id = if is_inline_function(callee_bits) {
            unbox_function_id(callee_bits)
        } else if is_heap_kind(callee_bits, HK_CLOSURE) {
            let closure = unified_unbox::<JITClosure>(callee_bits);
            closure.function_id
        } else if shape_value::ValueBits::from_raw(callee_bits).is_heap()
            && !shape_value::ValueBits::from_raw(callee_bits).is_unified_heap()
        {
            // VM-format heap pointer: decode through ValueWord. Use
            // `clone_from_bits` so the refcount is bumped — the original
            // share on the stack is still live and independent.
            //
            // SAFETY: `callee_bits` was produced by the interpreter's
            // op_make_closure (or a prior VM-format pass) which stores
            // a live Arc<HeapValue> with refcount ≥ 1. Cloning here
            // bumps the count; the clone is dropped at function exit.
            let vw = unsafe { ValueWord::clone_from_bits(callee_bits) };
            let Some(hv) = vw.as_heap_ref() else {
                if debug { eprintln!("[jit-call-value] BAIL: VM-format heap but as_heap_ref() returned None: {:#x}", callee_bits); }
                return TAG_NULL;
            };
            let Some(handle) = hv.as_closure_handle() else {
                if debug { eprintln!("[jit-call-value] BAIL: VM-format heap is not a closure: {:#x}", callee_bits); }
                return TAG_NULL;
            };
            let fid = handle.function_id() as u16;
            let n = handle.capture_count();
            let mut caps: Vec<u64> = Vec::with_capacity(n);
            for i in 0..n {
                // `capture_execution_bits` returns:
                //   - Immutable: the widened ValueWord bit pattern (Arc
                //     refcount already bumped by the handle's live
                //     closure block — no extra retain needed; the
                //     receiving JIT frame does not free these).
                //   - OwnedMutable: raw `*mut ValueWord` pointer bits.
                //   - Shared: raw `*const SharedCell` pointer bits.
                //
                // All three variants are passed straight through to the
                // callee's frame.upvalues, matching interpreter semantics.
                caps.push(handle.capture_execution_bits(i));
            }
            vm_captures = Some(caps);
            // Drop the cloned handle — the captures are already copied.
            // `vw` goes out of scope here, releasing the +1 share we took.
            drop(vw);
            fid
        } else {
            if debug { eprintln!("[jit-call-value] BAIL: callee is neither function nor closure: {:#x}", callee_bits); }
            return TAG_NULL;
        };

        // Look up the function pointer in the function table
        if ctx_ref.function_table.is_null()
            || (function_id as usize) >= ctx_ref.function_table_len
        {
            if debug { eprintln!("[jit-call-value] BAIL: fn_id={} out of bounds (table_len={}, table_null={})", function_id, ctx_ref.function_table_len, ctx_ref.function_table.is_null()); }
            return TAG_NULL;
        }
        let raw_fn_ptr =
            *(ctx_ref.function_table as *const *const u8).add(function_id as usize);
        if raw_fn_ptr.is_null() {
            // Not JIT-compiled — dispatch through trampoline VM.
            //
            // We must pass captures through to the interpreter or the
            // closure would be invoked as a bare function, returning
            // `Null` (this was the pre-fix trampoline-null bug).
            //
            // Three callee shapes:
            //   - TAG_FUNCTION inline: no captures — pass `None`.
            //   - VM-format heap closure: captures live in `vm_captures`
            //     (populated above via `VmClosureHandle`).
            //   - Unified-heap `HK_CLOSURE` (JITClosure): captures live
            //     inline at `closure.captures_ptr` — extract here.
            let unified_caps_storage: Option<Vec<u64>> =
                if vm_captures.is_none() && is_heap_kind(callee_bits, HK_CLOSURE) {
                    let closure = unified_unbox::<JITClosure>(callee_bits);
                    let count = closure.captures_count as usize;
                    let mut v: Vec<u64> = Vec::with_capacity(count);
                    for i in 0..count {
                        v.push(*closure.captures_ptr.add(i));
                    }
                    Some(v)
                } else {
                    None
                };
            let upvalues: Option<&[u64]> = vm_captures
                .as_deref()
                .or_else(|| unified_caps_storage.as_deref());
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                eprintln!(
                    "[jit-call-value] function {} NOT JIT-compiled, dispatching through trampoline VM (upvalues={})",
                    function_id,
                    upvalues.map(|c| c.len() as isize).unwrap_or(-1),
                );
            }
            let result = dispatch_call_via_trampoline_vm(
                function_id as u32,
                upvalues,
                &args,
                ctx as *const JITContext,
            );
            // Drop any VM-format capture storage now that the call
            // returned. The VmClosureHandle share was released when the
            // original ValueWord clone went out of scope above.
            drop(vm_captures);
            drop(unified_caps_storage);
            return result;
        }

        // Reset ctx.stack_ptr before calling — the callee's internal operations
        // (BuiltinCall, CallForeign, etc.) use ctx.stack and assume it starts
        // at a consistent state. Without this reset, stale stack_ptr from
        // previous operations causes writes to wrong positions.
        ctx_ref.stack_ptr = 0;

        // For closure calls, prepend captured values as leading native args.
        // The JIT-compiled function signature is (ctx, capture0, ..., captureN, param0, ..., paramM)
        // matching the VM calling convention where bytecode does LoadLocal/StoreLocal
        // for captures first, then params.
        let full_args;
        let call_args: &[u64] = if let Some(caps) = vm_captures.as_ref() {
            // Shape 3: VM-format closure. Captures were extracted via
            // `VmClosureHandle::capture_execution_bits` above.
            if debug {
                eprintln!(
                    "[jit-call-value] vm-closure fn_id={}: prepending {} captures before {} args",
                    function_id, caps.len(), args.len()
                );
            }
            full_args = {
                let mut v = Vec::with_capacity(caps.len() + args.len());
                v.extend_from_slice(caps);
                v.extend_from_slice(&args);
                v
            };
            &full_args
        } else if is_heap_kind(callee_bits, HK_CLOSURE) {
            // Shape 2: unified-heap JITClosure. Captures live inline at
            // `closure.captures_ptr`; read directly without crossing the
            // VM-format boundary.
            let closure = unified_unbox::<JITClosure>(callee_bits);
            let count = closure.captures_count as usize;
            if debug {
                eprintln!(
                    "[jit-call-value] closure fn_id={}: prepending {} captures before {} args",
                    closure.function_id, count, args.len()
                );
            }
            full_args = {
                let mut v = Vec::with_capacity(count + args.len());
                for i in 0..count {
                    v.push(*closure.captures_ptr.add(i));
                }
                v.extend_from_slice(&args);
                v
            };
            &full_args
        } else {
            // Shape 1: inline TAG_FUNCTION — no captures.
            &args
        };

        if debug {
            eprintln!(
                "[jit-call-value] calling fn_id={} with {} args, fn_ptr={:?}, table_len={}",
                function_id, call_args.len(), raw_fn_ptr, ctx_ref.function_table_len
            );
        }
        // Call the JIT-compiled function with the correct number of native args.
        let signal = call_jit_fn_with_args(raw_fn_ptr, ctx, call_args);
        if debug {
            eprintln!(
                "[jit-call-value] returned signal={}, stack_ptr={}",
                signal, (*ctx).stack_ptr
            );
        }

        // Check for deopt signal
        if signal < 0 {
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                eprintln!(
                    "[jit-call-value] function {} deopted (signal={})",
                    function_id, signal
                );
            }
            return TAG_NULL;
        }

        // Read result from ctx.stack[0] (callee stores it there)
        let result = if ctx_ref.stack_ptr > 0 {
            ctx_ref.stack_ptr -= 1;
            ctx_ref.stack[ctx_ref.stack_ptr]
        } else {
            ctx_ref.stack[0]
        };
        if debug {
            eprintln!(
                "[jit-call-value] result: stack_ptr={} after pop, result={:#x} (f64={})",
                ctx_ref.stack_ptr, result,
                f64::from_bits(result)
            );
        }
        result
    }
}

/// Call a JIT-compiled function pointer with the right number of native arguments.
/// The function has Cranelift signature: fn(ctx_ptr: i64, arg0: i64, ...) -> i32
unsafe fn call_jit_fn_with_args(
    fn_ptr: *const u8,
    ctx: *mut JITContext,
    args: &[u64],
) -> i32 {
    type F0 = unsafe extern "C" fn(*mut JITContext) -> i32;
    type F1 = unsafe extern "C" fn(*mut JITContext, u64) -> i32;
    type F2 = unsafe extern "C" fn(*mut JITContext, u64, u64) -> i32;
    type F3 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64) -> i32;
    type F4 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64) -> i32;
    type F5 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64) -> i32;
    type F6 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64) -> i32;
    type F7 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64, u64) -> i32;
    type F8 = unsafe extern "C" fn(*mut JITContext, u64, u64, u64, u64, u64, u64, u64, u64) -> i32;

    let result = match args.len() {
        0 => std::mem::transmute::<_, F0>(fn_ptr)(ctx),
        1 => std::mem::transmute::<_, F1>(fn_ptr)(ctx, args[0]),
        2 => std::mem::transmute::<_, F2>(fn_ptr)(ctx, args[0], args[1]),
        3 => std::mem::transmute::<_, F3>(fn_ptr)(ctx, args[0], args[1], args[2]),
        4 => std::mem::transmute::<_, F4>(fn_ptr)(ctx, args[0], args[1], args[2], args[3]),
        5 => std::mem::transmute::<_, F5>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4]),
        6 => std::mem::transmute::<_, F6>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5]),
        7 => std::mem::transmute::<_, F7>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6]),
        8 => std::mem::transmute::<_, F8>(fn_ptr)(ctx, args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7]),
        _ => {
            // Too many args for direct dispatch — fall back to trampoline
            -1
        }
    };
    result
}

/// fold(array, initial, fn) - left fold over array
/// Stack layout: [array, fn, initial, arg_count=3]
pub extern "C" fn jit_control_fold(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop initial value
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let initial = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop callback
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let callback = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_NULL;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        let mut accumulator = initial;
        for (index, &value) in elements.iter().enumerate() {
            // Call: callback(accumulator, value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = callback;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = accumulator;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 3u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            accumulator = jit_call_value(ctx);
        }

        accumulator
    }
}

/// reduce(array, fn, initial) - reduce array to single value
/// Stack layout: [array, fn, initial, arg_count=3]
pub extern "C" fn jit_control_reduce(ctx: *mut JITContext) -> u64 {
    // reduce is the same as fold
    jit_control_fold(ctx)
}

/// map(array, fn) - transform each element
/// Stack layout: [array, fn, arg_count=2]
pub extern "C" fn jit_control_map(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop callback
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let callback = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_NULL;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        let mut results = Vec::with_capacity(elements.len());
        for (index, &value) in elements.iter().enumerate() {
            // Call: callback(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = callback;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            results.push(result);
        }

        // Write barrier: notify GC that result array contains callback heap refs
        for &r in &results {
            crate::ffi::gc::jit_write_barrier(0, r);
        }
        JitArray::from_vec(results).heap_box()
    }
}

/// filter(array, predicate) - keep elements where predicate returns true
/// Stack layout: [array, predicate, arg_count=2]
pub extern "C" fn jit_control_filter(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop predicate
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let predicate = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_NULL;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        let mut results = Vec::new();
        for (index, &value) in elements.iter().enumerate() {
            // Call: predicate(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            if result == TAG_BOOL_TRUE {
                results.push(value);
            }
        }

        JitArray::from_vec(results).heap_box()
    }
}

/// forEach(array, fn, count) - execute fn for each element (side effects)
/// Stack layout: [array, fn, count=2]
pub extern "C" fn jit_control_foreach(ctx: *mut JITContext, _count: usize) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Pop callback
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let callback = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_NULL;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        for (index, &value) in elements.iter().enumerate() {
            // Call: callback(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = callback;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let _result = jit_call_value(ctx);
        }

        TAG_NULL // forEach returns null/unit
    }
}

/// find(array, predicate) - find first element matching predicate
/// Stack layout: [array, predicate, arg_count=2]
pub extern "C" fn jit_control_find(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop predicate
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let predicate = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_NULL;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        for (index, &value) in elements.iter().enumerate() {
            // Call: predicate(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            if result == TAG_BOOL_TRUE {
                return value;
            }
        }

        TAG_NULL // Not found
    }
}

unsafe fn jit_callable_invoker(
    ctx: *mut c_void,
    callable: &ValueWord,
    args: &[ValueWord],
) -> Result<ValueWord, String> {
    if ctx.is_null() {
        return Err("native callback invoker received null JIT context".to_string());
    }

    let jit_ctx = unsafe { &mut *(ctx as *mut JITContext) };
    let base_sp = jit_ctx.stack_ptr;
    let needed = args.len().saturating_add(2); // callee + args + arg_count
    if base_sp.saturating_add(needed) > jit_ctx.stack.len() {
        return Err("native callback exceeded JIT stack capacity".to_string());
    }

    jit_ctx.stack[jit_ctx.stack_ptr] = nanboxed_to_jit_bits(callable);
    jit_ctx.stack_ptr += 1;
    for arg in args {
        jit_ctx.stack[jit_ctx.stack_ptr] = nanboxed_to_jit_bits(arg);
        jit_ctx.stack_ptr += 1;
    }
    jit_ctx.stack[jit_ctx.stack_ptr] = args.len() as u64; // arg_count (raw i64 ABI)
    jit_ctx.stack_ptr += 1;

    let result_bits = jit_call_value(jit_ctx as *mut JITContext);
    jit_ctx.stack_ptr = base_sp;
    Ok(jit_bits_to_nanboxed_with_ctx(
        result_bits,
        jit_ctx as *const JITContext,
    ))
}

/// Invoke a linked foreign function from JIT code.
///
/// Args are read from `ctx.stack` (already materialized by lowering):
/// `[... arg0, arg1, ..., argN-1]` with `arg_count` provided out-of-band.
enum ForeignInvokeMode {
    Any,
    NativeOnly,
    DynamicOnly,
}

unsafe fn jit_call_foreign_impl(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
    mode: ForeignInvokeMode,
) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let ctx_ref = unsafe { &mut *ctx };
    if ctx_ref.foreign_bridge_ptr.is_null() {
        return TAG_NULL;
    }
    if ctx_ref.stack_ptr < arg_count {
        return TAG_NULL;
    }

    let args_start = ctx_ref.stack_ptr - arg_count;
    let mut args = Vec::with_capacity(arg_count);
    for idx in args_start..ctx_ref.stack_ptr {
        args.push(jit_bits_to_nanboxed_with_ctx(
            ctx_ref.stack[idx],
            ctx as *const JITContext,
        ));
    }
    ctx_ref.stack_ptr = args_start;

    let bridge = unsafe {
        &*(ctx_ref.foreign_bridge_ptr as *const crate::foreign_bridge::JitForeignBridgeState)
    };
    let raw_invoker = RawCallableInvoker {
        ctx: ctx as *mut c_void,
        invoke: jit_callable_invoker,
    };

    let result = match mode {
        ForeignInvokeMode::Any => bridge.invoke(foreign_idx as usize, &args, Some(raw_invoker)),
        ForeignInvokeMode::NativeOnly => {
            bridge.invoke_native(foreign_idx as usize, &args, Some(raw_invoker))
        }
        ForeignInvokeMode::DynamicOnly => bridge.invoke_dynamic(foreign_idx as usize, &args),
    };

    match result {
        Ok(result) => nanboxed_to_jit_bits(&result),
        Err(err) => {
            let err_nb = ValueWord::from_err(ValueWord::from_string(Arc::new(err)));
            nanboxed_to_jit_bits(&err_nb)
        }
    }
}

pub extern "C" fn jit_call_foreign(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::Any) }
}

pub extern "C" fn jit_call_foreign_native(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::NativeOnly) }
}

pub extern "C" fn jit_call_foreign_dynamic(
    ctx: *mut JITContext,
    foreign_idx: u32,
    arg_count: usize,
) -> u64 {
    unsafe { jit_call_foreign_impl(ctx, foreign_idx, arg_count, ForeignInvokeMode::DynamicOnly) }
}

unsafe fn jit_call_foreign_native_args_fixed<const N: usize>(
    ctx: *mut JITContext,
    foreign_idx: u32,
    args: [u64; N],
) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let ctx_ref = unsafe { &mut *ctx };
    if ctx_ref.foreign_bridge_ptr.is_null() {
        return TAG_NULL;
    }

    let bridge = unsafe {
        &*(ctx_ref.foreign_bridge_ptr as *const crate::foreign_bridge::JitForeignBridgeState)
    };
    let raw_invoker = RawCallableInvoker {
        ctx: ctx as *mut c_void,
        invoke: jit_callable_invoker,
    };
    let boxed_args: [ValueWord; N] = std::array::from_fn(|idx| {
        jit_bits_to_nanboxed_with_ctx(args[idx], ctx as *const JITContext)
    });

    match bridge.invoke_native(foreign_idx as usize, &boxed_args, Some(raw_invoker)) {
        Ok(result) => nanboxed_to_jit_bits(&result),
        Err(err) => {
            let err_nb = ValueWord::from_err(ValueWord::from_string(Arc::new(err)));
            nanboxed_to_jit_bits(&err_nb)
        }
    }
}

macro_rules! define_jit_call_foreign_native_fixed {
    ($name:ident, [$($arg:ident),*]) => {
        pub extern "C" fn $name(
            ctx: *mut JITContext,
            foreign_idx: u32,
            $($arg: u64),*
        ) -> u64 {
            unsafe { jit_call_foreign_native_args_fixed(ctx, foreign_idx, [$($arg),*]) }
        }
    };
}

define_jit_call_foreign_native_fixed!(jit_call_foreign_native_0, []);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_1, [arg0]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_2, [arg0, arg1]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_3, [arg0, arg1, arg2]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_4, [arg0, arg1, arg2, arg3]);
define_jit_call_foreign_native_fixed!(jit_call_foreign_native_5, [arg0, arg1, arg2, arg3, arg4]);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_6,
    [arg0, arg1, arg2, arg3, arg4, arg5]
);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_7,
    [arg0, arg1, arg2, arg3, arg4, arg5, arg6]
);
define_jit_call_foreign_native_fixed!(
    jit_call_foreign_native_8,
    [arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7]
);

/// Trampoline placeholder for mixed-table VM fallback paths.
///
/// When implemented, this will dispatch to the VM interpreter for functions
/// that weren't JIT-compiled. The return value from the VM is in ValueWord
/// format, so it must be converted to JIT format via `vm_result_to_jit`.
pub unsafe extern "C" fn jit_vm_fallback_trampoline(
    _ctx: *mut std::ffi::c_void,
    _function_id: u32,
    _args_ptr: *const u64,
    _args_len: u32,
) -> u64 {
    // TODO: when implemented, convert result via vm_result_to_jit():
    //   let vm_result = /* dispatch to VM interpreter */;
    //   crate::ffi::object::conversion::vm_result_to_jit(vm_result)
    TAG_NULL
}

/// findIndex(array, predicate) - find index of first element matching predicate
/// Stack layout: [array, predicate, arg_count=2]
pub extern "C" fn jit_control_find_index(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return box_number(-1.0);
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return box_number(-1.0);
        }
        ctx_ref.stack_ptr -= 1;

        // Pop predicate
        if ctx_ref.stack_ptr == 0 {
            return box_number(-1.0);
        }
        ctx_ref.stack_ptr -= 1;
        let predicate = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return box_number(-1.0);
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return box_number(-1.0);
        }

        let elements = JitArray::from_heap_bits(array_bits);

        for (index, &value) in elements.iter().enumerate() {
            // Call: predicate(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            if result == TAG_BOOL_TRUE {
                return box_number(index as f64);
            }
        }

        box_number(-1.0) // Not found
    }
}

/// some(array, predicate) - true if any element matches predicate
/// Stack layout: [array, predicate, arg_count=2]
pub extern "C" fn jit_control_some(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_BOOL_FALSE;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop predicate
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;
        let predicate = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_BOOL_FALSE;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        for (index, &value) in elements.iter().enumerate() {
            // Call: predicate(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            if result == TAG_BOOL_TRUE {
                return TAG_BOOL_TRUE;
            }
        }

        TAG_BOOL_FALSE
    }
}

/// every(array, predicate) - true if all elements match predicate
/// Stack layout: [array, predicate, arg_count=2]
pub extern "C" fn jit_control_every(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_BOOL_FALSE;
        }
        let ctx_ref = &mut *ctx;

        // Pop arg_count
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;

        // Pop predicate
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;
        let predicate = ctx_ref.stack[ctx_ref.stack_ptr];

        // Pop array
        if ctx_ref.stack_ptr == 0 {
            return TAG_BOOL_FALSE;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        if !is_heap_kind(array_bits, HK_ARRAY) {
            return TAG_BOOL_FALSE;
        }

        let elements = JitArray::from_heap_bits(array_bits);

        if elements.is_empty() {
            return TAG_BOOL_TRUE; // Empty array - vacuous truth
        }

        for (index, &value) in elements.iter().enumerate() {
            // Call: predicate(value, index)
            ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = value;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = 2u64; // arg_count (raw i64 ABI)
            ctx_ref.stack_ptr += 1;

            let result = jit_call_value(ctx);
            if result != TAG_BOOL_TRUE {
                return TAG_BOOL_FALSE;
            }
        }

        TAG_BOOL_TRUE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: the MIR CallValue producer stores arg_count as a raw i64
    // (mir_compiler/terminators.rs). jit_call_value must decode it as a raw
    // usize — not via unbox_number. Previously the decode went through
    // unbox_number, which misread 1 as ~5e-324 and truncated to 0, dropping
    // all args and promoting the first arg to the callee slot.
    //
    // This test exercises the decode end-to-end by laying out a stack of
    // [callee, arg1..argN, arg_count] with a non-callable callee (TAG_NULL).
    // After decode, jit_call_value pops arg_count + args + callee and bails
    // at the "neither function nor closure" check. Correct decode leaves
    // stack_ptr at 0; the old (buggy) decode would leave leftover args.
    #[test]
    fn jit_call_value_decodes_arg_count_as_raw_i64() {
        for arg_count in [0usize, 1, 2, 3] {
            let mut ctx = JITContext::default();
            // Layout: [callee, arg0, ..., arg_{N-1}, arg_count]
            ctx.stack[0] = TAG_NULL; // callee — non-callable, triggers BAIL after decode
            for i in 0..arg_count {
                ctx.stack[1 + i] = TAG_NULL;
            }
            // Producer writes arg_count as raw i64 via `iconst(I64, args.len())`.
            ctx.stack[1 + arg_count] = arg_count as u64;
            ctx.stack_ptr = 1 + arg_count + 1;

            let result = jit_call_value(&mut ctx as *mut JITContext);
            assert_eq!(result, TAG_NULL, "arg_count={}: expected BAIL → TAG_NULL", arg_count);
            assert_eq!(
                ctx.stack_ptr, 0,
                "arg_count={}: stack not fully drained (incorrect decode)", arg_count
            );
        }
    }

    #[test]
    fn native_fixed_arity_helpers_return_null_for_null_context() {
        assert_eq!(jit_call_foreign_native_0(std::ptr::null_mut(), 0), TAG_NULL);
        assert_eq!(
            jit_call_foreign_native_1(std::ptr::null_mut(), 0, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_2(std::ptr::null_mut(), 0, TAG_NULL, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_3(std::ptr::null_mut(), 0, TAG_NULL, TAG_NULL, TAG_NULL),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_4(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_5(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_6(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_7(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
        assert_eq!(
            jit_call_foreign_native_8(
                std::ptr::null_mut(),
                0,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL,
                TAG_NULL
            ),
            TAG_NULL
        );
    }
}
