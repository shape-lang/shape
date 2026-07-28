//! Language runtime capability wrapper (`shape.language_runtime`).
//!
//! Wraps a loaded `LanguageRuntimeVTable` plugin to provide foreign function
//! compilation and invocation for inline foreign language blocks.

use shape_abi_v1::{ErrorModel, LanguageRuntimeLspConfig, LanguageRuntimeVTable};
use shape_ast::error::{Result, ShapeError};
use std::ffi::{CStr, c_void};
use std::sync::Arc;

/// Handle to a compiled foreign function within a language runtime.
#[derive(Clone)]
pub struct CompiledForeignFunction {
    handle: *mut c_void,
    /// Weak reference to the runtime for invoke/dispose
    _runtime: Arc<LanguageRuntimeState>,
}

// SAFETY: The handle is opaque and managed by the extension.
// The extension is responsible for thread safety of its own handles.
unsafe impl Send for CompiledForeignFunction {}
unsafe impl Sync for CompiledForeignFunction {}

struct LanguageRuntimeState {
    vtable: &'static LanguageRuntimeVTable,
    instance: *mut c_void,
    config_bytes: Arc<[u8]>,
}

// SAFETY: Language runtime extensions must be thread-safe.
unsafe impl Send for LanguageRuntimeState {}
unsafe impl Sync for LanguageRuntimeState {}

impl Drop for LanguageRuntimeState {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.vtable.drop {
            unsafe { drop_fn(self.instance) };
        }
    }
}

/// How the host may drive one extension instance concurrently, as declared by
/// the extension through `LanguageRuntimeVTable::instance_concurrency`.
///
/// ADR-019 §5 / #202. Real foreign async runs `invoke` off the interpreter
/// thread, so the instance pointer becomes genuinely shared and the answer
/// matters. It is READ from the vtable, never inferred from the language id —
/// deciding a capability from a terminal name is the spelling-selected
/// semantics this codebase forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceConcurrency {
    /// Undeclared, or explicitly interpreter-thread-only: the host must not
    /// touch this instance from any other thread. Async foreign calls into it
    /// are refused, not offloaded.
    InterpreterThreadOnly,
    /// Every vtable entry takes `&self` on the far side and the instance is
    /// interiorly synchronized: several threads may `invoke` at once.
    Shared,
    /// The instance is bound to the thread that created it. The host must give
    /// the language dedicated worker threads, each owning its own instance
    /// built through [`PluginLanguageRuntime::fresh_instance`].
    ThreadAffine,
}

impl InstanceConcurrency {
    fn from_declared(raw: u32) -> Self {
        match raw {
            shape_abi_v1::INSTANCE_CONCURRENCY_SHARED => Self::Shared,
            shape_abi_v1::INSTANCE_CONCURRENCY_THREAD_AFFINE => Self::ThreadAffine,
            // Covers INTERPRETER_THREAD_ONLY and any value from an extension
            // speaking a newer vocabulary: an unrecognised declaration reads as
            // the most restrictive model rather than being guessed at.
            _ => Self::InterpreterThreadOnly,
        }
    }
}

/// Wrapper around a loaded language runtime extension.
pub struct PluginLanguageRuntime {
    /// The self-declared language identifier (e.g., "python").
    language_id: String,
    /// Shared state for the runtime instance.
    state: Arc<LanguageRuntimeState>,
    /// Error model declared by the runtime.
    error_model: ErrorModel,
    /// Off-thread-invocation model declared by the runtime (ADR-019 §5 / #202).
    instance_concurrency: InstanceConcurrency,
}

/// Host-consumable LSP configuration declared by a language runtime extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLspConfig {
    pub language_id: String,
    pub server_command: Vec<String>,
    pub file_extension: String,
    pub extra_paths: Vec<String>,
}

