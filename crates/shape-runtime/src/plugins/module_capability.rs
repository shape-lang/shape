//! Base module capability wrapper (`shape.module`).
//!
//! Exposes plugin module functions as runtime `ModuleExports` exports.

use serde_json::Value;
use shape_abi_v1::{
    ModuleInvokeResult, ModuleInvokeResultKind, ModuleSchema as AbiModuleSchema, ModuleVTable,
    PluginError,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use shape_wire::{WireValue, render_any_error_plain};
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Deserialize)]
struct ArtifactPayload {
    module_path: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    compiled: Option<Vec<u8>>,
}

/// Parsed schema for one module export.
#[derive(Debug, Clone)]
pub struct ParsedModuleFunction {
    pub name: String,
    pub description: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
}

/// Parsed bundled module artifact.
#[derive(Debug, Clone)]
pub struct ParsedModuleArtifact {
    pub module_path: String,
    pub source: Option<String>,
    pub compiled: Option<Vec<u8>>,
}

/// Parsed `shape.module` schema.
#[derive(Debug, Clone)]
pub struct ParsedModuleSchema {
    pub module_name: String,
    pub functions: Vec<ParsedModuleFunction>,
    pub artifacts: Vec<ParsedModuleArtifact>,
}

/// Wrapper around the `shape.module` capability.
pub struct PluginModule {
    name: String,
    vtable: &'static ModuleVTable,
    instance: *mut c_void,
    schema: ParsedModuleSchema,
}

impl PluginModule {
    /// Create a new module-capability wrapper from a plugin vtable.
    pub fn new(name: String, vtable: &'static ModuleVTable, config: &Value) -> Result<Self> {
        let config_bytes = rmp_serde::to_vec(config).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize module config for '{}': {}", name, e),
            location: None,
        })?;

        let init_fn = vtable.init.ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Plugin '{}' module capability has no init function", name),
            location: None,
        })?;

        let instance = unsafe { init_fn(config_bytes.as_ptr(), config_bytes.len()) };
        if instance.is_null() {
            return Err(ShapeError::RuntimeError {
                message: format!("Plugin '{}' module init returned null", name),
                location: None,
            });
        }

        let schema = parse_module_schema(vtable, instance, &name)?;

        Ok(Self {
            name,
            vtable,
            instance,
            schema,
        })
    }

    /// Plugin/module name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Parsed module schema.
    pub fn schema(&self) -> &ParsedModuleSchema {
        &self.schema
    }

    /// Build a runtime `ModuleExports` wrapper for VM module dispatch.
    pub fn to_module_exports(&self) -> crate::module_exports::ModuleExports {
        use crate::module_exports::{ModuleExports, ModuleFunction, ModuleParam};

        let mut module = ModuleExports::new(self.schema.module_name.clone());
        module.description = format!("Plugin module exported by '{}'", self.name);

        let invoker = Arc::new(ModuleInvoker {
            name: self.name.clone(),
            vtable: self.vtable,
            instance: self.instance,
        });

        for function in &self.schema.functions {
            let fn_name = function.name.clone();
            let invoker_ref = Arc::clone(&invoker);

            let schema = ModuleFunction {
                description: function.description.clone(),
                params: function
                    .params
                    .iter()
                    .enumerate()
                    .map(|(idx, ty)| ModuleParam {
                        name: format!("arg{}", idx),
                        type_name: ty.clone(),
                        required: true,
                        description: String::new(),
                        ..Default::default()
                    })
                    .collect(),
                return_type: function.return_type.clone(),
            };

            let fn_name_for_closure = fn_name.clone();
            module.add_function_with_schema(
                fn_name,
                move |args: &[ValueWord], _ctx: &crate::module_exports::ModuleContext| {
                    invoker_ref.invoke_nb(&fn_name_for_closure, args)
                },
                schema,
            );
        }

        for artifact in &self.schema.artifacts {
            module.add_shape_artifact(
                artifact.module_path.clone(),
                artifact.source.clone(),
                artifact.compiled.clone(),
            );
        }

        module
    }

    /// Invoke one module export with shape-wire arguments/results.
    pub fn invoke_wire(&self, function: &str, args: &[WireValue]) -> Result<WireValue> {
        let invoker = ModuleInvoker {
            name: self.name.clone(),
            vtable: self.vtable,
            instance: self.instance,
        };
        invoker
            .invoke_wire(function, args)
            .map_err(|message| ShapeError::RuntimeError {
                message,
                location: None,
            })
    }

    /// Invoke one module export with ValueWord arguments/results.
    ///
    /// This is the primary host-side call path for runtime/LSP internals and
    /// uses `shape-wire` payloads end-to-end.
    pub fn invoke_nb(&self, function: &str, args: &[ValueWord]) -> Result<ValueWord> {
        let invoker = ModuleInvoker {
            name: self.name.clone(),
            vtable: self.vtable,
            instance: self.instance,
        };
        invoker
            .invoke_nb(function, args)
            .map_err(|message| ShapeError::RuntimeError {
                message,
                location: None,
            })
    }
}

