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

/// Why a runtime cannot share host buffers with foreign code (ADR-019 §2 /
/// #199).
///
/// Every variant is a REFUSAL, never a weakening: a `shared` parameter against a
/// runtime in any of these states is an error at the call, not a quiet deep
/// copy. A silent fallback would make the declared `shared` spelling a lie about
/// what the boundary did, and that spelling exists precisely to be visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferRefusal {
    /// The extension declares no capability block, or one with no buffer
    /// capability in it — the ordinary state of every extension built before
    /// #199, and of any runtime that chooses not to implement sharing.
    NotOffered,
    /// The block declares a capability version this host does not know. Refused
    /// rather than read as far as it seems to be understood: a capability table
    /// read at guessed offsets hands raw host memory to foreign code.
    UnknownVersion(u32),
    /// The block is physically shorter than the field being read. Same
    /// treatment as an unknown version.
    Truncated,
    /// A buffer capability with no `invoke_with_buffers` entry: nothing to call.
    NoInvokeEntry,
    /// A buffer capability with no `outstanding_exports` entry — the runtime
    /// offers views but cannot say whether the foreign side released them.
    ///
    /// ADR-019 §2's named refusal: "where a runtime offers no release
    /// accounting, the mode is refused for that language rather than silently
    /// weakened". Without it the host would unpin memory that foreign code may
    /// still hold, which is the corruption class this capability exists to make
    /// impossible.
    NoReleaseAccounting,
}

impl BufferRefusal {
    /// A sentence fragment completing "… because {}".
    pub fn explain(self) -> String {
        match self {
            BufferRefusal::NotOffered => {
                "this language runtime does not offer buffer sharing — its extension \
                 declares no buffer capability (ADR-019 §2)"
                    .to_string()
            }
            BufferRefusal::UnknownVersion(v) => format!(
                "the extension declares capability version {v}, which this host does not \
                 know; a capability block read at guessed offsets would hand raw host \
                 memory to foreign code, so it is refused whole"
            ),
            BufferRefusal::Truncated => {
                "the extension's capability block is shorter than the fields it claims, so \
                 it is refused whole rather than read past its end"
                    .to_string()
            }
            BufferRefusal::NoInvokeEntry => {
                "the extension declares a buffer capability with no invoke entry point".to_string()
            }
            BufferRefusal::NoReleaseAccounting => {
                "the extension offers buffer views but no release accounting, so nothing \
                 could tell the host whether foreign code still holds a view when the call \
                 returns. ADR-019 §2 refuses the mode for such a runtime rather than \
                 weakening it: an unreleased view over reclaimed memory is exactly the \
                 corruption this capability is designed to make impossible"
                    .to_string()
            }
        }
    }
}

/// A runtime's negotiated buffer-sharing capability (ADR-019 §2 / #199).
///
/// Read once at construction, like every other declaration on this vtable: the
/// capability is a property of the extension build, and one that changed
/// mid-run could invalidate a view already handed out.
#[derive(Clone, Copy)]
pub struct BufferCapabilityInfo {
    /// Bitmask of `shape_abi_v1::BUFFER_MODE_*` the runtime implements.
    modes: u32,
    /// Bitmask of `1 << shape_abi_v1::BUFFER_ELEM_*` the runtime can project.
    elem_types: u32,
    invoke_with_buffers: unsafe extern "C" fn(
        instance: *mut c_void,
        handle: *mut c_void,
        args: *const u8,
        args_len: usize,
        views: *const shape_abi_v1::ForeignBufferView,
        views_len: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32,
    outstanding_exports: unsafe extern "C" fn(instance: *mut c_void) -> u64,
}

impl BufferCapabilityInfo {
    /// Whether the runtime implements `mode` (one of the `BUFFER_MODE_*`
    /// constants).
    pub fn supports_mode(&self, mode: u32) -> bool {
        self.modes & mode != 0
    }

