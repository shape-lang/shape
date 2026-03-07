//! V8 exception -> Shape error string conversion.

use crate::runtime::CompiledFunction;

/// Extract a human-readable error string from a V8 exception value.
///
/// Attempts to pull the `message` property and `stack` trace from the
/// exception object. Falls back to a `.to_string()` representation if
/// the exception is not an Error object.
pub fn map_v8_exception(
    scope: &mut deno_core::v8::HandleScope,
    exception: deno_core::v8::Local<deno_core::v8::Value>,
) -> String {
    let exception_obj = if exception.is_object() {
        exception.to_object(scope)
    } else {
        None
    };

    // Try to read .stack first (includes message + trace)
    if let Some(obj) = &exception_obj {
        let stack_key = deno_core::v8::String::new(scope, "stack").unwrap();
        if let Some(stack_val) = obj.get(scope, stack_key.into()) {
            if stack_val.is_string() {
                let stack_str = stack_val.to_rust_string_lossy(scope);
                if !stack_str.is_empty() {
                    return stack_str;
                }
            }
        }
    }

    // Fall back to .message
    if let Some(obj) = &exception_obj {
        let msg_key = deno_core::v8::String::new(scope, "message").unwrap();
        if let Some(msg_val) = obj.get(scope, msg_key.into()) {
            if msg_val.is_string() {
                let msg_str = msg_val.to_rust_string_lossy(scope);
                if !msg_str.is_empty() {
                    return msg_str;
                }
            }
        }
    }

    // Last resort: stringify the value
    exception.to_rust_string_lossy(scope)
}

/// Format a V8 error with context from the compiled function metadata.
///
/// Prefixes the V8 error message with the Shape function name for
/// easier debugging when multiple foreign blocks exist.
pub fn format_ts_error(
    scope: &mut deno_core::v8::HandleScope,
    exception: deno_core::v8::Local<deno_core::v8::Value>,
    func: &CompiledFunction,
) -> String {
    let raw = map_v8_exception(scope, exception);
    format!("TypeScript error in '{}': {}", func.name, raw)
}

/// Format an error string with the compiled function context (no V8 scope needed).
pub fn format_ts_error_str(func: &CompiledFunction, msg: &str) -> String {
    format!("TypeScript error in '{}': {}", func.name, msg)
}