impl Drop for PluginModule {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.vtable.drop {
            unsafe { drop_fn(self.instance) };
        }
    }
}

// SAFETY: access goes through plugin vtable calls that are required to be thread-safe.
unsafe impl Send for PluginModule {}
unsafe impl Sync for PluginModule {}

struct ModuleInvoker {
    name: String,
    vtable: &'static ModuleVTable,
    instance: *mut c_void,
}

impl ModuleInvoker {
    fn invoke_nb(
        &self,
        function: &str,
        args: &[ValueWord],
    ) -> std::result::Result<ValueWord, String> {
        let ctx = crate::Context::new_empty();
        let wire_args: Vec<WireValue> = args
            .iter()
            .map(|nb| crate::wire_conversion::nb_to_wire(nb, &ctx))
            .collect();

        let wire_bytes = rmp_serde::to_vec(&wire_args).map_err(|e| {
            format!(
                "Failed to serialize wire args for '{}.{}': {}",
                self.name, function, e
            )
        })?;

        match self
            .invoke_with_args(function, &wire_bytes)
            .map_err(|err| err.message)?
        {
            ModuleInvokePayload::Wire(bytes) => {
                let payload = decode_payload_to_wire(&bytes).map_err(|e| {
                    format!(
                        "Failed to decode module result for '{}.{}': {}",
                        self.name, function, e
                    )
                })?;
                let normalized = normalize_invoke_result(payload, &self.name, function)?;
                Ok(crate::wire_conversion::wire_to_nb(&normalized))
            }
            ModuleInvokePayload::TableArrowIpc(ipc_bytes) => {
                let dt = crate::wire_conversion::datatable_from_ipc_bytes(&ipc_bytes, None, None)
                    .map_err(|e| {
                    format!(
                        "Failed to decode table payload for '{}.{}': {}",
                        self.name, function, e
                    )
                })?;
                Ok(ValueWord::from_datatable(Arc::new(dt)))
            }
        }
    }

    fn invoke_wire(
        &self,
        function: &str,
        args: &[WireValue],
    ) -> std::result::Result<WireValue, String> {
        let wire_bytes = rmp_serde::to_vec(args).map_err(|e| {
            format!(
                "Failed to serialize wire args for '{}.{}': {}",
                self.name, function, e
            )
        })?;

        match self
            .invoke_with_args(function, &wire_bytes)
            .map_err(|err| err.message)?
        {
            ModuleInvokePayload::Wire(bytes) => {
                let payload = decode_payload_to_wire(&bytes).map_err(|e| {
                    format!(
                        "Failed to decode module result for '{}.{}': {}",
                        self.name, function, e
                    )
                })?;
                normalize_invoke_result(payload, &self.name, function)
            }
            ModuleInvokePayload::TableArrowIpc(ipc_bytes) => {
                let dt = crate::wire_conversion::datatable_from_ipc_bytes(&ipc_bytes, None, None)
                    .map_err(|e| {
                    format!(
                        "Failed to decode table payload for '{}.{}': {}",
                        self.name, function, e
                    )
                })?;
                let nb = ValueWord::from_datatable(Arc::new(dt));
                let ctx = crate::Context::new_empty();
                Ok(crate::wire_conversion::nb_to_wire(&nb, &ctx))
            }
        }
    }

