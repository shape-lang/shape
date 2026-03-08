//! CPython interpreter management and foreign function execution.
//!
//! This module owns the Python interpreter lifecycle and implements the
//! core LanguageRuntime operations: init, compile, invoke, dispose.
//!
//! When the `pyo3` feature is enabled, this uses pyo3 to embed CPython.
//! Without it, all operations return stub errors.

use crate::error_mapping;
use crate::marshaling;
use shape_abi_v1::{LanguageRuntimeLspConfig, PluginError};
use std::collections::HashMap;
use std::ffi::c_void;

/// Opaque handle to a compiled Python function.
pub struct CompiledFunction {
    /// The function name in Shape.
    pub name: String,
    /// Generated Python source for the wrapper function.
    pub python_source: String,
    /// Parameter names in call order.
    pub param_names: Vec<String>,
    /// Shape source line where the foreign block body starts (for error mapping).
    pub shape_body_start_line: u32,
    /// Whether the function was declared `async` in Shape.
    pub is_async: bool,
    /// Declared return type string from Shape (e.g. "Result<int>", "Result<{id: int, name: string}>").
    /// Used by the typed marshalling path to validate/coerce Python return values.
    pub return_type: String,
}

/// The Python runtime instance. One per `init()` call.
pub struct PythonRuntime {
    /// Compiled function handles, keyed by an incrementing ID.
    functions: HashMap<usize, CompiledFunction>,
    /// Next handle ID.
    next_id: usize,
}

impl PythonRuntime {
    /// Initialize a new Python runtime.
    ///
    /// `_config_msgpack` is the MessagePack-encoded configuration from the
    /// host. Currently unused -- reserved for future settings like
    /// virtualenv path, Python version constraints, etc.
    pub fn new(_config_msgpack: &[u8]) -> Result<Self, String> {
        #[cfg(feature = "pyo3")]
        {
            // Activate virtualenv if one is detected. This mirrors what
            // `source .venv/bin/activate` does: update sys.prefix and add
            // site-packages so that `import <pkg>` works for venv packages.
            Self::activate_virtualenv();
        }

        Ok(PythonRuntime {
            functions: HashMap::new(),
            next_id: 1,
        })
    }

