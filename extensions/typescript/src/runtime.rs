//! V8/TypeScript runtime management via deno_core.
//!
//! This module owns the V8 isolate lifecycle and implements the core
//! LanguageRuntime operations: init, compile, invoke, dispose.

use crate::marshaling;
use shape_abi_v1::{LanguageRuntimeLspConfig, PluginError};
use std::collections::HashMap;
use std::ffi::c_void;

/// Opaque handle to a compiled TypeScript function.
pub struct CompiledFunction {
    /// The function name in Shape.
    pub name: String,
    /// Generated JavaScript source for the wrapper function.
    /// This is the transpiled TS wrapped in a callable form.
    pub js_source: String,
    /// Parameter names in call order.
    pub param_names: Vec<String>,
    /// Shape source line where the foreign block body starts (for error mapping).
    pub shape_body_start_line: u32,
    /// Whether the function was declared `async` in Shape.
    pub is_async: bool,
    /// Declared return type string from Shape (e.g. "Result<int>").
    pub return_type: String,
    /// The name of the global wrapper function registered in V8.
    pub v8_fn_name: String,
}

/// The TypeScript runtime instance. One per `init()` call.
///
/// Wraps a `deno_core::JsRuntime` that embeds V8 with TypeScript
/// transpilation support via deno_core's built-in facilities.
pub struct TsRuntime {
    /// The deno_core JS runtime (owns the V8 isolate).
    js_runtime: deno_core::JsRuntime,
    /// Compiled function handles, keyed by an incrementing ID.
    functions: HashMap<usize, CompiledFunction>,
    /// Next handle ID.
    next_id: usize,
    /// Reusable tokio runtime for async calls. Created once on first async
    /// invocation and reused for all subsequent calls, avoiding the overhead
    /// of building a new runtime per call.
    tokio_runtime: Option<tokio::runtime::Runtime>,
}

