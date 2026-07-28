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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

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
    /// The resolved `__shape_fn__` callable, compiled once per handle.
    ///
    /// ADR-019 §1 / #196. `invoke` used to run `PyModule::from_code` on EVERY
    /// call: it recompiled the wrapper source and built a fresh module every
    /// time, so module-level state could not persist between calls and the
    /// compile cost was paid per invocation. Holding the callable keeps its
    /// `__globals__` — the module dict — alive, which is what makes a foreign
    /// function's module state behave like a module's.
    ///
    /// `OnceLock` rather than a `Mutex`: the value is written once and read
    /// forever, and `invoke` takes `&self`.
    #[cfg(feature = "pyo3")]
    pub compiled_fn: std::sync::OnceLock<pyo3::Py<pyo3::PyAny>>,
    /// The Python module name this function's wrapper is executed under.
    ///
    /// Unique per handle. `PyModule::from_code`'s module-name argument goes
    /// through `PyImport_ExecCodeModuleEx`, which registers the module in
    /// `sys.modules` and REUSES an existing entry of the same name — so the
    /// former fixed `"__shape__"` gave every `fn python` in a program one
    /// shared global namespace. Two functions defining the same module-level
    /// name clobbered each other, and "module setup runs once per handle"
    /// could not be true of a module shared by every handle.
    ///
    /// Not dunder-prefixed: this is an ordinary `sys.modules` key, and a
    /// `__name__` would suggest a Python internal it is not.
    pub module_name: String,
}

/// The Python runtime instance. One per `init()` call.
///
/// ADR-019 §5 / #202. Every method takes `&self` and every field is interiorly
/// synchronized. The host holds this instance behind a `Send + Sync` pointer
/// and, since #202, calls `invoke` from blocking-pool worker threads while the
/// interpreter thread may be inside `compile` / `register_types` — so a method
/// taking `&mut self` through that shared pointer would be an aliasing
/// violation on a live path (the hazard #196's `register_types` exposure note
/// recorded). The cure is structural: `&mut *(instance as *mut PythonRuntime)`
/// no longer appears anywhere in this extension.
///
/// Guards are never held across a Python call. `invoke` clones the
/// `Arc<CompiledFunction>` out under a short read guard and releases it before
/// attaching the GIL, so a foreign body that re-enters Shape and compiles
/// another `fn python` cannot deadlock against its own invoke.
pub struct PythonRuntime {
    /// Compiled function handles, keyed by an incrementing ID.
    ///
    /// Values are `Arc` so a caller can take one out of the map and drop the
    /// guard before running Python with it.
    functions: RwLock<HashMap<usize, Arc<CompiledFunction>>>,
    /// Next handle ID.
    next_id: AtomicUsize,
    /// The `.pyi` document generated from the contract most recently delivered
    /// through `register_types` (ADR-019 §1 / #196). Empty until the host
    /// delivers one.
    stub_document: RwLock<String>,
    /// Which views from the most recent `invoke_with_views` were still exported
    /// when the body returned, as a bitmask of view indices (ADR-019 §2 / #199).
    ///
    /// Written by `invoke_with_views` before it returns and read by the host
    /// immediately after, on the same thread, so an interleaved second call
    /// cannot answer for the first: the host asks before it lets go of the
    /// interpreter. `AtomicU64` rather than a lock because the read happens
    /// while nothing may block.
    ///
    /// Starts at `u64::MAX` — "every view retained" — so a host that somehow
    /// asks before any buffer invoke ran gets the answer that refuses, not the
    /// answer that reclaims.
    last_retained: std::sync::atomic::AtomicU64,
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
            functions: RwLock::new(HashMap::new()),
            next_id: AtomicUsize::new(1),
            stub_document: RwLock::new(String::new()),
            last_retained: std::sync::atomic::AtomicU64::new(u64::MAX),
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