    /// Detect and activate a Python virtualenv.
    ///
    /// Mirrors Pyright's discovery order so the runtime resolves the same
    /// environment as the language server:
    /// 1. `pyrightconfig.json` `venvPath` + `venv` in the working directory
    /// 2. `.venv/` in the working directory
    /// 3. `venv/` in the working directory
    /// 4. `VIRTUAL_ENV` environment variable
    ///
    /// When found, adds the venv's site-packages to `sys.path` and updates
    /// `sys.prefix` so that `import <pkg>` works for venv-installed packages.
    #[cfg(feature = "pyo3")]
    fn activate_virtualenv() {
        use pyo3::prelude::*;

        let cwd = std::env::current_dir().ok();

        // 1. Check pyrightconfig.json for venvPath + venv
        let from_pyright_config = cwd.as_ref().and_then(|cwd| {
            let config_path = cwd.join("pyrightconfig.json");
            let contents = std::fs::read_to_string(&config_path).ok()?;
            let config: serde_json::Value = serde_json::from_str(&contents).ok()?;
            let venv_path = config.get("venvPath")?.as_str()?;
            let venv_name = config.get("venv")?.as_str()?;
            let base = if std::path::Path::new(venv_path).is_absolute() {
                std::path::PathBuf::from(venv_path)
            } else {
                cwd.join(venv_path)
            };
            let candidate = base.join(venv_name);
            candidate.is_dir().then_some(candidate)
        });

        // 2-3. Check .venv/ and venv/ in working directory
        let from_local_dir = || -> Option<std::path::PathBuf> {
            let cwd = cwd.as_ref()?;
            for name in &[".venv", "venv"] {
                let candidate = cwd.join(name);
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
            None
        };

        // 4. VIRTUAL_ENV environment variable
        let from_env = || -> Option<std::path::PathBuf> {
            let path = std::path::PathBuf::from(std::env::var("VIRTUAL_ENV").ok()?);
            path.is_dir().then_some(path)
        };

        let venv = from_pyright_config
            .or_else(from_local_dir)
            .or_else(from_env);

        let Some(venv) = venv else { return };
        let venv_str = venv.display().to_string();

        Python::attach(|py| {
            let code = format!(
                concat!(
                    "import sys, site, os\n",
                    "venv = \"{venv}\"\n",
                    "sys.prefix = venv\n",
                    "sys.exec_prefix = venv\n",
                    "lib_dir = os.path.join(venv, \"lib\")\n",
                    "if os.path.isdir(lib_dir):\n",
                    "    for entry in os.listdir(lib_dir):\n",
                    "        sp = os.path.join(lib_dir, entry, \"site-packages\")\n",
                    "        if os.path.isdir(sp):\n",
                    "            site.addsitedir(sp)\n",
                    "            break\n",
                ),
                venv = venv_str,
            );

            if let Err(e) = py.run(&std::ffi::CString::new(code).unwrap(), None, None) {
                eprintln!("shape-ext-python: failed to activate venv: {e}");
            }
        });
    }

    /// Register Shape type schemas for Python stub generation.
    ///
    /// The runtime receives the full set of Shape types so it can generate
    /// Python dataclass stubs that the user's code can reference.
    pub fn register_types(&mut self, _types_msgpack: &[u8]) -> Result<(), String> {
        // Stub: the real implementation will deserialize TypeSchemaExport[]
        // and generate Python dataclass definitions injected into the
        // interpreter's namespace.
        Ok(())
    }

    /// Compile a foreign function body into a callable Python function.
    ///
    /// When `is_async` is false, wraps the user's body in:
    /// ```python
    /// def __shape_fn__(param1, param2) -> return_type:
    ///     <body>
    /// ```
    ///
    /// When `is_async` is true, wraps it in an async def with an asyncio runner:
    /// ```python
    /// import asyncio
    /// async def __shape_async__(param1, param2) -> return_type:
    ///     <body>
    /// def __shape_fn__(param1, param2) -> return_type:
    ///     return asyncio.run(__shape_async__(param1, param2))
    /// ```
    ///
    /// Returns a handle that can be passed to `invoke()`.
    pub fn compile(
        &mut self,
        name: &str,
        source: &str,
        param_names: &[String],
        param_types: &[String],
        return_type: &str,
        is_async: bool,
    ) -> Result<*mut c_void, String> {
        // Build type-hinted parameter list.
        let params: Vec<String> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(pname, ptype)| {
                format!(
                    "{}: {}",
                    pname,
                    marshaling::shape_type_to_python_hint(ptype)
                )
            })
            .collect();
        let params_str = params.join(", ");
        let return_hint = marshaling::shape_type_to_python_hint(return_type);

        // Indent the user body by 4 spaces.
        let indented_body: String = source
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let python_source = if is_async {
            // Wrap in async def + synchronous asyncio.run() entry point.
            let plain_params: Vec<&str> = param_names.iter().map(|s| s.as_str()).collect();
            let call_args = plain_params.join(", ");
            format!(
                "import asyncio\n\
                 async def __shape_async__({params_str}) -> {return_hint}:\n\
                 {indented_body}\n\
                 def __shape_fn__({params_str}) -> {return_hint}:\n\
                 {sync_indent}return asyncio.run(__shape_async__({call_args}))\n",
                sync_indent = "    ",
            )
        } else {
            format!("def __shape_fn__({params_str}) -> {return_hint}:\n{indented_body}")
        };

        let id = self.next_id;
        self.next_id += 1;

        let func = CompiledFunction {
            name: name.to_string(),
            python_source,
            param_names: param_names.to_vec(),
            shape_body_start_line: 0,
            is_async,
            return_type: return_type.to_string(),
        };

        self.functions.insert(id, func);

        // The handle is the function ID cast to a pointer.
        Ok(id as *mut c_void)
    }