    fn invoke_with_args(
        &self,
        function: &str,
        args_bytes: &[u8],
    ) -> std::result::Result<ModuleInvokePayload, ModuleInvokeFailure> {
        if let Some(invoke_ex_fn) = self.vtable.invoke_ex {
            let mut out = ModuleInvokeResult::empty();
            let status = unsafe {
                invoke_ex_fn(
                    self.instance,
                    function.as_ptr(),
                    function.len(),
                    args_bytes.as_ptr(),
                    args_bytes.len(),
                    &mut out,
                )
            };

            if status != PluginError::Success as i32 {
                return Err(ModuleInvokeFailure {
                    message: format!(
                        "Plugin '{}' module invoke_ex failed for '{}': status {}",
                        self.name, function, status
                    ),
                });
            }

            let payload = self.take_payload_bytes(out.payload_ptr, out.payload_len);
            return match out.kind {
                ModuleInvokeResultKind::WireValueMsgpack => Ok(ModuleInvokePayload::Wire(payload)),
                ModuleInvokeResultKind::TableArrowIpc => {
                    Ok(ModuleInvokePayload::TableArrowIpc(payload))
                }
            };
        }

        self.invoke_with_args_legacy(function, args_bytes)
    }

    fn invoke_with_args_legacy(
        &self,
        function: &str,
        args_bytes: &[u8],
    ) -> std::result::Result<ModuleInvokePayload, ModuleInvokeFailure> {
        let invoke_fn = self.vtable.invoke.ok_or_else(|| ModuleInvokeFailure {
            message: format!(
                "Plugin '{}' module capability does not implement invoke()",
                self.name
            ),
        })?;

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let status = unsafe {
            invoke_fn(
                self.instance,
                function.as_ptr(),
                function.len(),
                args_bytes.as_ptr(),
                args_bytes.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };

        if status != PluginError::Success as i32 {
            return Err(ModuleInvokeFailure {
                message: format!(
                    "Plugin '{}' module invoke failed for '{}': status {}",
                    self.name, function, status
                ),
            });
        }

        Ok(ModuleInvokePayload::Wire(
            self.take_payload_bytes(out_ptr, out_len),
        ))
    }

    fn take_payload_bytes(&self, ptr: *mut u8, len: usize) -> Vec<u8> {
        if ptr.is_null() {
            return Vec::new();
        }

        let bytes = if len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(ptr, len).to_vec() }
        };

        if let Some(free_fn) = self.vtable.free_buffer {
            unsafe { free_fn(ptr, len) };
        }
        bytes
    }
}

// SAFETY: access goes through plugin vtable calls that are required to be thread-safe.
unsafe impl Send for ModuleInvoker {}
unsafe impl Sync for ModuleInvoker {}

#[derive(Debug)]
struct ModuleInvokeFailure {
    message: String,
}

#[derive(Debug)]
enum ModuleInvokePayload {
    Wire(Vec<u8>),
    TableArrowIpc(Vec<u8>),
}

