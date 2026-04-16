// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     jit_box(HK_JIT_OBJECT, ...) — group/groupBy result object
//     jit_box(HK_ARRAY, ...) — group values inside object
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 1 site (group/groupBy)
//!
//! Method Call FFI Functions for JIT
//!
//! Dispatches method calls on various types (array, string, object, series, etc.)
//! Split into type-specific helper modules for maintainability.

use crate::context::JITContext;
use crate::jit_array::JitArray;
use crate::nan_boxing::*;
use shape_runtime::context::ExecutionContext;
use std::collections::HashMap;
use shape_value::ValueWordExt;

// Module declarations
pub mod array;
pub mod duration;
pub mod matrix;
pub mod number;
pub mod object;
pub mod result;
pub mod signal_builder;
pub mod string;
pub mod time;

// Re-export the individual method handlers
pub use array::call_array_method;
pub use duration::call_duration_method;
pub use matrix::call_matrix_method;
pub use number::call_number_method;
pub use object::call_object_method;
pub use result::call_result_method;
pub use signal_builder::call_signalbuilder_method;
pub use string::call_string_method;
pub use time::call_time_method;

// ============================================================================
// User-Defined Method Support
// ============================================================================

/// Determine the type name of a JIT NaN-boxed receiver value.
///
/// For TypedObjects, uses the schema_id to look up the type name from the
/// ExecutionContext's type_schema_registry. For other types, returns a static
/// type name string.
unsafe fn receiver_type_name(receiver_bits: u64, exec_ctx: &ExecutionContext) -> Option<String> {
    use crate::ffi::typed_object::jit_typed_object_schema_id;

    if is_number(receiver_bits) {
        return Some("number".to_string());
    }
    if receiver_bits == TAG_BOOL_TRUE || receiver_bits == TAG_BOOL_FALSE {
        return Some("bool".to_string());
    }
    if receiver_bits == TAG_NULL || receiver_bits == TAG_NONE {
        return None;
    }

    match heap_kind(receiver_bits) {
        Some(HK_STRING) => Some("string".to_string()),
        Some(HK_ARRAY) => Some("Array".to_string()),
        Some(HK_TYPED_OBJECT) => {
            // Look up the schema name from the type_schema_registry
            let schema_id = jit_typed_object_schema_id(receiver_bits);
            if schema_id == 0 {
                return None;
            }
            let registry = exec_ctx.type_schema_registry();
            registry.get_by_id(schema_id).map(|s| s.name.clone())
        }
        Some(HK_JIT_OBJECT) => Some("object".to_string()),
        Some(HK_DURATION) => Some("Duration".to_string()),
        Some(HK_TIME) => Some("DateTime".to_string()),
        _ => None,
    }
}

/// Search the JITContext's function_names table for a function with the given
/// UFCS name (e.g. "Point::distance") and return its index.
unsafe fn find_function_by_name(ctx_ref: &JITContext, ufcs_name: &str) -> Option<usize> {
    if ctx_ref.function_names_ptr.is_null() || ctx_ref.function_names_len == 0 {
        return None;
    }
    let names = unsafe {
        std::slice::from_raw_parts(ctx_ref.function_names_ptr, ctx_ref.function_names_len)
    };
    for (idx, name) in names.iter().enumerate() {
        if name == ufcs_name {
            return Some(idx);
        }
    }
    None
}