    /// Accept the declared Shape contract and generate the `.pyi` stub for it.
    ///
    /// ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196). The payload is a
    /// `ForeignContractExport`: functions and named object types already
    /// classified against the marshaling table, so this side never parses a
    /// Shape type spelling.
    ///
    /// A payload from a host speaking a newer contract version is refused
    /// rather than guessed at — a misread contract yields a confidently wrong
    /// stub, which is worse than none.
    pub fn register_types(&self, types_msgpack: &[u8]) -> Result<(), String> {
        if types_msgpack.is_empty() {
            self.write_stub_document(String::new());
            return Ok(());
        }
        let contract: shape_abi_v1::foreign_types::ForeignContractExport =
            rmp_serde::from_slice(types_msgpack)
                .map_err(|e| format!("register_types: undecodable contract payload: {e}"))?;
        contract.check_version()?;
        self.write_stub_document(crate::stubs::render_stub(&contract));
        Ok(())
    }

    fn write_stub_document(&self, doc: String) {
        let mut guard = self
            .stub_document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = doc;
    }

    /// The `.pyi` document generated from the last delivered contract.
    ///
    /// Returns an owned copy: the document lives behind the interior lock that
    /// keeps this runtime `&self`-only (see the type doc), so there is no
    /// borrow to hand out.
    pub fn stub_document(&self) -> String {
        self.stub_document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
        &self,
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

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let func = CompiledFunction {
            name: name.to_string(),
            python_source,
            param_names: param_names.to_vec(),
            shape_body_start_line: 0,
            is_async,
            return_type: return_type.to_string(),
            #[cfg(feature = "pyo3")]
            compiled_fn: std::sync::OnceLock::new(),
            module_name: format!("shape_foreign_{name}_{id}"),
        };

        self.functions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, Arc::new(func));