    /// Invoke a previously compiled function with msgpack-encoded arguments.
    ///
    /// Returns msgpack-encoded result on success.
    pub fn invoke(&self, handle: *mut c_void, args_msgpack: &[u8]) -> Result<Vec<u8>, String> {
        let id = handle as usize;
        let func = self
            .functions
            .get(&id)
            .ok_or_else(|| format!("invalid function handle: {id}"))?;

        #[cfg(feature = "pyo3")]
        {
            use pyo3::prelude::*;
            use pyo3::types::PyModule;

            Python::attach(|py| {
                // 1. Execute the compiled source to define __shape_fn__
                let source_cstring = std::ffi::CString::new(func.python_source.as_str())
                    .map_err(|e| format!("Invalid source (contains null byte): {}", e))?;
                let code = PyModule::from_code(py, &source_cstring, c"<shape>", c"__shape__")
                    .map_err(|e| error_mapping::format_python_error(py, &e, func))?;

                let shape_fn = code
                    .getattr("__shape_fn__")
                    .map_err(|e| error_mapping::format_python_error(py, &e, func))?;

                // 2. Deserialize msgpack args -> Vec<rmpv::Value> -> Vec<Py<PyAny>>
                let args_values: Vec<rmpv::Value> = if args_msgpack.is_empty() {
                    Vec::new()
                } else {
                    rmp_serde::from_slice(args_msgpack)
                        .map_err(|e| format!("Failed to deserialize args: {}", e))?
                };

                let py_args: Vec<pyo3::Py<pyo3::PyAny>> = args_values
                    .iter()
                    .map(|v| marshaling::msgpack_to_pyobject(py, v))
                    .collect::<Result<_, _>>()?;

                // 3. Call the function
                let py_tuple = pyo3::types::PyTuple::new(py, &py_args)
                    .map_err(|e| format!("Failed to create args tuple: {}", e))?;
                let result = shape_fn
                    .call1(&py_tuple)
                    .map_err(|e| error_mapping::format_python_error(py, &e, func))?;

                // 4. Convert result -> msgpack (type-aware path)
                let result_value =
                    marshaling::pyobject_to_typed_msgpack(py, &result, &func.return_type)?;
                rmp_serde::to_vec(&result_value)
                    .map_err(|e| format!("Failed to serialize result: {}", e))
            })
        }

        #[cfg(not(feature = "pyo3"))]
        {
            let _ = args_msgpack;
            let _ = &func.python_source;
            let _ = error_mapping::parse_traceback;
            Err(format!(
                "python runtime: pyo3 feature not enabled (function: {})",
                func.name
            ))
        }
    }

    /// Dispose a compiled function handle, freeing associated resources.
    pub fn dispose_function(&mut self, handle: *mut c_void) {
        let id = handle as usize;
        self.functions.remove(&id);
    }

    /// Return the language identifier.
    pub fn language_id() -> &'static str {
        "python"
    }

    /// Return LSP configuration for Python (pyright).
    pub fn lsp_config() -> LanguageRuntimeLspConfig {
        LanguageRuntimeLspConfig {
            language_id: "python".into(),
            server_command: vec!["pyright-langserver".into(), "--stdio".into()],
            file_extension: ".py".into(),
            extra_paths: Vec::new(),
        }
    }
}

// ============================================================================
// C ABI callback functions (wired from lib.rs vtable)
// ============================================================================