impl TsRuntime {
    /// Initialize a new TypeScript runtime backed by V8.
    ///
    /// `_config_msgpack` is the MessagePack-encoded configuration from the
    /// host. Currently unused -- reserved for future settings like
    /// tsconfig overrides, module resolution paths, etc.
    pub fn new(_config_msgpack: &[u8]) -> Result<Self, String> {
        let js_runtime = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
            ..Default::default()
        });

        Ok(TsRuntime {
            js_runtime,
            functions: HashMap::new(),
            next_id: 1,
            tokio_runtime: None,
        })
    }

    /// Register Shape type schemas for TypeScript declaration generation.
    ///
    /// The runtime receives the full set of Shape types so it can generate
    /// TypeScript interface declarations that the user's code can reference.
    pub fn register_types(&mut self, _types_msgpack: &[u8]) -> Result<(), String> {
        // Stub: the real implementation will deserialize TypeSchemaExport[]
        // and generate TypeScript interface declarations injected into the
        // runtime's global scope.
        Ok(())
    }

    /// Compile a foreign function body into a callable JavaScript function.
    ///
    /// Wraps the user's TypeScript body in a JavaScript function definition
    /// (deno_core handles TS->JS transpilation). The wrapper is evaluated
    /// in the V8 isolate so it can be called later via `invoke()`.
    ///
    /// When `is_async` is false, wraps the user's body in:
    /// ```js
    /// function __shape_ts_<id>(param1, param2) {
    ///     <body>
    /// }
    /// ```
    ///
    /// When `is_async` is true, wraps in an async function:
    /// ```js
    /// async function __shape_ts_<id>(param1, param2) {
    ///     <body>
    /// }
    /// ```
    ///
    /// Returns a handle that can be passed to `invoke()`.
    pub fn compile(
        &mut self,
        name: &str,
        source: &str,
        param_names: &[String],
        _param_types: &[String],
        return_type: &str,
        is_async: bool,
    ) -> Result<*mut c_void, String> {
        let id = self.next_id;
        self.next_id += 1;

        let v8_fn_name = format!("__shape_ts_{id}");
        let params_str = param_names.join(", ");

        // Indent the user body by 4 spaces.
        let indented_body: String = source
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let js_source = if is_async {
            format!("async function {v8_fn_name}({params_str}) {{\n{indented_body}\n}}")
        } else {
            format!("function {v8_fn_name}({params_str}) {{\n{indented_body}\n}}")
        };

        // Evaluate the function definition in V8 so it is available for later calls.
        self.js_runtime
            .execute_script("<shape-ts-compile>", js_source.clone())
            .map_err(|e| format!("TypeScript compilation error in '{}': {}", name, e))?;

        let func = CompiledFunction {
            name: name.to_string(),
            js_source,
            param_names: param_names.to_vec(),
            shape_body_start_line: 0,
            is_async,
            return_type: return_type.to_string(),
            v8_fn_name,
        };

        self.functions.insert(id, func);

        // The handle is the function ID cast to a pointer.
        Ok(id as *mut c_void)
    }

    /// Invoke a previously compiled function with msgpack-encoded arguments.
    ///
    /// Deserializes args from msgpack, calls the V8 function, and serializes
    /// the result back to msgpack.
    pub fn invoke(&mut self, handle: *mut c_void, args_msgpack: &[u8]) -> Result<Vec<u8>, String> {
        let id = handle as usize;
        let func = self
            .functions
            .get(&id)
            .ok_or_else(|| format!("invalid function handle: {id}"))?;

        let v8_fn_name = func.v8_fn_name.clone();
        let func_name = func.name.clone();
        let is_async = func.is_async;

        // Deserialize msgpack args to rmpv values first, before entering V8 scope.
        let arg_values: Vec<rmpv::Value> = if args_msgpack.is_empty() {
            Vec::new()
        } else {
            rmp_serde::from_slice(args_msgpack)
                .map_err(|e| format!("Failed to deserialize args: {}", e))?
        };

        // Build a JS expression that calls the function with serialized arguments.
        // We pass args by building a JS literal expression from the rmpv values.
        let args_js = arg_values
            .iter()
            .map(|v| rmpv_to_js_literal(v))
            .collect::<Vec<_>>()
            .join(", ");

        let call_expr = if is_async {
            // For async functions, wrap in an immediately-invoked async context.
            // deno_core's execute_script returns a value, but async functions return
            // a Promise. We need to use the async runtime to resolve it.
            format!("(async () => await {v8_fn_name}({args_js}))()")
        } else {
            format!("{v8_fn_name}({args_js})")
        };

        if is_async {
            // Use the tokio runtime that deno_core manages internally.
            let result = self
                .js_runtime
                .execute_script("<shape-ts-invoke>", call_expr)
                .map_err(|e| format!("TypeScript error in '{}': {}", func_name, e))?;

            // For async, we need to poll the event loop to resolve the promise.
            // Lazily create a tokio runtime and cache it so subsequent async
            // calls reuse the same runtime instead of building a new one each time.
            let rt = self.tokio_runtime.get_or_insert(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create async runtime: {}", e))?,
            );

            let resolved = rt.block_on(async {
                let resolved = self.js_runtime.resolve(result);
                self.js_runtime
                    .with_event_loop_promise(resolved, deno_core::PollEventLoopOptions::default())
                    .await
            });

            let global = resolved
                .map_err(|e| format!("TypeScript async error in '{}': {}", func_name, e))?;

            // Convert the resolved value to msgpack
            let scope = &mut self.js_runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, global);
            marshaling::v8_to_msgpack(scope, local)
        } else {
            let result = self
                .js_runtime
                .execute_script("<shape-ts-invoke>", call_expr)
                .map_err(|e| format!("TypeScript error in '{}': {}", func_name, e))?;

            let scope = &mut self.js_runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, result);
            marshaling::v8_to_msgpack(scope, local)
        }
    }

    /// Dispose a compiled function handle, removing it from V8.
    pub fn dispose_function(&mut self, handle: *mut c_void) {
        let id = handle as usize;
        if let Some(func) = self.functions.remove(&id) {
            // Delete the global function from V8 to free resources.
            let delete_script = format!("delete globalThis.{};", func.v8_fn_name);
            let _ = self
                .js_runtime
                .execute_script("<shape-ts-dispose>", delete_script);
        }
    }

    /// Return the language identifier.
    pub fn language_id() -> &'static str {
        "typescript"
    }

    /// Return LSP configuration for TypeScript.
    pub fn lsp_config() -> LanguageRuntimeLspConfig {
        LanguageRuntimeLspConfig {
            language_id: "typescript".into(),
            server_command: vec!["typescript-language-server".into(), "--stdio".into()],
            file_extension: ".ts".into(),
            extra_paths: Vec::new(),
        }
    }
}