        // The handle is the function ID cast to a pointer.
        Ok(id as *mut c_void)
    }

    /// Invoke a previously compiled function with msgpack-encoded arguments.
    ///
    /// Returns msgpack-encoded result on success.
    ///
    /// #202: callable from any thread, concurrently. The handle lookup takes a
    /// read guard that is released before the GIL is attached, so two overlapped
    /// `async fn python` invokes never contend on host state — only on the GIL,
    /// which Python releases across `time.sleep` and blocking IO.
    pub fn invoke(&self, handle: *mut c_void, args_msgpack: &[u8]) -> Result<Vec<u8>, String> {
        let id = handle as usize;
        let func = {
            let functions = self
                .functions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                functions
                    .get(&id)
                    .ok_or_else(|| format!("invalid function handle: {id}"))?,
            )
        };
        let func = &*func;

        #[cfg(feature = "pyo3")]
        {
            use pyo3::prelude::*;
            use pyo3::types::PyModule;

            Python::attach(|py| {
                // 1. Resolve `__shape_fn__`, compiling the wrapper module the
                //    FIRST time this handle is invoked and reusing it after
                //    (ADR-019 §1 / #196).
                //
                //    Before: `PyModule::from_code` ran on every call, so the
                //    wrapper source was recompiled per invocation and each call
                //    got a fresh module dict — module-level state could not
                //    survive between calls of the same foreign function, which
                //    is not what a Python module does. Keeping the callable
                //    keeps its `__globals__` alive and fixes both.
                let shape_fn = match func.compiled_fn.get() {
                    Some(cached) => cached.bind(py).clone(),
                    None => {
                        let source_cstring = std::ffi::CString::new(func.python_source.as_str())
                            .map_err(|e| format!("Invalid source (contains null byte): {}", e))?;
                        let module_name = std::ffi::CString::new(func.module_name.as_str())
                            .map_err(|e| format!("Invalid module name: {}", e))?;
                        let code =
                            PyModule::from_code(py, &source_cstring, c"<shape>", &module_name)
                                .map_err(|e| error_mapping::format_python_error(py, &e, func))?;
                        let resolved = code
                            .getattr("__shape_fn__")
                            .map_err(|e| error_mapping::format_python_error(py, &e, func))?;
                        // A racing caller may have won; either callable is the
                        // same function object semantically, so take whichever
                        // is stored.
                        let _ = func.compiled_fn.set(resolved.clone().unbind());
                        resolved
                    }
                };

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

                // 4. Convert result -> msgpack (STRUCTURAL / untyped path).
                //
                // ffi-rebuild §4.5 (1b) / clause-2 host-as-oracle: the HOST is
                // the single return-type conformance oracle. The extension
                // returns the value structurally; shape-vm's `unmarshal_result`
                // validates it against the declared type. This keeps a genuine
                // Python EXCEPTION (rc != 0 → class-1 `Err`, §4.5 (1a)) cleanly
                // distinguishable from a NONCONFORMING return (host-detected →
                // class-1 `Err` with the `TypeConformanceError:` discriminator,
                // §4.5 (1b)). Pre-validating here would collapse both into the
                // rc != 0 channel and defeat the discriminator. `func.return_type`
                // is intentionally not consulted for the return value.
                let result_value = marshaling::pyobject_to_msgpack(py, &result)?;
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

    /// Which views from the most recent [`Self::invoke_with_views`] were still
    /// exported when the body returned (ADR-019 §2 / #199).
    pub fn last_retained_views(&self) -> u64 {
        self.last_retained
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Invoke with host buffers exported as call-scoped `memoryview`s
    /// (ADR-019 §2 / #199).
    ///
    /// `args_msgpack` is the ordinary argument array with nil at every position
    /// a view stands in for; each view names its own position. What the body
    /// receives at those positions is a `memoryview` over the caller's array —
    /// read-only for `shared`, writable for `shared mut` — cast to the element's
    /// `struct` format so `xs[3]` is the fourth number and not the fourth byte.
    ///
    /// Before returning, every view is released and the retained mask recorded.
    /// The order matters and is the reason this is not a loop over `drop`:
    ///
    /// 1. drop the argument tuple and the argument vector, so the only
    ///    remaining reference to each view is this function's own;
    /// 2. read each view's reference count — anything above 1 means the BODY
    ///    kept the object;
    /// 3. release the cast view, then the base view. `release()` raises
    ///    `BufferError` while any export is outstanding, which catches the
    ///    `numpy.asarray` case where the object was dropped but a consumer
    ///    still holds a buffer over it.
    ///
    /// Step 3 runs even when step 2 already found the view retained, so a
    /// stashed view is left POISONED rather than dangling: a later read through
    /// it raises `ValueError: operation forbidden on released memoryview`
    /// instead of reading memory the host has reclaimed.
    ///
    /// # Safety
    ///
    /// Each `view.data` must point to `view.len` correctly-aligned elements of
    /// `view.elem_type` that stay valid until this returns. The host guarantees
    /// that by holding the caller's own share of the array for the whole call.
    pub fn invoke_with_views(
        &self,
        handle: *mut c_void,
        args_msgpack: &[u8],
        views: &[shape_abi_v1::ForeignBufferView],
    ) -> Result<Vec<u8>, String> {
        use std::sync::atomic::Ordering;

        if views.len() > shape_abi_v1::MAX_SHARED_VIEWS {
            // The mask has one bit per view; more views than bits would make the
            // accounting silently incomplete, which is the one thing it must
            // never be.
            self.last_retained.store(u64::MAX, Ordering::Release);
            return Err(format!(
                "python runtime: {} shared views exceeds the {} the release accounting can \
                 report on",
                views.len(),
                shape_abi_v1::MAX_SHARED_VIEWS
            ));
        }

        // Pessimistic until proven otherwise: if anything below fails before the
        // accounting runs, the host reads "retained" and refuses rather than
        // reclaiming memory on the strength of an answer nobody computed.
        self.last_retained.store(u64::MAX, Ordering::Release);

        let id = handle as usize;
        let func = {
            let functions = self
                .functions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                functions
                    .get(&id)
                    .ok_or_else(|| format!("invalid function handle: {id}"))?,
            )
        };
        let func = &*func;

        #[cfg(feature = "pyo3")]
        {
            use pyo3::prelude::*;
            use pyo3::types::PyModule;

            Python::attach(|py| {
                let shape_fn = match func.compiled_fn.get() {
                    Some(cached) => cached.bind(py).clone(),
                    None => {
                        let source_cstring = std::ffi::CString::new(func.python_source.as_str())
                            .map_err(|e| format!("Invalid source (contains null byte): {}", e))?;
                        let module_name = std::ffi::CString::new(func.module_name.as_str())
                            .map_err(|e| format!("Invalid module name: {}", e))?;
                        let code =
                            PyModule::from_code(py, &source_cstring, c"<shape>", &module_name)
                                .map_err(|e| error_mapping::format_python_error(py, &e, func))?;
                        let resolved = code
                            .getattr("__shape_fn__")
                            .map_err(|e| error_mapping::format_python_error(py, &e, func))?;
                        let _ = func.compiled_fn.set(resolved.clone().unbind());
                        resolved
                    }
                };

                let args_values: Vec<rmpv::Value> = if args_msgpack.is_empty() {
                    Vec::new()
                } else {
                    rmp_serde::from_slice(args_msgpack)
                        .map_err(|e| format!("Failed to deserialize args: {}", e))?
                };
                let mut py_args: Vec<pyo3::Py<pyo3::PyAny>> = args_values
                    .iter()
                    .map(|v| marshaling::msgpack_to_pyobject(py, v))
                    .collect::<Result<_, _>>()?;

                // Build one view per exported buffer and substitute it for the
                // nil the host left in its place. Each goes through a
                // `ShapeBuffer` exporter this extension owns, which is where
                // CPython's export accounting lives (see `buffers.rs`).
                let mut built: Vec<crate::buffers::LentView<'_>> = Vec::with_capacity(views.len());
                for view in views {
                    // SAFETY: the host measured `data`/`len` from a live Shape
                    // array whose share the caller holds for the whole call —
                    // this function's documented safety contract.
                    let lent = unsafe { crate::buffers::LentView::new(py, view) }?;
                    let slot = view.arg_index as usize;
                    if slot >= py_args.len() {
                        return Err(format!(
                            "python runtime: view names argument {slot}, but the call has {} \
                             arguments",
                            py_args.len()
                        ));
                    }
                    py_args[slot] = lent.for_body();
                    built.push(lent);
                }

                let py_tuple = pyo3::types::PyTuple::new(py, &py_args)
                    .map_err(|e| format!("Failed to create args tuple: {}", e))?;
                let call_result = shape_fn.call1(&py_tuple);

                // Drop every reference this function handed out, so what the
                // export count reports afterwards is the BODY's doing alone.
                drop(py_tuple);
                py_args.clear();

                let mut retained: u64 = 0;
                for (i, lent) in built.iter().enumerate() {
                    if lent.reclaim() {
                        retained |= 1u64 << i;
                    }
                }
                self.last_retained.store(retained, Ordering::Release);

                let result =
                    call_result.map_err(|e| error_mapping::format_python_error(py, &e, func))?;
                let result_value = marshaling::pyobject_to_msgpack(py, &result)?;
                rmp_serde::to_vec(&result_value)
                    .map_err(|e| format!("Failed to serialize result: {}", e))
            })
        }

        #[cfg(not(feature = "pyo3"))]
        {
            let _ = (args_msgpack, views, &func.python_source);
            Err(format!(
                "python runtime: pyo3 feature not enabled (function: {})",
                func.name
            ))
        }
    }

    /// The compiled function behind a live handle, if any.
    ///
    /// Returns an `Arc` rather than a borrow so the interior lock is released
    /// before the caller uses it (see the type doc).
    pub fn compiled(&self, handle: *mut c_void) -> Option<Arc<CompiledFunction>> {
        self.functions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(handle as usize))
            .map(Arc::clone)
    }

    /// Dispose a compiled function handle, freeing associated resources.
    pub fn dispose_function(&self, handle: *mut c_void) {
        let id = handle as usize;
        let removed = self
            .functions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);

        // Each handle owns a module in `sys.modules` (see
        // `CompiledFunction::module_name`); dropping the handle without
        // dropping the module would leak one module per compiled function for
        // the life of the interpreter.
        #[cfg(feature = "pyo3")]
        if let Some(func) = removed {
            use pyo3::prelude::*;
            pyo3::Python::attach(|py| {
                if let Ok(sys) = py.import("sys") {
                    if let Ok(modules) = sys.getattr("modules") {
                        let _ = modules.del_item(&func.module_name);
                    }
                }
            });
        }
        #[cfg(not(feature = "pyo3"))]
        let _ = removed;
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
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
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

/// Return the `.pyi` generated from the contract last delivered through
/// `python_register_types` (ADR-019 §1 / #196). Caller frees via `free_buffer`.
pub unsafe extern "C" fn python_generate_stubs(
    instance: *mut c_void,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if instance.is_null() {
        return PluginError::NotInitialized as i32;
    }
    if out_ptr.is_null() || out_len.is_null() {
        return PluginError::InvalidArgument as i32;
    }
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
    let mut bytes = runtime.stub_document().as_bytes().to_vec();
    bytes.shrink_to_fit();
    let len = bytes.len();
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
    PluginError::Success as i32
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
    let runtime = unsafe { &*(instance as *const PythonRuntime) };

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
            // Classify the error to return the most appropriate error code:
            // - Marshal/serialization failures -> InvalidArgument
            // - Invalid handle -> InvalidArgument
            // - pyo3 not enabled -> NotImplemented
            // - Everything else (Python exceptions, etc.) -> InternalError
            let error_code = if msg.contains("Failed to deserialize")
                || msg.contains("Failed to serialize")
                || msg.contains("Failed to create args tuple")
                || msg.contains("invalid function handle")
            {
                PluginError::InvalidArgument
            } else if msg.contains("pyo3 feature not enabled") || msg.contains("not implemented") {
                PluginError::NotImplemented
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

pub unsafe extern "C" fn python_dispose_function(instance: *mut c_void, handle: *mut c_void) {
    if instance.is_null() {
        return;
    }
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
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

#[cfg(all(test, feature = "pyo3"))]
mod module_setup_tests {
    //! ADR-019 §1 / #196 — tripwire (3): module setup runs ONCE per handle.
    //!
    //! MEASURED before the fix: `invoke` called `PyModule::from_code` on every
    //! call, so each invocation recompiled the wrapper source into a FRESH
    //! module. Two consequences, one of them a correctness bug: the compile
    //! cost was paid per call, and module-level state could not survive between
    //! calls of the same foreign function — `globals()` was a new dict each
    //! time, which is not how a Python module behaves.
    //!
    //! The fixture counts module-level executions the only way a caller can
    //! observe them: through the module dict's own persistence. Under the old
    //! behaviour the body below returns 1 forever; under the fix it counts up.
    use super::*;

    fn runtime_with(body: &str, name: &str) -> (PythonRuntime, *mut c_void) {
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let handle = runtime
            .compile(name, body, &[], &[], "Result<int>", false)
            .expect("compile succeeds");
        (runtime, handle)
    }

    fn invoke_int(runtime: &PythonRuntime, handle: *mut c_void) -> i64 {
        let out = runtime.invoke(handle, &[]).expect("invoke succeeds");
        let value: rmpv::Value = rmp_serde::from_slice(&out).expect("decodable result");
        value.as_i64().expect("integer result")
    }

    /// ADR-019 §5 / #202 — the aliasing cure, exercised.
    ///
    /// The host now calls `invoke` from blocking-pool worker threads while the
    /// interpreter thread may be inside `compile`, which is only sound because
    /// every method here takes `&self`. Before the cure the FFI shims handed
    /// out `&mut *(instance as *mut PythonRuntime)` and this pattern was
    /// undefined behaviour, not merely racy.
    ///
    /// The overlap is the second claim: `time.sleep` releases the GIL, so two
    /// concurrent invokes finish in about one sleep rather than two. That
    /// property is what the extension's `INSTANCE_CONCURRENCY_SHARED`
    /// declaration promises the host, and it is asserted here rather than
    /// assumed.
    #[test]
    fn concurrent_invokes_on_one_runtime_are_sound_and_overlap() {
        use std::sync::Arc;

        let runtime = Arc::new(PythonRuntime::new(&[]).expect("runtime initializes"));
        let handle = runtime
            .compile(
                "nap",
                "import time\ntime.sleep(0.3)\nreturn 1\n",
                &[],
                &[],
                "Result<int>",
                false,
            )
            .expect("compile succeeds");
        // The handle is an opaque id, not a pointer into the runtime; passing
        // it across threads is what the host does too.
        let handle_bits = handle as usize;

        let start = std::time::Instant::now();
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let runtime = Arc::clone(&runtime);
                std::thread::spawn(move || invoke_int(&runtime, handle_bits as *mut c_void))
            })
            .collect();
        let results: Vec<i64> = threads
            .into_iter()
            .map(|t| t.join().expect("no panic on the worker"))
            .collect();
        let elapsed = start.elapsed();

        assert_eq!(results, vec![1, 1], "both concurrent invokes must succeed");
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "two concurrent 300ms sleeps must overlap (CPython releases the GIL \
             across time.sleep); took {elapsed:?}, serialized would be ~600ms"
        );
    }

    /// The counter lives in the module's own globals, so it increments only if
    /// the module survives between calls.
    const COUNTING_BODY: &str = "global _setups\n\
                                 _setups = globals().get('_setups', 0) + 1\n\
                                 return _setups\n";

    #[test]
    fn module_setup_runs_once_per_handle_across_many_invocations() {
        let (runtime, handle) = runtime_with(COUNTING_BODY, "count_setups");

        let observed: Vec<i64> = (0..4).map(|_| invoke_int(&runtime, handle)).collect();

        assert_eq!(
            observed,
            vec![1, 2, 3, 4],
            "the module dict must persist across calls — all-ones means the \
             module was re-executed per invocation"
        );
    }

    /// The flip side, and the shape a real program has: two `fn python`
    /// declarations in ONE runtime must not share module state.
    ///
    /// This is what caught the fixed-module-name defect. `PyModule::from_code`
    /// registers under its module-name argument, and the former literal
    /// `"__shape__"` meant every foreign function in a program executed into
    /// one `sys.modules` entry — so one function's module-level names silently
    /// clobbered another's, and this test read 2 where it should read 1.
    #[test]
    fn two_functions_in_one_runtime_do_not_share_a_module() {
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let first = runtime
            .compile("counter_a", COUNTING_BODY, &[], &[], "Result<int>", false)
            .expect("first compiles");
        let second = runtime
            .compile("counter_b", COUNTING_BODY, &[], &[], "Result<int>", false)
            .expect("second compiles");

        assert_eq!(invoke_int(&runtime, first), 1);
        assert_eq!(invoke_int(&runtime, first), 2);
        assert_eq!(
            invoke_int(&runtime, second),
            1,
            "a second declaration starts with its own module state"
        );
        assert_eq!(
            invoke_int(&runtime, first),
            3,
            "and the first is undisturbed"
        );
    }

    /// Disposal takes the handle's module with it — otherwise a long-running
    /// host leaks one `sys.modules` entry per compiled foreign function.
    #[test]
    fn disposing_a_handle_removes_its_module() {
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let handle = runtime
            .compile("disposable", COUNTING_BODY, &[], &[], "Result<int>", false)
            .expect("compiles");
        let module_name = runtime
            .compiled(handle)
            .expect("handle is live")
            .module_name
            .clone();
        invoke_int(&runtime, handle);

        let registered = |name: &str| -> bool {
            use pyo3::prelude::*;
            pyo3::Python::attach(|py| {
                py.import("sys")
                    .and_then(|sys| sys.getattr("modules"))
                    .and_then(|m| m.contains(name))
                    .unwrap_or(false)
            })
        };
        assert!(
            registered(&module_name),
            "the module is registered while live"
        );
        runtime.dispose_function(handle);
        assert!(
            !registered(&module_name),
            "disposal must remove the module from sys.modules"
        );
    }

    /// The direct structural claim: the callable is resolved once and reused.
    #[test]
    fn the_compiled_callable_is_resolved_once_and_reused() {
        let (runtime, handle) = runtime_with("return 1\n", "identity");

        let compiled = runtime.compiled(handle).expect("handle is live");
        assert!(
            compiled.compiled_fn.get().is_none(),
            "nothing is compiled before the first call — compilation stays lazy"
        );

        invoke_int(&runtime, handle);
        let first = compiled
            .compiled_fn
            .get()
            .expect("the first call populates the cache")
            .as_ptr();

        invoke_int(&runtime, handle);
        let second = compiled
            .compiled_fn
            .get()
            .expect("still populated")
            .as_ptr();

        assert_eq!(
            first, second,
            "the second call must reuse the same callable object"
        );
    }

    /// A body that fails to compile must still fail on every call, not be
    /// cached as broken or silently succeed the second time.
    #[test]
    fn a_body_that_does_not_compile_fails_on_every_call() {
        // A genuine syntax error, not merely an undefined name: `if True` with
        // no colon cannot compile, so the failure happens at module setup —
        // the step being cached.
        let (runtime, handle) = runtime_with("if True\n    pass\n", "broken");
        for _ in 0..2 {
            let err = runtime
                .invoke(handle, &[])
                .expect_err("a syntax error must surface on every call");
            assert!(
                err.contains("SyntaxError") || err.contains("invalid syntax"),
                "the Python error must reach the caller verbatim, got: {err}"
            );
        }
    }
}

#[cfg(test)]
mod contract_wire_tests {
    //! ADR-019 §1 / #196 — the host encodes the contract with
    //! `rmp_serde::to_vec_named` (`PluginLanguageRuntime::register_contract`);
    //! this asserts that exact encoding decodes here. The two sides share only
    //! the `shape-abi-v1` type, so nothing but a test binds the wire format.
    use super::*;
    use shape_abi_v1::foreign_types::{
        ForeignContractExport, ForeignFunctionContract, ForeignParamContract, ForeignScalar,
        ForeignType,
    };

    fn sample_contract(language: &str) -> ForeignContractExport {
        use shape_abi_v1::foreign_types::BufferShare;
        let mut contract = ForeignContractExport::new(language);
        contract.functions.push(ForeignFunctionContract {
            name: "add".to_string(),
            params: vec![ForeignParamContract {
                name: "a".to_string(),
                ty: ForeignType::Scalar(ForeignScalar::Int),
                share: BufferShare::Copied,
            }],
            returns: ForeignType::Optional(Box::new(ForeignType::Scalar(ForeignScalar::String))),
        });
        contract
    }

    #[test]
    fn the_hosts_encoding_decodes_and_produces_a_stub() {
        let contract = sample_contract("python");
        let bytes = rmp_serde::to_vec_named(&contract).expect("encode as the host does");

        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        runtime
            .register_types(&bytes)
            .expect("the extension decodes the host's payload");

        let stub = runtime.stub_document();
        assert!(!stub.is_empty(), "a delivered contract must produce a stub");
        assert!(
            stub.contains("add"),
            "the stub declares the function: {stub}"
        );
    }

    #[test]
    fn a_future_contract_version_is_refused_not_guessed() {
        let mut contract = sample_contract("python");
        contract.version = 999;
        let bytes = rmp_serde::to_vec_named(&contract).expect("encode");

        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let err = runtime
            .register_types(&bytes)
            .expect_err("an unknown contract version must be refused");
        assert!(err.contains("999"), "the refusal names the version: {err}");
        assert!(
            runtime.stub_document().is_empty(),
            "a refused contract must not leave a half-built stub"
        );
    }

    #[test]
    fn an_empty_payload_clears_the_stub() {
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let bytes = rmp_serde::to_vec_named(&sample_contract("python")).expect("encode");
        runtime.register_types(&bytes).expect("accepted");
        assert!(!runtime.stub_document().is_empty());
        runtime.register_types(&[]).expect("empty payload accepted");
        assert!(runtime.stub_document().is_empty());
    }
}