pub unsafe extern "C" fn python_init(config: *const u8, config_len: usize) -> *mut c_void {
    // Promote libpython symbols to global visibility before any Python code
    // runs. Python C extensions (numpy, pandas, etc.) loaded via `import`
    // expect CPython API symbols (PyExc_ValueError, etc.) to be globally
    // visible. Since the host loads this .so with RTLD_LOCAL, libpython's
    // symbols are hidden. Re-opening with RTLD_NOLOAD | RTLD_GLOBAL
    // promotes them without loading a second copy.
    #[cfg(unix)]
    promote_libpython_symbols();

    let config_slice = if config.is_null() || config_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(config, config_len) }
    };

    match PythonRuntime::new(config_slice) {
        Ok(runtime) => Box::into_raw(Box::new(runtime)) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

pub unsafe extern "C" fn python_register_types(
    instance: *mut c_void,
    types_msgpack: *const u8,
    types_len: usize,
) -> i32 {
    if instance.is_null() {
        return PluginError::NotInitialized as i32;
    }
    let runtime = unsafe { &mut *(instance as *mut PythonRuntime) };
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

pub unsafe extern "C" fn python_compile(
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
    let runtime = unsafe { &mut *(instance as *mut PythonRuntime) };

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
        None => "_", // Default to inferred for generic/complex return types
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

pub unsafe extern "C" fn python_invoke(
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
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
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
            // Write error message to output buffer so the host can read it
            let mut err_bytes = msg.into_bytes();
            let len = err_bytes.len();
            let ptr = err_bytes.as_mut_ptr();
            std::mem::forget(err_bytes);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            PluginError::NotImplemented as i32
        }
    }
}

pub unsafe extern "C" fn python_dispose_function(instance: *mut c_void, handle: *mut c_void) {
    if instance.is_null() {
        return;
    }
    let runtime = unsafe { &mut *(instance as *mut PythonRuntime) };
    runtime.dispose_function(handle);
}

pub unsafe extern "C" fn python_language_id(_instance: *mut c_void) -> *const std::ffi::c_char {
    // "python\0" -- static, owned by the extension.
    c"python".as_ptr()
}

pub unsafe extern "C" fn python_get_lsp_config(
    _instance: *mut c_void,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return PluginError::InvalidArgument as i32;
    }
    let config = PythonRuntime::lsp_config();
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

pub unsafe extern "C" fn python_free_buffer(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        let _ = unsafe { Vec::from_raw_parts(ptr, len, len) };
    }
}

pub unsafe extern "C" fn python_drop(instance: *mut c_void) {
    if !instance.is_null() {
        let _ = unsafe { Box::from_raw(instance as *mut PythonRuntime) };
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Re-open libpython with RTLD_GLOBAL so its symbols are visible to C
/// extensions (numpy, pandas, etc.) loaded later via Python's own dlopen.
///
/// We try common sonames in order. RTLD_NOLOAD ensures we only promote
/// the copy already in memory — no new loading occurs.
#[cfg(unix)]
fn promote_libpython_symbols() {
    const SONAMES: &[&[u8]] = &[
        b"libpython3.13.so.1.0\0",
        b"libpython3.13.so\0",
        b"libpython3.12.so.1.0\0",
        b"libpython3.12.so\0",
        b"libpython3.11.so.1.0\0",
        b"libpython3.11.so\0",
        b"libpython3.so\0",
    ];
    for soname in SONAMES {
        let handle = unsafe {
            libc::dlopen(
                soname.as_ptr() as *const std::ffi::c_char,
                libc::RTLD_NOLOAD | libc::RTLD_NOW | libc::RTLD_GLOBAL,
            )
        };
        if !handle.is_null() {
            unsafe { libc::dlclose(handle) };
            return;
        }
    }
    // If none matched, fall through silently — basic Python works fine,
    // only C extensions that reference libpython symbols will fail.
}

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
    fn lsp_config_exposes_pyright_defaults() {
        let config = PythonRuntime::lsp_config();
        assert_eq!(config.language_id, "python");
        assert_eq!(
            config.server_command,
            vec!["pyright-langserver".to_string(), "--stdio".to_string()]
        );
        assert_eq!(config.file_extension, ".py");
        assert!(config.extra_paths.is_empty());
    }

    #[test]
    fn python_get_lsp_config_returns_valid_msgpack_payload() {
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        let code =
            unsafe { python_get_lsp_config(std::ptr::null_mut(), &mut out_ptr, &mut out_len) };
        assert_eq!(code, PluginError::Success as i32);
        assert!(!out_ptr.is_null());
        assert!(out_len > 0);

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let decoded: LanguageRuntimeLspConfig =
            rmp_serde::from_slice(bytes).expect("payload should decode");
        assert_eq!(decoded.language_id, "python");
        assert_eq!(decoded.file_extension, ".py");

        unsafe { python_free_buffer(out_ptr, out_len) };
    }
}