/// Try to call a user-defined method from impl blocks via UFCS dispatch.
///
/// User-defined methods (from `extend` / `impl` blocks) are compiled as functions
/// named `"TypeName::method_name"`. This function:
/// 1. Determines the receiver type name from the NaN-boxed bits
/// 2. Constructs the UFCS name `"TypeName::method_name"`
/// 3. Looks up the function index in function_names
/// 4. Calls the function via function_table, passing (receiver, ...args)
/// 5. Returns the result as NaN-boxed u64
///
/// Returns Some(result) if the method was found and executed, None otherwise.
unsafe fn try_call_user_method(
    ctx: *const JITContext,
    receiver_bits: u64,
    method_name: &str,
    args: &[u64],
) -> Option<u64> {
    let ctx_ref = unsafe { &*ctx };

    // Need execution context to access the type schema registry
    if ctx_ref.exec_context_ptr.is_null() {
        return None;
    }
    let exec_ctx = unsafe { &*(ctx_ref.exec_context_ptr as *const ExecutionContext) };

    // Determine the receiver's type name
    let type_name = unsafe { receiver_type_name(receiver_bits, exec_ctx) }?;

    // Construct UFCS function name: "TypeName::method_name"
    let ufcs_name = format!("{}::{}", type_name, method_name);

    // Look up the function index in the JIT function table
    let func_idx = unsafe { find_function_by_name(ctx_ref, &ufcs_name) }?;

    // Check that we have a valid function table entry
    if ctx_ref.function_table.is_null() || func_idx >= ctx_ref.function_table_len {
        return None;
    }

    // Read the raw pointer from the function table. A null entry means the
    // function was not JIT-compiled (interpreted only).
    let raw_fn_ptr = unsafe { *(ctx_ref.function_table as *const *const u8).add(func_idx) };
    if raw_fn_ptr.is_null() {
        return None;
    }
    let fn_ptr = unsafe { *ctx_ref.function_table.add(func_idx) };

    // Push receiver + args onto the JIT stack for the function call.
    // UFCS convention: first parameter is `self` (the receiver), then the rest.
    let ctx_mut = unsafe { &mut *(ctx as *mut JITContext) };
    ctx_mut.stack[ctx_mut.stack_ptr] = receiver_bits;
    ctx_mut.stack_ptr += 1;
    for &arg in args {
        ctx_mut.stack[ctx_mut.stack_ptr] = arg;
        ctx_mut.stack_ptr += 1;
    }

    // Call the JIT-compiled function
    let _result_code = unsafe { fn_ptr(ctx_mut) };

    // Pop result from stack
    if ctx_mut.stack_ptr > 0 {
        ctx_mut.stack_ptr -= 1;
        Some(ctx_mut.stack[ctx_mut.stack_ptr])
    } else {
        Some(TAG_NULL)
    }
}

// ============================================================================
// Main Dispatcher
// ============================================================================