/// Convert an rmpv::Value to a JavaScript literal string.
///
/// This is used to inline argument values into the call expression.
fn rmpv_to_js_literal(value: &rmpv::Value) -> String {
    match value {
        rmpv::Value::Nil => "null".to_string(),
        rmpv::Value::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                n.to_string()
            } else if let Some(n) = i.as_u64() {
                n.to_string()
            } else {
                "0".to_string()
            }
        }
        rmpv::Value::F32(f) => format!("{}", f),
        rmpv::Value::F64(f) => format!("{}", f),
        rmpv::Value::String(s) => {
            if let Some(s) = s.as_str() {
                format!("\"{}\"", escape_js_string(s))
            } else {
                "null".to_string()
            }
        }
        rmpv::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(rmpv_to_js_literal).collect();
            format!("[{}]", items.join(", "))
        }
        rmpv::Value::Map(entries) => {
            let pairs: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    let key_str = match k {
                        rmpv::Value::String(s) => {
                            if let Some(s) = s.as_str() {
                                format!("\"{}\"", escape_js_string(s))
                            } else {
                                "\"\"".to_string()
                            }
                        }
                        _ => rmpv_to_js_literal(k),
                    };
                    format!("{}: {}", key_str, rmpv_to_js_literal(v))
                })
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        rmpv::Value::Binary(b) => {
            // Encode as a Uint8Array literal
            let items: Vec<String> = b.iter().map(|byte| byte.to_string()).collect();
            format!("new Uint8Array([{}])", items.join(", "))
        }
        rmpv::Value::Ext(_, _) => "null".to_string(),
    }
}

/// Escape a string for inclusion in a JavaScript string literal.
fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

// ============================================================================
// C ABI callback functions (wired from lib.rs vtable)
// ============================================================================

pub unsafe extern "C" fn ts_init(config: *const u8, config_len: usize) -> *mut c_void {
    let config_slice = if config.is_null() || config_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(config, config_len) }
    };

    match TsRuntime::new(config_slice) {
        Ok(runtime) => Box::into_raw(Box::new(runtime)) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

pub unsafe extern "C" fn ts_register_types(
    instance: *mut c_void,
    types_msgpack: *const u8,
    types_len: usize,
) -> i32 {
    if instance.is_null() {
        return PluginError::NotInitialized as i32;
    }
    let runtime = unsafe { &mut *(instance as *mut TsRuntime) };
    let types_slice = if types_msgpack.is_null() || types_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(types_msgpack, types_len) }
    };

    match runtime.register_types(types_slice) {
        Ok(()) => PluginError::Success as i32,
        Err(_) => PluginError::InternalError as i32,
    }
}

pub unsafe extern "C" fn ts_compile(
    instance: *mut c_void,
    name: *const u8,
    name_len: usize,
    source: *const u8,
    source_len: usize,
    param_names_msgpack: *const u8,
    param_names_len: usize,
    param_types_msgpack: *const u8,
    param_types_len: usize,
    return_type: *const u8,
    return_type_len: usize,
    is_async: bool,
    out_error: *mut *mut u8,
    out_error_len: *mut usize,
) -> *mut c_void {
    if instance.is_null() {
        return std::ptr::null_mut();
    }
    let runtime = unsafe { &mut *(instance as *mut TsRuntime) };

    let name_str = match str_from_raw(name, name_len) {
        Some(s) => s,
        None => {
            write_error(out_error, out_error_len, "invalid function name");
            return std::ptr::null_mut();
        }
    };
    let source_str = match str_from_raw(source, source_len) {
        Some(s) => s,
        None => {
            write_error(out_error, out_error_len, "invalid source text");
            return std::ptr::null_mut();
        }
    };
    let return_type_str = match str_from_raw(return_type, return_type_len) {
        Some(s) => s,
        None => "_",
    };

    let param_names: Vec<String> = if param_names_msgpack.is_null() || param_names_len == 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(param_names_msgpack, param_names_len) };
        match rmp_serde::from_slice(slice) {
            Ok(v) => v,
            Err(_) => {
                write_error(out_error, out_error_len, "invalid param names msgpack");
                return std::ptr::null_mut();
            }
        }
    };

    let param_types: Vec<String> = if param_types_msgpack.is_null() || param_types_len == 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(param_types_msgpack, param_types_len) };
        match rmp_serde::from_slice(slice) {
            Ok(v) => v,
            Err(_) => {
                write_error(out_error, out_error_len, "invalid param types msgpack");
                return std::ptr::null_mut();
            }
        }
    };

    match runtime.compile(
        name_str,
        source_str,
        &param_names,
        &param_types,
        return_type_str,
        is_async,
    ) {
        Ok(handle) => handle,
        Err(msg) => {
            write_error(out_error, out_error_len, &msg);
            std::ptr::null_mut()
        }
    }
}

