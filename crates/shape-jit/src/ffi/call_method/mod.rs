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
            jit_unbox::<String>(method_bits).clone()
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
                    if method_name == "reduce" {
                        let initial = if args.len() > 1 {
                            args[1]
                        } else {
                            box_number(0.0)
                        };
                        // Push: [array, callback, initial, arg_count=3]
                        ctx_ref.stack[ctx_ref.stack_ptr] = working_array_bits;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
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
                                    jit_unbox::<String>(key_result).clone()
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
                            jit_box(HK_JIT_OBJECT, obj)
                        }
                        _ => TAG_NULL,
                    };

                    return result;
                }
                _ => {}
            }
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

        builtin_result
    }
}