/// Call a method on a value
/// Stack layout at call: [receiver, arg1, ..., argN, method_name, arg_count]
/// The FFI pops values from ctx.stack and dispatches to the appropriate method
/// Dispatch a method call through the trampoline VM for receivers
/// that the JIT's built-in method dispatch doesn't handle (VM-format
/// HashMaps, TypedObjects, etc.).
fn dispatch_method_via_trampoline(
    receiver_bits: u64,
    method_name: &str,
    args: &[u64],
    ctx: *mut JITContext,
) -> Option<u64> {
    use crate::ffi::object::conversion::{jit_bits_to_nanboxed_with_ctx, nanboxed_to_jit_bits};

    crate::ffi::control::with_trampoline_vm_mut(|vm| {
        // Guard: only dispatch VM-format heap values through the trampoline.
        // Unified heap values (bit-47 set) are JIT-native objects that should have
        // been handled by the built-in dispatch above.
        if shape_value::tags::is_unified_heap(receiver_bits) {
            return TAG_NULL; // JIT object without a matching method
        }
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            let hk = heap_kind(receiver_bits);
            eprintln!("[trampoline-method] receiver={:#x} heap_kind={:?} method={}", receiver_bits, hk, method_name);
        }
        let receiver_vw = unsafe { shape_value::ValueWord::clone_from_bits(receiver_bits) };

        // Args come from JIT-compiled code and might be JIT or VM format.
        // Use jit_bits_to_nanboxed for conversion.
        let args_vw: Vec<shape_value::ValueWord> = args
            .iter()
            .map(|&bits| jit_bits_to_nanboxed_with_ctx(bits, ctx as *const JITContext))
            .collect();

        // Handle common methods on VM-allocated types directly.
        if let Some(hm) = receiver_vw.as_hashmap_data() {
            return match method_name {
                "get" => {
                    if let Some(key) = args_vw.first() {
                        if let Some(idx) = hm.find_key(key) {
                            nanboxed_to_jit_bits(&hm.values[idx])
                        } else {
                            TAG_NULL
                        }
                    } else {
                        TAG_NULL
                    }
                }
                "set" => {
                    if args_vw.len() >= 2 {
                        let key = args_vw[0].clone();
                        let val = args_vw[1].clone();
                        let mut new_keys = hm.keys.clone();
                        let mut new_values = hm.values.clone();
                        if let Some(idx) = hm.find_key(&key) {
                            new_values[idx] = val;
                        } else {
                            new_keys.push(key);
                            new_values.push(val);
                        }
                        let new_hm = shape_value::ValueWord::from_hashmap_pairs(
                            new_keys, new_values,
                        );
                        nanboxed_to_jit_bits(&new_hm)
                    } else {
                        TAG_NULL
                    }
                }
                "has" => {
                    if let Some(key) = args_vw.first() {
                        if hm.find_key(key).is_some() {
                            crate::nan_boxing::TAG_BOOL_TRUE
                        } else {
                            crate::nan_boxing::TAG_BOOL_FALSE
                        }
                    } else {
                        crate::nan_boxing::TAG_BOOL_FALSE
                    }
                }
                "keys" => {
                    nanboxed_to_jit_bits(&shape_value::ValueWord::from_array(
                        std::sync::Arc::new(hm.keys.clone()),
                    ))
                }
                "values" => {
                    nanboxed_to_jit_bits(&shape_value::ValueWord::from_array(
                        std::sync::Arc::new(hm.values.clone()),
                    ))
                }
                "length" | "len" | "size" => {
                    crate::nan_boxing::box_number(hm.keys.len() as f64)
                }
                _ => TAG_NULL,
            };
        }

        // TypedObject methods
        if let Some((_schema_id, _slots, _heap_mask)) = receiver_vw.as_typed_object() {
            // TypedObject methods are dispatched through the VM
            // TODO: add common method handling
        }

        // Generic fallback: dispatch through the trampoline VM's method dispatch.
        // This handles DataTable.column(), .toArray(), and any other VM-format
        // object methods not explicitly handled above.
        {
            // Push: [receiver, arg0, ..., argN, method_name, arg_count]
            if vm.push_raw_u64(receiver_vw.clone()).is_ok() {
                let mut push_ok = true;
                for arg in &args_vw {
                    if vm.push_raw_u64(arg.clone()).is_err() {
                        push_ok = false;
                        break;
                    }
                }
                if push_ok {
                    let method_vw = shape_value::ValueWord::from_string(
                        std::sync::Arc::new(method_name.to_string()),
                    );
                    let arg_count_vw = shape_value::ValueWord::from_f64(args_vw.len() as f64);
                    if vm.push_raw_u64(method_vw).is_ok() && vm.push_raw_u64(arg_count_vw).is_ok() {
                        // Use legacy stack-based calling convention (operand: None)
                        // so method name is read from the stack, not the trampoline VM's
                        // string table.
                        let instr = shape_vm::bytecode::Instruction {
                            opcode: shape_vm::bytecode::OpCode::CallMethod,
                            operand: None,
                        };
                        match vm.op_call_method(&instr, None) {
                            Ok(()) => {
                                if let Ok(result) = vm.pop_raw_u64() {
                                    return nanboxed_to_jit_bits(&result);
                                }
                            }
                            Err(e) => {
                                if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                                    eprintln!("[trampoline-method-vm] op_call_method FAILED: {} method={}", e, method_name);
                                }
                            }
                        }
                    }
                }
            }
        }

        TAG_NULL
    })
}