/// Write a UTF-8 error message to out_error/out_error_len for the caller to free.
fn write_error(out_error: *mut *mut u8, out_error_len: *mut usize, msg: &str) {
    if out_error.is_null() || out_error_len.is_null() {
        return;
    }
    let mut bytes = msg.as_bytes().to_vec();
    let len = bytes.len();
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    unsafe {
        *out_error = ptr;
        *out_error_len = len;
    }
}

pub unsafe extern "C" fn ts_invoke(
    instance: *mut c_void,
    handle: *mut c_void,
    args_msgpack: *const u8,
    args_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if instance.is_null() || out_ptr.is_null() || out_len.is_null() {
        return PluginError::InvalidArgument as i32;
    }
    let runtime = unsafe { &mut *(instance as *mut TsRuntime) };
    let args_slice = if args_msgpack.is_null() || args_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args_msgpack, args_len) }
    };

    match runtime.invoke(handle, args_slice) {
        Ok(mut bytes) => {
            let len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            PluginError::Success as i32
        }
        Err(msg) => {
            // Classify the error to return the most appropriate error code:
            // - Marshal/serialization failures -> InvalidArgument
            // - Invalid handle -> InvalidArgument
            // - Everything else (V8/TS exceptions, etc.) -> InternalError
            let error_code = if msg.contains("Failed to deserialize")
                || msg.contains("Failed to serialize")
                || msg.contains("invalid function handle")
            {
                PluginError::InvalidArgument
            } else {
                PluginError::InternalError
            };

            // Write error message to output buffer so the host can read it
            let mut err_bytes = msg.into_bytes();
            let len = err_bytes.len();
            let ptr = err_bytes.as_mut_ptr();
            std::mem::forget(err_bytes);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            error_code as i32
        }
    }
}

pub unsafe extern "C" fn ts_dispose_function(instance: *mut c_void, handle: *mut c_void) {
    if instance.is_null() {
        return;
    }
    let runtime = unsafe { &mut *(instance as *mut TsRuntime) };
    runtime.dispose_function(handle);
}

pub unsafe extern "C" fn ts_language_id(_instance: *mut c_void) -> *const std::ffi::c_char {
    // "typescript\0" -- static, owned by the extension.
    c"typescript".as_ptr()
}

pub unsafe extern "C" fn ts_get_lsp_config(
    _instance: *mut c_void,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return PluginError::InvalidArgument as i32;
    }
    let config = TsRuntime::lsp_config();
    match rmp_serde::to_vec(&config) {
        Ok(mut bytes) => {
            let len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            PluginError::Success as i32
        }
        Err(_) => PluginError::InternalError as i32,
    }
}

pub unsafe extern "C" fn ts_free_buffer(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        let _ = unsafe { Vec::from_raw_parts(ptr, len, len) };
    }
}

pub unsafe extern "C" fn ts_drop(instance: *mut c_void) {
    if !instance.is_null() {
        let _ = unsafe { Box::from_raw(instance as *mut TsRuntime) };
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn str_from_raw<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(slice).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_config_exposes_typescript_defaults() {
        let config = TsRuntime::lsp_config();
        assert_eq!(config.language_id, "typescript");
        assert_eq!(
            config.server_command,
            vec![
                "typescript-language-server".to_string(),
                "--stdio".to_string()
            ]
        );
        assert_eq!(config.file_extension, ".ts");
        assert!(config.extra_paths.is_empty());
    }

    #[test]
    fn ts_get_lsp_config_returns_valid_msgpack_payload() {
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        let code = unsafe { ts_get_lsp_config(std::ptr::null_mut(), &mut out_ptr, &mut out_len) };
        assert_eq!(code, PluginError::Success as i32);
        assert!(!out_ptr.is_null());
        assert!(out_len > 0);

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let decoded: LanguageRuntimeLspConfig =
            rmp_serde::from_slice(bytes).expect("payload should decode");
        assert_eq!(decoded.language_id, "typescript");
        assert_eq!(decoded.file_extension, ".ts");

        unsafe { ts_free_buffer(out_ptr, out_len) };
    }
}