impl PluginLanguageRuntime {
    /// Create a new language runtime wrapper from a plugin vtable.
    pub fn new(vtable: &'static LanguageRuntimeVTable, config: &serde_json::Value) -> Result<Self> {
        let config_bytes = rmp_serde::to_vec(config).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize language runtime config: {}", e),
            location: None,
        })?;
        Self::from_config_bytes(vtable, Arc::<[u8]>::from(config_bytes))
    }

    fn from_config_bytes(
        vtable: &'static LanguageRuntimeVTable,
        config_bytes: Arc<[u8]>,
    ) -> Result<Self> {
        let init_fn = vtable.init.ok_or_else(|| ShapeError::RuntimeError {
            message: "Language runtime vtable has no init function".to_string(),
            location: None,
        })?;

        let instance = unsafe { init_fn(config_bytes.as_ptr(), config_bytes.len()) };
        if instance.is_null() {
            return Err(ShapeError::RuntimeError {
                message: "Language runtime init returned null".to_string(),
                location: None,
            });
        }

        // Get language ID
        let lang_id_fn = vtable.language_id.ok_or_else(|| ShapeError::RuntimeError {
            message: "Language runtime vtable has no language_id function".to_string(),
            location: None,
        })?;
        let lang_ptr = unsafe { lang_id_fn(instance) };
        let language_id = if lang_ptr.is_null() {
            return Err(ShapeError::RuntimeError {
                message: "Language runtime returned null language_id".to_string(),
                location: None,
            });
        } else {
            unsafe { CStr::from_ptr(lang_ptr) }
                .to_string_lossy()
                .to_string()
        };

        let error_model = vtable.error_model;
        // ADR-019 §5 (#202). Read once at construction: the declaration is a
        // property of the runtime build, and an instance that changed its mind
        // mid-run could invalidate an offload already in flight.
        let instance_concurrency = match vtable.instance_concurrency {
            Some(f) => InstanceConcurrency::from_declared(unsafe { f(instance) }),
            None => InstanceConcurrency::InterpreterThreadOnly,
        };
        let state = Arc::new(LanguageRuntimeState {
            vtable,
            instance,
            config_bytes,
        });

        Ok(Self {
            language_id,
            state,
            error_model,
            instance_concurrency,
        })
    }

    /// The off-thread-invocation model this runtime declares (ADR-019 §5 /
    /// #202). Drives whether an `async fn <language>` call can be offloaded,
    /// and if so onto what.
    pub fn instance_concurrency(&self) -> InstanceConcurrency {
        self.instance_concurrency
    }

    /// Create a new runtime instance using the same vtable and init config.
    ///
    /// Some embedded runtimes are thread-affine (notably V8/deno_core). Serve
    /// workers use this to instantiate the runtime on the blocking worker that
    /// will compile and invoke foreign functions.
    pub fn fresh_instance(&self) -> Result<Self> {
        Self::from_config_bytes(self.state.vtable, Arc::clone(&self.state.config_bytes))
    }

    /// The language identifier this runtime handles (e.g., "python").
    pub fn language_id(&self) -> &str {
        &self.language_id
    }

    /// Whether this runtime has a dynamic error model.
    ///
    /// When `true`, every foreign function call can fail at runtime, so return
    /// values are automatically wrapped in `Result<T>`.
    pub fn has_dynamic_errors(&self) -> bool {
        self.error_model == ErrorModel::Dynamic
    }

    /// Query optional child-LSP configuration declared by the runtime.
    pub fn lsp_config(&self) -> Result<Option<RuntimeLspConfig>> {
        let get_lsp_config = match self.state.vtable.get_lsp_config {
            Some(f) => f,
            None => return Ok(None),
        };

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe { get_lsp_config(self.state.instance, &mut out_ptr, &mut out_len) };
        if rc != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' get_lsp_config failed (error code {})",
                    self.language_id, rc
                ),
                location: None,
            });
        }

        if out_ptr.is_null() || out_len == 0 {
            return Ok(None);
        }

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
        if let Some(free_fn) = self.state.vtable.free_buffer {
            unsafe { free_fn(out_ptr, out_len) };
        }

        let decoded: LanguageRuntimeLspConfig =
            rmp_serde::from_slice(&bytes).map_err(|e| ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' returned invalid get_lsp_config payload: {}",
                    self.language_id, e
                ),
                location: None,
            })?;

        Ok(Some(RuntimeLspConfig {
            language_id: decoded.language_id,
            server_command: decoded.server_command,
            file_extension: decoded.file_extension,
            extra_paths: decoded.extra_paths,
        }))
    }

    /// Deliver the declared Shape contract for this language, then collect the
    /// interface stub the extension generates from it.
    ///
    /// ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196). This is the whole stub
    /// channel in one call, because the two halves are not independently
    /// meaningful: a contract delivered with no stub collected is the
    /// caller-less `register_types` this ticket exists to fix, and a stub
    /// requested without a contract has nothing to describe.
    ///
    /// Returns `None` when the extension declares no stub channel (the vtable
    /// slot is `None` — an extension built before the capability existed). The
    /// contract is still delivered in that case; only the stub is unavailable.
    pub fn register_contract(
        &self,
        contract: &shape_abi_v1::foreign_types::ForeignContractExport,
    ) -> Result<Option<String>> {
        let bytes = rmp_serde::to_vec_named(contract).map_err(|e| ShapeError::RuntimeError {
            message: format!(
                "Failed to serialize the foreign contract for '{}': {}",
                self.language_id, e
            ),
            location: None,
        })?;
        self.register_types(&bytes)?;
        self.generate_stubs()
    }

    /// Return the interface stub document the extension generated from the
    /// contract most recently delivered through [`Self::register_types`].
    ///
    /// `None` means the extension declares no stub channel; `Some("")` means it
    /// declares one and produced nothing.
    pub fn generate_stubs(&self) -> Result<Option<String>> {
        let Some(generate_fn) = self.state.vtable.generate_stubs else {
            return Ok(None);
        };

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe { generate_fn(self.state.instance, &mut out_ptr, &mut out_len) };
        if rc != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' generate_stubs failed (error code {})",
                    self.language_id, rc
                ),
                location: None,
            });
        }
        if out_ptr.is_null() || out_len == 0 {
            return Ok(Some(String::new()));
        }

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
        if let Some(free_fn) = self.state.vtable.free_buffer {
            unsafe { free_fn(out_ptr, out_len) };
        }

        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' returned a non-UTF-8 stub document: {}",
                    self.language_id, e
                ),
                location: None,
            })
    }

    /// Register Shape type schemas with the runtime for stub generation.
    ///
    /// Prefer [`Self::register_contract`], which owns the payload encoding and
    /// collects the generated stub; this is the raw vtable call.
    pub fn register_types(&self, types_msgpack: &[u8]) -> Result<()> {
        let register_fn = match self.state.vtable.register_types {
            Some(f) => f,
            None => return Ok(()), // Optional capability
        };

        let rc = unsafe {
            register_fn(
                self.state.instance,
                types_msgpack.as_ptr(),
                types_msgpack.len(),
            )
        };
        if rc != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' register_types failed (error code {})",
                    self.language_id, rc
                ),
                location: None,
            });
        }
        Ok(())
    }

    /// Pre-compile a foreign function body.
    pub fn compile(
        &self,
        name: &str,
        source: &str,
        param_names: &[String],
        param_types: &[String],
        return_type: Option<&str>,
        is_async: bool,
    ) -> Result<CompiledForeignFunction> {
        let compile_fn = self
            .state
            .vtable
            .compile
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' has no compile function",
                    self.language_id
                ),
                location: None,
            })?;

        let names_bytes = rmp_serde::to_vec(param_names).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize param names: {}", e),
            location: None,
        })?;
        let types_bytes = rmp_serde::to_vec(param_types).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize param types: {}", e),
            location: None,
        })?;
        let return_type_str = return_type.unwrap_or("");

        let mut out_error: *mut u8 = std::ptr::null_mut();
        let mut out_error_len: usize = 0;

        let handle = unsafe {
            compile_fn(
                self.state.instance,
                name.as_ptr(),
                name.len(),
                source.as_ptr(),
                source.len(),
                names_bytes.as_ptr(),
                names_bytes.len(),
                types_bytes.as_ptr(),
                types_bytes.len(),
                return_type_str.as_ptr(),
                return_type_str.len(),
                is_async,
                &mut out_error,
                &mut out_error_len,
            )
        };

        if handle.is_null() {
            let msg = if !out_error.is_null() && out_error_len > 0 {
                let error_bytes =
                    unsafe { std::slice::from_raw_parts(out_error, out_error_len) }.to_vec();
                if let Some(free_fn) = self.state.vtable.free_buffer {
                    unsafe { free_fn(out_error, out_error_len) };
                }
                String::from_utf8_lossy(&error_bytes).to_string()
            } else {
                "unknown compilation error".to_string()
            };

            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' failed to compile foreign function '{}': {}",
                    self.language_id, name, msg
                ),
                location: None,
            });
        }

        Ok(CompiledForeignFunction {
            handle,
            _runtime: Arc::clone(&self.state),
        })
    }

    /// Invoke a compiled foreign function with msgpack-encoded arguments.
    pub fn invoke(
        &self,
        compiled: &CompiledForeignFunction,
        args_msgpack: &[u8],
    ) -> Result<Vec<u8>> {
        let invoke_fn = self
            .state
            .vtable
            .invoke
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' has no invoke function",
                    self.language_id
                ),
                location: None,
            })?;

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        let rc = unsafe {
            invoke_fn(
                self.state.instance,
                compiled.handle,
                args_msgpack.as_ptr(),
                args_msgpack.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };

        if rc != 0 {
            // Try to read error message from output buffer
            let msg = if !out_ptr.is_null() && out_len > 0 {
                let error_bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
                if let Some(free_fn) = self.state.vtable.free_buffer {
                    unsafe { free_fn(out_ptr, out_len) };
                }
                String::from_utf8_lossy(&error_bytes).to_string()
            } else {
                format!("error code {}", rc)
            };
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' invoke failed: {}",
                    self.language_id, msg
                ),
                location: None,
            });
        }

        if out_ptr.is_null() || out_len == 0 {
            return Ok(vec![]);
        }

        let result = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();

        // Free the buffer
        if let Some(free_fn) = self.state.vtable.free_buffer {
            unsafe { free_fn(out_ptr, out_len) };
        }

        Ok(result)
    }

    /// Dispose of a compiled foreign function handle.
    pub fn dispose_function(&self, compiled: &CompiledForeignFunction) {
        if let Some(dispose_fn) = self.state.vtable.dispose_function {
            unsafe {
                dispose_fn(self.state.instance, compiled.handle);
            }
        }
    }

    /// Retrieve the bundled `.shape` module source from this language runtime.
    ///
    /// Returns `Some((namespace, source))` if the extension bundles a Shape
    /// module artifact, where `namespace` is the extension's own namespace
    /// (e.g. `"python"`, `"typescript"`) -- NOT `"std::core::*"`.
    ///
    /// Returns `None` if the extension does not bundle any Shape source.
    pub fn shape_source(&self) -> Result<Option<(String, String)>> {
        let get_source_fn = match self.state.vtable.get_shape_source {
            Some(f) => f,
            None => return Ok(None),
        };

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe { get_source_fn(self.state.instance, &mut out_ptr, &mut out_len) };
        if rc != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' get_shape_source failed (error code {})",
                    self.language_id, rc
                ),
                location: None,
            });
        }

        if out_ptr.is_null() || out_len == 0 {
            return Ok(None);
        }

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
        if let Some(free_fn) = self.state.vtable.free_buffer {
            unsafe { free_fn(out_ptr, out_len) };
        }

        let source = String::from_utf8(bytes).map_err(|e| ShapeError::RuntimeError {
            message: format!(
                "Language runtime '{}' returned invalid UTF-8 shape source: {}",
                self.language_id, e
            ),
            location: None,
        })?;

        // The namespace is the language_id itself (e.g. "python", "typescript"),
        // NOT "std::core::python".
        Ok(Some((self.language_id.clone(), source)))
    }
}