pub extern "C" fn jit_call_method(ctx: *mut JITContext, stack_count: usize) -> u64 {
    unsafe {
        if ctx.is_null() || stack_count < 3 {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;

        // Pop arg_count from stack (number)
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let arg_count = if is_number(arg_count_bits) {
            unbox_number(arg_count_bits) as usize
        } else {
            return TAG_NULL;
        };

        // Pop method_name from stack (string)
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let method_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let method_name = if is_heap_kind(method_bits, HK_STRING) {
            unbox_string(method_bits).to_string()
        } else {
            return method_bits; // Return non-string value as-is
        };
        // Pop args from stack
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if ctx_ref.stack_ptr == 0 {
                return TAG_NULL;
            }
            ctx_ref.stack_ptr -= 1;
            args.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        args.reverse(); // Restore original order

        // Pop receiver from stack
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let receiver_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        // Special-case higher-order methods that need callback execution
        // Handle both arrays and series
        if is_heap_kind(receiver_bits, HK_ARRAY) {
            match method_name.as_str() {
                "find" | "findIndex" | "some" | "every" | "filter" | "map" | "count" | "group"
                | "groupBy" | "reduce" => {
                    // These methods need callback execution via control functions
                    if args.is_empty() {
                        return TAG_NULL;
                    }
                    let predicate = args[0]; // The callback function

                    let working_array_bits = receiver_bits;

                    // Handle reduce separately (needs initial value)
                    // Shape syntax: arr.reduce(initial, callback)
                    // So args[0] = initial, args[1] = callback
                    if method_name == "reduce" {
                        let (callback, initial) = if args.len() > 1 {
                            (args[1], args[0])
                        } else {
                            // Single arg — treat it as callback with default initial
                            (args[0], box_number(0.0))
                        };
                        // Push: [array, callback, initial, arg_count=3]
                        ctx_ref.stack[ctx_ref.stack_ptr] = working_array_bits;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = callback;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = initial;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = box_number(3.0);
                        ctx_ref.stack_ptr += 1;
                        return super::control::jit_control_reduce(ctx);
                    }

                    // Push array onto stack for other operations
                    ctx_ref.stack[ctx_ref.stack_ptr] = working_array_bits;
                    ctx_ref.stack_ptr += 1;
                    // Push predicate onto stack
                    ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
                    ctx_ref.stack_ptr += 1;
                    // Push arg_count onto stack
                    ctx_ref.stack[ctx_ref.stack_ptr] = box_number(2.0);
                    ctx_ref.stack_ptr += 1;

                    let result = match method_name.as_str() {
                        "find" => super::control::jit_control_find(ctx),
                        "findIndex" => super::control::jit_control_find_index(ctx),
                        "some" => super::control::jit_control_some(ctx),
                        "every" => super::control::jit_control_every(ctx),
                        "filter" => super::control::jit_control_filter(ctx),
                        "map" => super::control::jit_control_map(ctx),
                        "count" => {
                            // count(pred) = filter(pred).length
                            let filtered = super::control::jit_control_filter(ctx);
                            if is_heap_kind(filtered, HK_ARRAY) {
                                let arr = JitArray::from_heap_bits(filtered);
                                return box_number(arr.len() as f64);
                            }
                            box_number(0.0)
                        }
                        "group" | "groupBy" => {
                            // group(keyFn) - groups elements by the result of keyFn
                            let elements = JitArray::from_heap_bits(working_array_bits);

                            let mut groups: HashMap<String, Vec<u64>> = HashMap::new();

                            for (index, &value) in elements.iter().enumerate() {
                                // Call predicate to get the key
                                ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
                                ctx_ref.stack_ptr += 1;
                                ctx_ref.stack[ctx_ref.stack_ptr] = value;
                                ctx_ref.stack_ptr += 1;
                                ctx_ref.stack[ctx_ref.stack_ptr] = box_number(index as f64);
                                ctx_ref.stack_ptr += 1;
                                ctx_ref.stack[ctx_ref.stack_ptr] = box_number(2.0);
                                ctx_ref.stack_ptr += 1;

                                let key_result = super::control::jit_call_value(ctx);

                                // Convert key to string for HashMap
                                let key = if is_heap_kind(key_result, HK_STRING) {
                                    unbox_string(key_result).to_string()
                                } else if is_number(key_result) {
                                    format!("{}", unbox_number(key_result))
                                } else if key_result == TAG_BOOL_TRUE {
                                    "true".to_string()
                                } else if key_result == TAG_BOOL_FALSE {
                                    "false".to_string()
                                } else {
                                    "null".to_string()
                                };

                                groups.entry(key).or_default().push(value);
                            }

                            // AUDIT(C5): heap island — each group's Vec<u64> is jit_box'd
                            // into a JitAlloc<JitArray>, then that u64 is stored as a
                            // HashMap value. The HashMap itself is then jit_box'd into
                            // a JitAlloc<HashMap>. The inner JitArray allocations escape
                            // into the HashMap without GC tracking.
                            // When GC feature enabled, route through gc_allocator.
                            let mut obj: HashMap<String, u64> = HashMap::new();
                            for (key, values) in groups {
                                obj.insert(key, JitArray::from_vec(values).heap_box());
                            }
                            unified_box(HK_JIT_OBJECT, obj)
                        }
                        _ => TAG_NULL,
                    };

                    return result;
                }
                _ => {}
            }
        }

        // Try VM-allocated object methods first (HashMap.get, TypedObject methods, etc.)
        // These are Arc<HeapValue> values that the JIT's heap_kind check misidentifies.
        // Check before built-in dispatch to avoid reading garbage from non-JitAlloc headers.
        if shape_value::tags::is_tagged(receiver_bits)
            && shape_value::tags::get_tag(receiver_bits) == shape_value::tags::TAG_HEAP
            && !shape_value::tags::is_unified_heap(receiver_bits)
        {
            let vw = unsafe { shape_value::ValueWord::clone_from_bits(receiver_bits) };
            if let Some(hm) = vw.as_hashmap_data() {
                match method_name.as_str() {
                    "get" => {
                        if let Some(key) = args.first() {
                            // Convert key from JIT format (unified heap) to VM format
                            let key_vw = super::object::conversion::jit_bits_to_nanboxed(*key);
                            if let Some(idx) = hm.find_key(&key_vw) {
                                let result = super::object::conversion::nanboxed_to_jit_bits(&hm.values[idx]);
                                return result;
                            }
                        }
                        return TAG_NULL;
                    }
                    "keys" => {
                        return super::object::conversion::nanboxed_to_jit_bits(
                            &shape_value::ValueWord::from_array(std::sync::Arc::new(hm.keys.clone())),
                        );
                    }
                    "length" | "len" | "size" => {
                        return box_number(hm.keys.len() as f64);
                    }
                    // For other methods (set, has, values, entries, etc.),
                    // fall through to the trampoline VM dispatch below.
                    _ => {}
                }
            }
            // VM-format heap value that's not a handled HashMap method.
            // Skip built-in dispatch (heap_kind reads garbage for Arc<HeapValue>)
            // and go directly to the trampoline VM dispatch.
            if let Some(result) = dispatch_method_via_trampoline(
                receiver_bits, &method_name, &args, ctx,
            ) {
                return result;
            }
            return TAG_NULL;
        }

        // Try built-in methods first
        // Check for Result types (Ok/Err) before the heap kind match since they use sub-tags
        let builtin_result = if is_ok_tag(receiver_bits) || is_err_tag(receiver_bits) {
            call_result_method(receiver_bits, &method_name, &args)
        } else if is_number(receiver_bits) {
            call_number_method(receiver_bits, &method_name, &args)
        } else if is_inline_function(receiver_bits) {
            TAG_NULL // Functions don't have methods
        } else {
            match heap_kind(receiver_bits) {
                Some(HK_ARRAY) => call_array_method(receiver_bits, &method_name, &args),
                Some(HK_STRING) => call_string_method(receiver_bits, &method_name, &args),
                Some(HK_JIT_OBJECT) => call_object_method(receiver_bits, &method_name, &args),
                Some(HK_DURATION) => call_duration_method(receiver_bits, &method_name, &args),
                Some(HK_COLUMN_REF) => TAG_NULL, // Series type removed
                Some(HK_MATRIX) => call_matrix_method(receiver_bits, &method_name, &args),
                Some(HK_TIME) => call_time_method(receiver_bits, &method_name, &args),
                Some(HK_JIT_SIGNAL_BUILDER) => {
                    call_signalbuilder_method(receiver_bits, &method_name, &args)
                }
                _ => TAG_NULL,
            }
        };

        // If built-in method returned NULL, try user-defined methods from TypeMethodRegistry
        if builtin_result == TAG_NULL {
            if let Some(user_result) = try_call_user_method(ctx, receiver_bits, &method_name, &args)
            {
                return user_result;
            }
        }

        // If still NULL, try dispatching through the trampoline VM.
        // This handles VM-allocated objects (HashMap, TypedObject, etc.)
        // that the JIT's built-in method dispatch doesn't recognize.
        if builtin_result == TAG_NULL {
            if let Some(result) = dispatch_method_via_trampoline(
                receiver_bits, &method_name, &args, ctx,
            ) {
                return result;
            }
        }

        builtin_result
    }
}