    /// Whether the runtime can project `elem_type` (one of the `BUFFER_ELEM_*`
    /// constants).
    pub fn supports_elem(&self, elem_type: u32) -> bool {
        self.elem_types & (1u32 << elem_type) != 0
    }
}

/// The outcome of one buffer-sharing invoke.
///
/// `retained` is queried unconditionally — including when the invoke itself
/// failed, because a foreign body that raised half-way through is exactly the
/// case most likely to have left a view alive.
pub struct BufferInvokeOutcome {
    /// The invoke's own result: MessagePack result bytes, or the runtime error.
    pub result: Result<Vec<u8>>,
    /// Bitmask of view indices still exported when the body returned. Zero is
    /// the only value that lets the host reclaim the memory.
    pub retained: u64,
}

/// Read the extension's capability block, refusing anything this host cannot
/// read exactly (ADR-019 §2 / #199).
///
/// The size guard is the whole extensibility mechanism: a field is read only
/// when the extension's own `struct_size` covers it, so a block written against
/// an older definition reads as "the newer capabilities are absent" instead of
/// as garbage.
fn negotiate_buffers(
    vtable: &'static LanguageRuntimeVTable,
    instance: *mut c_void,
) -> std::result::Result<BufferCapabilityInfo, BufferRefusal> {
    use shape_abi_v1::{BufferCapability, EXTENSION_CAPABILITIES_VERSION, ExtensionCapabilities};

    let Some(get) = vtable.capabilities else {
        return Err(BufferRefusal::NotOffered);
    };
    let caps = unsafe { get(instance) };
    if caps.is_null() {
        return Err(BufferRefusal::NotOffered);
    }
    // Only `struct_size` and `version` may be read before the guard is known —
    // they are the first two fields of every version of the block, which is the
    // one thing the format promises unconditionally.
    let (declared_size, version) = unsafe { ((*caps).struct_size as usize, (*caps).version) };
    if version != EXTENSION_CAPABILITIES_VERSION {
        return Err(BufferRefusal::UnknownVersion(version));
    }
    const NEED_BUFFERS: usize = std::mem::offset_of!(ExtensionCapabilities, buffers)
        + std::mem::size_of::<*const BufferCapability>();
    if declared_size < NEED_BUFFERS {
        return Err(BufferRefusal::Truncated);
    }

    let buffers = unsafe { (*caps).buffers };
    if buffers.is_null() {
        return Err(BufferRefusal::NotOffered);
    }
    let buf_size = unsafe { (*buffers).struct_size as usize };
    const NEED_OUTSTANDING: usize = std::mem::offset_of!(BufferCapability, outstanding_exports)
        + std::mem::size_of::<Option<unsafe extern "C" fn(*mut c_void) -> u64>>();
    if buf_size < NEED_OUTSTANDING {
        return Err(BufferRefusal::Truncated);
    }

    let (modes, elem_types, invoke, outstanding) = unsafe {
        (
            (*buffers).modes,
            (*buffers).elem_types,
            (*buffers).invoke_with_buffers,
            (*buffers).outstanding_exports,
        )
    };
    let Some(invoke_with_buffers) = invoke else {
        return Err(BufferRefusal::NoInvokeEntry);
    };
    let Some(outstanding_exports) = outstanding else {
        return Err(BufferRefusal::NoReleaseAccounting);
    };
    Ok(BufferCapabilityInfo {
        modes,
        elem_types,
        invoke_with_buffers,
        outstanding_exports,
    })
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
    /// Buffer-sharing capability, or the reason there is none (ADR-019 §2 /
    /// #199).
    buffers: std::result::Result<BufferCapabilityInfo, BufferRefusal>,
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
        // ADR-019 §2 (#199). Read once, for the same reason: a capability that
        // changed mid-run could invalidate a view already handed to foreign
        // code.
        let buffers = negotiate_buffers(vtable, instance);
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
            buffers,
        })
    }

    /// This runtime's negotiated buffer-sharing capability, or the reason it
    /// has none (ADR-019 §2 / #199).
    pub fn buffer_capability(&self) -> std::result::Result<&BufferCapabilityInfo, BufferRefusal> {
        self.buffers.as_ref().map_err(|e| *e)
    }

    /// Invoke a compiled function with host buffers exported as call-scoped
    /// views (ADR-019 §2 / #199).
    ///
    /// Every pointer in `views` must stay valid until this returns, and the
    /// caller must not reclaim any of them while
    /// [`BufferInvokeOutcome::retained`] names them.
    ///
    /// # Safety
    ///
    /// `views` carries raw pointers into host memory that foreign code will
    /// read and — in [`shape_abi_v1::BUFFER_MODE_SHARED_MUT`] — write. The
    /// caller is responsible for the ADR-006 borrow discipline that makes that
    /// sound: a shared view must be reachable by no concurrent writer, and a
    /// mutable view by nothing else at all.
    pub unsafe fn invoke_with_buffers(
        &self,
        compiled: &CompiledForeignFunction,
        args_msgpack: &[u8],
        views: &[shape_abi_v1::ForeignBufferView],
    ) -> std::result::Result<BufferInvokeOutcome, BufferRefusal> {
        let cap = *self.buffers.as_ref().map_err(|e| *e)?;

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe {
            (cap.invoke_with_buffers)(
                self.state.instance,
                compiled.handle,
                args_msgpack.as_ptr(),
                args_msgpack.len(),
                views.as_ptr(),
                views.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };

        // Asked BEFORE the result buffer is interpreted or freed, and asked even
        // when the invoke failed: a body that raised part-way through is the
        // case most likely to have left a view alive, and the host must not
        // reclaim the memory on the strength of the error alone.
        let retained = unsafe { (cap.outstanding_exports)(self.state.instance) };

        let result = if rc != 0 {
            let msg = if !out_ptr.is_null() && out_len > 0 {
                let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
                if let Some(free_fn) = self.state.vtable.free_buffer {
                    unsafe { free_fn(out_ptr, out_len) };
                }
                String::from_utf8_lossy(&bytes).to_string()
            } else {
                format!("error code {}", rc)
            };
            Err(ShapeError::RuntimeError {
                message: format!(
                    "Language runtime '{}' invoke failed: {}",
                    self.language_id, msg
                ),
                location: None,
            })
        } else if out_ptr.is_null() || out_len == 0 {
            Ok(Vec::new())
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
            if let Some(free_fn) = self.state.vtable.free_buffer {
                unsafe { free_fn(out_ptr, out_len) };
            }
            Ok(bytes)
        };

        Ok(BufferInvokeOutcome { result, retained })
    }

    /// The off-thread-invocation model this runtime declares (ADR-019 §5 /
    /// #202). Drives whether an `async fn <language>` call can be offloaded,
    /// and if so onto what.
    pub fn instance_concurrency(&self) -> InstanceConcurrency {
        self.instance_concurrency
    }

    /// Whether this runtime can release foreign objects it hands out as opaque
    /// references (ADR-019 §3 / #200).
    ///
    /// `false` means the extension declares no `dispose_ref` entry. The host
    /// refuses to mint a foreign reference against such a runtime rather than
    /// creating one that would leak by construction.
    pub fn can_dispose_refs(&self) -> bool {
        self.state.vtable.dispose_ref.is_some()
    }

    /// Release a foreign object this instance minted (ADR-019 §3 / #200).
    ///
    /// Called from the foreign-reference carrier's `Drop`, so it neither
    /// returns nor reports: disposal is infallible in v1 and there is no caller
    /// to tell. A runtime with no `dispose_ref` entry never minted a reference
    /// in the first place ([`Self::can_dispose_refs`] gates that), so the
    /// no-entry case is unreachable rather than silently skipped.
    ///
    /// **This must be called on `self` = the instance that minted `handle`.**
    /// For a thread-affine runtime that is a specific worker's private
    /// instance, and calling it on any other — including the interpreter
    /// thread's — enters an isolate from the wrong thread.
    pub fn dispose_ref(&self, handle: u64) {
        if let Some(dispose) = self.state.vtable.dispose_ref {
            unsafe { dispose(self.state.instance, handle) };
        }
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

#[cfg(test)]
mod buffer_capability_negotiation_tests {
    //! ADR-019 §2 / R25 (POLY-ZERO-COPY, issue #199) — what the host reads out
    //! of an extension's capability block, and what it refuses.
    //!
    //! Driven against in-process fake vtables rather than a built `.so`: this
    //! asserts the HOST's reading, and must fail if the host starts trusting a
    //! block it cannot read exactly — independently of whether any interpreter
    //! is installed.
    //!
    //! The refusal that matters most is [`BufferRefusal::NoReleaseAccounting`].
    //! ADR-019 §2 fixes it as a refusal rather than a weaker guarantee, because
    //! the alternative is unpinning host memory that foreign code may still
    //! hold — the corruption class this whole capability exists to prevent.
    use super::*;
    use shape_abi_v1::{
        BUFFER_ELEM_FLOAT64, BUFFER_ELEM_INT64, BUFFER_MODE_SHARED, BUFFER_MODE_SHARED_MUT,
        BufferCapability, EXTENSION_CAPABILITIES_VERSION, ExtensionCapabilities, ForeignBufferView,
        STATE_MODEL_STATEFUL_OPAQUE,
    };
    use std::ffi::c_char;

    unsafe extern "C" fn init(_c: *const u8, _l: usize) -> *mut c_void {
        1usize as *mut c_void
    }
    unsafe extern "C" fn language_id(_i: *mut c_void) -> *const c_char {
        c"fake".as_ptr()
    }
    unsafe extern "C" fn drop_instance(_i: *mut c_void) {}
    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn compile(
        _i: *mut c_void,
        _n: *const u8,
        _nl: usize,
        _s: *const u8,
        _sl: usize,
        _pn: *const u8,
        _pnl: usize,
        _pt: *const u8,
        _ptl: usize,
        _rt: *const u8,
        _rtl: usize,
        _a: bool,
        _oe: *mut *mut u8,
        _oel: *mut usize,
    ) -> *mut c_void {
        1usize as *mut c_void
    }
    unsafe extern "C" fn invoke(
        _i: *mut c_void,
        _h: *mut c_void,
        _a: *const u8,
        _al: usize,
        _op: *mut *mut u8,
        _ol: *mut usize,
    ) -> i32 {
        0
    }
    unsafe extern "C" fn dispose_function(_i: *mut c_void, _h: *mut c_void) {}
    unsafe extern "C" fn free_buffer(ptr: *mut u8, len: usize) {
        if !ptr.is_null() {
            unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
        }
    }
    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn invoke_with_buffers(
        _i: *mut c_void,
        _h: *mut c_void,
        _a: *const u8,
        _al: usize,
        _v: *const ForeignBufferView,
        _vl: usize,
        _op: *mut *mut u8,
        _ol: *mut usize,
    ) -> i32 {
        0
    }
    unsafe extern "C" fn outstanding_exports(_i: *mut c_void) -> u64 {
        0
    }

    /// A complete buffer capability: both modes, both element types, release
    /// accounting present.
    static FULL_BUFFERS: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED | BUFFER_MODE_SHARED_MUT,
        elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: Some(outstanding_exports),
    };

    /// The ADR-019 §2 case: views offered, nothing that can say they were
    /// released.
    static UNACCOUNTED_BUFFERS: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED | BUFFER_MODE_SHARED_MUT,
        elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: None,
    };

    /// Read-only sharing only — a runtime that can hand out a view but not let
    /// foreign code write through it.
    static READ_ONLY_BUFFERS: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED,
        elem_types: 1 << BUFFER_ELEM_FLOAT64,
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: Some(outstanding_exports),
    };

    static CAPS_FULL: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &FULL_BUFFERS,
    };
    static CAPS_UNACCOUNTED: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &UNACCOUNTED_BUFFERS,
    };
    static CAPS_READ_ONLY: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &READ_ONLY_BUFFERS,
    };
    static CAPS_EMPTY: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: std::ptr::null(),
    };
    static CAPS_FUTURE_VERSION: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION + 7,
        buffers: &FULL_BUFFERS,
    };
    /// A block whose declared size stops short of the `buffers` pointer — the
    /// shape an extension built against a two-field definition would have.
    static CAPS_TRUNCATED: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: 8,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &FULL_BUFFERS,
    };

    macro_rules! caps_fn {
        ($name:ident, $block:expr) => {
            unsafe extern "C" fn $name(_i: *mut c_void) -> *const ExtensionCapabilities {
                $block
            }
        };
    }
    caps_fn!(caps_full, &CAPS_FULL);
    caps_fn!(caps_unaccounted, &CAPS_UNACCOUNTED);
    caps_fn!(caps_read_only, &CAPS_READ_ONLY);
    caps_fn!(caps_empty, &CAPS_EMPTY);
    caps_fn!(caps_future, &CAPS_FUTURE_VERSION);
    caps_fn!(caps_truncated, &CAPS_TRUNCATED);
    caps_fn!(caps_null, std::ptr::null());

    fn vtable_with(
        capabilities: Option<unsafe extern "C" fn(*mut c_void) -> *const ExtensionCapabilities>,
    ) -> &'static LanguageRuntimeVTable {
        // Leaked deliberately: `PluginLanguageRuntime` holds a `&'static`
        // vtable because a real one lives in a loaded `.so` for the process
        // lifetime. One leak per test case is the honest fake.
        Box::leak(Box::new(LanguageRuntimeVTable {
            init: Some(init),
            register_types: None,
            compile: Some(compile),
            invoke: Some(invoke),
            dispose_function: Some(dispose_function),
            language_id: Some(language_id),
            get_lsp_config: None,
            free_buffer: Some(free_buffer),
            drop: Some(drop_instance),
            error_model: ErrorModel::Dynamic,
            get_shape_source: None,
            runtime_descriptor: None,
            state_model: STATE_MODEL_STATEFUL_OPAQUE,
            generate_stubs: None,
            instance_concurrency: None,
            dispose_ref: None,
            capabilities,
        }))
    }

    fn runtime_with(
        capabilities: Option<unsafe extern "C" fn(*mut c_void) -> *const ExtensionCapabilities>,
    ) -> PluginLanguageRuntime {
        PluginLanguageRuntime::new(vtable_with(capabilities), &serde_json::Value::Null)
            .expect("the fake runtime initializes")
    }

    #[test]
    fn a_complete_capability_negotiates() {
        let runtime = runtime_with(Some(caps_full));
        let cap = runtime
            .buffer_capability()
            .expect("a complete block negotiates");
        assert!(cap.supports_mode(BUFFER_MODE_SHARED));
        assert!(cap.supports_mode(BUFFER_MODE_SHARED_MUT));
        assert!(cap.supports_elem(BUFFER_ELEM_INT64));
        assert!(cap.supports_elem(BUFFER_ELEM_FLOAT64));
    }

    #[test]
    fn a_runtime_without_release_accounting_has_the_mode_refused() {
        // ADR-019 §2's named refusal, and #199's tripwire (2) second half: a
        // language that cannot say whether a view was released does not get a
        // weakened mode, it gets no mode.
        let runtime = runtime_with(Some(caps_unaccounted));
        let refusal = runtime
            .buffer_capability()
            .err()
            .expect("views without accounting are refused");
        assert_eq!(refusal, BufferRefusal::NoReleaseAccounting);
        assert!(
            refusal.explain().contains("release accounting"),
            "the refusal says which half is missing, got: {}",
            refusal.explain()
        );
        assert!(
            refusal.explain().contains("rather than weakening it"),
            "the refusal states that it is a refusal by design, got: {}",
            refusal.explain()
        );
    }

    #[test]
    fn an_extension_declaring_no_capabilities_is_not_offered() {
        // Two spellings of the same truthful state: the slot is `None` (a
        // binary built before #199) or the accessor returns null.
        assert_eq!(
            runtime_with(None).buffer_capability().err(),
            Some(BufferRefusal::NotOffered)
        );
        assert_eq!(
            runtime_with(Some(caps_null)).buffer_capability().err(),
            Some(BufferRefusal::NotOffered)
        );
    }

    #[test]
    fn a_capability_block_with_no_buffers_is_not_offered() {
        // The TypeScript runtime's shape: it speaks the capability protocol and
        // declares that it has no buffer capability in it.
        assert_eq!(
            runtime_with(Some(caps_empty)).buffer_capability().err(),
            Some(BufferRefusal::NotOffered)
        );
    }

    #[test]
    fn an_unknown_capability_version_is_refused_whole() {
        let runtime = runtime_with(Some(caps_future));
        let refusal = runtime.buffer_capability().err().expect("refused");
        assert_eq!(
            refusal,
            BufferRefusal::UnknownVersion(EXTENSION_CAPABILITIES_VERSION + 7)
        );
        assert!(
            refusal.explain().contains("guessed offsets"),
            "the refusal says why reading it partly would be worse, got: {}",
            refusal.explain()
        );
    }

    #[test]
    fn a_block_shorter_than_its_fields_is_refused() {
        // The size guard is the whole extensibility mechanism: it is what lets
        // future capabilities be appended without another ABI bump, so it has to
        // hold in the direction that protects the host too.
        assert_eq!(
            runtime_with(Some(caps_truncated)).buffer_capability().err(),
            Some(BufferRefusal::Truncated)
        );
    }

    #[test]
    fn a_declared_mode_the_runtime_lacks_is_visible_to_the_caller() {
        // A read-only runtime negotiates, but `shared mut` against it is a
        // question the caller must still ask — the capability reports what it
        // has rather than the host assuming symmetry.
        let runtime = runtime_with(Some(caps_read_only));
        let cap = runtime.buffer_capability().expect("negotiates");
        assert!(cap.supports_mode(BUFFER_MODE_SHARED));
        assert!(!cap.supports_mode(BUFFER_MODE_SHARED_MUT));
        assert!(cap.supports_elem(BUFFER_ELEM_FLOAT64));
        assert!(!cap.supports_elem(BUFFER_ELEM_INT64));
    }
}