fn parse_module_schema(
    vtable: &'static ModuleVTable,
    instance: *mut c_void,
    plugin_name: &str,
) -> Result<ParsedModuleSchema> {
    let get_schema_fn = vtable
        .get_module_schema
        .ok_or_else(|| ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' module capability has no get_module_schema()",
                plugin_name
            ),
            location: None,
        })?;

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe { get_schema_fn(instance, &mut out_ptr, &mut out_len) };
    if status != PluginError::Success as i32 {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' get_module_schema failed with status {}",
                plugin_name, status
            ),
            location: None,
        });
    }

    if out_ptr.is_null() || out_len == 0 {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' returned empty module schema payload",
                plugin_name
            ),
            location: None,
        });
    }

    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len).to_vec() };
    if let Some(free_fn) = vtable.free_buffer {
        unsafe { free_fn(out_ptr, out_len) };
    }
    let schema: AbiModuleSchema =
        rmp_serde::from_slice(&bytes).map_err(|e| ShapeError::RuntimeError {
            message: format!(
                "Failed to decode module schema from '{}': {}",
                plugin_name, e
            ),
            location: None,
        })?;

    let module_name = if schema.module_name.is_empty() {
        plugin_name.to_string()
    } else {
        schema.module_name
    };

    let mut seen = HashSet::new();
    let mut functions = Vec::new();
    for f in schema.functions {
        if f.name.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Plugin '{}' module schema contains empty function name",
                    plugin_name
                ),
                location: None,
            });
        }
        if !seen.insert(f.name.clone()) {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Plugin '{}' module schema contains duplicate function '{}'",
                    plugin_name, f.name
                ),
                location: None,
            });
        }
        functions.push(ParsedModuleFunction {
            name: f.name,
            description: f.description,
            params: f.params,
            return_type: f.return_type,
        });
    }

    let artifacts = parse_module_artifacts(vtable, instance, plugin_name)?;

    Ok(ParsedModuleSchema {
        module_name,
        functions,
        artifacts,
    })
}

fn parse_module_artifacts(
    vtable: &'static ModuleVTable,
    instance: *mut c_void,
    plugin_name: &str,
) -> Result<Vec<ParsedModuleArtifact>> {
    let Some(get_artifacts_fn) = vtable.get_module_artifacts else {
        return Ok(Vec::new());
    };

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe { get_artifacts_fn(instance, &mut out_ptr, &mut out_len) };
    if status != PluginError::Success as i32 {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Plugin '{}' get_module_artifacts failed with status {}",
                plugin_name, status
            ),
            location: None,
        });
    }

    if out_ptr.is_null() || out_len == 0 {
        return Ok(Vec::new());
    }

    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len).to_vec() };
    if let Some(free_fn) = vtable.free_buffer {
        unsafe { free_fn(out_ptr, out_len) };
    }

    let parsed = rmp_serde::from_slice::<Vec<ArtifactPayload>>(&bytes).map_err(|e| {
        ShapeError::RuntimeError {
            message: format!(
                "Failed to decode module artifacts from '{}': {}",
                plugin_name, e
            ),
            location: None,
        }
    })?;

    let mut seen_paths = HashSet::new();
    let mut artifacts = Vec::new();
    for item in parsed {
        if item.module_path.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Plugin '{}' module artifacts contain empty module_path",
                    plugin_name
                ),
                location: None,
            });
        }
        if !seen_paths.insert(item.module_path.clone()) {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "Plugin '{}' module artifacts contain duplicate module_path '{}'",
                    plugin_name, item.module_path
                ),
                location: None,
            });
        }
        artifacts.push(ParsedModuleArtifact {
            module_path: item.module_path,
            source: item.source,
            compiled: item.compiled,
        });
    }

    Ok(artifacts)
}

fn decode_payload_to_wire(bytes: &[u8]) -> std::result::Result<WireValue, String> {
    if bytes.is_empty() {
        return Ok(WireValue::Null);
    }
    rmp_serde::from_slice::<WireValue>(bytes).map_err(|e| format!("invalid wire payload: {}", e))
}

fn normalize_invoke_result(
    payload: WireValue,
    module_name: &str,
    function: &str,
) -> std::result::Result<WireValue, String> {
    match payload {
        WireValue::Result { ok, value } => {
            if ok {
                Ok(*value)
            } else {
                Err(format!(
                    "Plugin '{}.{}' failed: {}",
                    module_name,
                    function,
                    format_wire_error_message(&value)
                ))
            }
        }
        other => Ok(other),
    }
}

fn format_wire_error_message(value: &WireValue) -> String {
    if let Some(rendered) = render_any_error_plain(value) {
        return rendered;
    }

    match value {
        WireValue::String(s) => s.clone(),
        WireValue::Object(map) => {
            if let Some(WireValue::String(message)) = map.get("message") {
                message.clone()
            } else {
                format!("{value:?}")
            }
        }
        _ => format!("{value:?}"),
    }
}
