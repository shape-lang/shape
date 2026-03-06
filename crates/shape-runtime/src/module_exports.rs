//! Runtime module export bindings for Shape extensions.
//!
//! This module defines the in-process representation used by VM/LSP/CLI after
//! a plugin has been loaded through the ABI capability interfaces.

use crate::type_schema::{TypeSchema, TypeSchemaRegistry};
use shape_value::ValueWord;
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Raw callable invoker as a function pointer + opaque context.
///
/// This is the `Send`-safe, `'static`-safe form of `invoke_callable` that
/// extensions (e.g., CFFI) can store in long-lived structs like callback
/// userdata.  The context pointer is valid for the duration of the
/// originating module function call.
#[derive(Clone, Copy)]
pub struct RawCallableInvoker {
    pub ctx: *mut c_void,
    pub invoke: unsafe fn(*mut c_void, &ValueWord, &[ValueWord]) -> Result<ValueWord, String>,
}

impl RawCallableInvoker {
    /// Invoke a Shape callable through this raw invoker.
    ///
    /// # Safety
    /// The caller must ensure `self.ctx` is still valid (i.e., the originating
    /// VM module call is still on the stack).
    pub unsafe fn call(
        &self,
        callable: &ValueWord,
        args: &[ValueWord],
    ) -> Result<ValueWord, String> {
        unsafe { (self.invoke)(self.ctx, callable, args) }
    }
}

/// Information about a single VM call frame, captured at a point in time.
#[derive(Debug, Clone)]
pub struct FrameInfo {
    pub function_id: Option<u16>,
    pub function_name: String,
    pub blob_hash: Option<[u8; 32]>,
    pub local_ip: usize,
    pub locals: Vec<ValueWord>,
    pub upvalues: Option<Vec<ValueWord>>,
    pub args: Vec<ValueWord>,
}

/// Trait providing read access to VM state for state module functions.
pub trait VmStateAccessor: Send + Sync {
    fn current_frame(&self) -> Option<FrameInfo>;
    fn all_frames(&self) -> Vec<FrameInfo>;
    fn caller_frame(&self) -> Option<FrameInfo>;
    fn current_args(&self) -> Vec<ValueWord>;
    fn current_locals(&self) -> Vec<(String, ValueWord)>;
    fn module_bindings(&self) -> Vec<(String, ValueWord)>;
    /// Total instruction count at the time of capture. Default impl for compat.
    fn instruction_count(&self) -> usize {
        0
    }
}

/// Execution context available to module functions during a VM call.
///
/// The VM constructs this before each module function dispatch and passes
/// it by reference.
pub struct ModuleContext<'a> {
    /// Type schema registry — lookup types by name or ID.
    pub schemas: &'a TypeSchemaRegistry,

    /// Invoke a Shape callable (function/closure) from host code.
    pub invoke_callable: Option<&'a dyn Fn(&ValueWord, &[ValueWord]) -> Result<ValueWord, String>>,

    /// Raw invoker for extensions that need to capture a callable invoker
    /// beyond the borrow lifetime (e.g., CFFI callback userdata).
    /// Valid only for the duration of the current module function call.
    pub raw_invoker: Option<RawCallableInvoker>,

    /// Content-addressed function hashes indexed by function ID.
    /// Provided by the VM when content-addressed metadata is available.
    /// Uses raw `[u8; 32]` to avoid a dependency on `shape-vm`'s `FunctionHash`.
    pub function_hashes: Option<&'a [Option<[u8; 32]>]>,

    /// Read-only access to VM state (call frames, locals, etc.).
    /// Provided by the VM when state introspection is needed.
    pub vm_state: Option<&'a dyn VmStateAccessor>,

    /// Permissions granted to the current execution context.
    /// When `Some`, module functions check this before performing I/O.
    /// When `None`, all operations are allowed (backwards compatible).
    pub granted_permissions: Option<shape_abi_v1::PermissionSet>,

    /// Scope constraints for the current execution context.
    /// Narrows permissions to specific paths, hosts, etc.
    pub scope_constraints: Option<shape_abi_v1::ScopeConstraints>,

    /// Callback for `state.resume()` to request full VM state restoration.
    /// The module function stores the snapshot; the dispatch loop applies it
    /// after the current instruction completes.
    pub set_pending_resume: Option<&'a dyn Fn(ValueWord)>,

    /// Callback for `state.resume_frame()` to request mid-function resume.
    /// Stores (ip_offset, locals) so the dispatch loop can override the
    /// call frame set up by invoke_callable.
    pub set_pending_frame_resume: Option<&'a dyn Fn(usize, Vec<ValueWord>)>,
}

/// Check whether the current execution context has a required permission.
///
/// If `granted_permissions` is `None`, all operations are allowed (backwards
/// compatible with code that predates the permission system). If `Some`, the
/// specific permission must be present in the set.
pub fn check_permission(
    ctx: &ModuleContext,
    permission: shape_abi_v1::Permission,
) -> Result<(), String> {
    if let Some(ref granted) = ctx.granted_permissions {
        if !granted.contains(&permission) {
            return Err(format!(
                "Permission denied: {} ({})",
                permission.description(),
                permission.name()
            ));
        }
    }
    Ok(())
}

/// A module function callable from Shape (synchronous).
///
/// Takes a slice of ValueWord arguments plus a `ModuleContext` that provides
/// access to the type schema registry and a callable invoker.
/// The function must be Send + Sync for thread safety.
pub type ModuleFn = Arc<
    dyn for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<ValueWord, String> + Send + Sync,
>;

/// An async module function callable from Shape.
///
/// Returns a boxed future that resolves to a ValueWord result.
/// The VM executor awaits this using the current tokio runtime.
///
/// Note: async functions do not receive a `ModuleContext` because the context
/// borrows from the VM and cannot be sent across await points.
pub type AsyncModuleFn = Arc<
    dyn Fn(&[ValueWord]) -> Pin<Box<dyn Future<Output = Result<ValueWord, String>> + Send>>
        + Send
        + Sync,
>;

/// Visibility policy for one extension export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleExportVisibility {
    /// Normal module API: available in runtime + comptime contexts.
    Public,
    /// Only callable from comptime contexts.
    ComptimeOnly,
    /// Internal helper: callable but hidden from normal user-facing discovery.
    Internal,
}

impl Default for ModuleExportVisibility {
    fn default() -> Self {
        Self::Public
    }
}

/// Schema for a single parameter of a module function.
/// Used by LSP for completions and by validation for type checking.
#[derive(Debug, Clone)]
pub struct ModuleParam {
    pub name: String,
    pub type_name: String,
    pub required: bool,
    pub description: String,
    pub default_snippet: Option<String>,
    pub allowed_values: Option<Vec<String>>,
    pub nested_params: Option<Vec<ModuleParam>>,
}

impl Default for ModuleParam {
    fn default() -> Self {
        Self {
            name: String::new(),
            type_name: "any".to_string(),
            required: false,
            description: String::new(),
            default_snippet: None,
            allowed_values: None,
            nested_params: None,
        }
    }
}

/// Schema for a module function — describes parameters and return type.
/// Used by LSP for completions, hover, and signature help.
#[derive(Debug, Clone)]
pub struct ModuleFunction {
    pub description: String,
    pub params: Vec<ModuleParam>,
    pub return_type: Option<String>,
}

/// Bundled module artifact from an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleArtifact {
    /// Import path for this module (e.g. "duckdb", "duckdb.query")
    pub module_path: String,
    /// Optional Shape source payload.
    pub source: Option<String>,
    /// Optional precompiled payload (opaque host format).
    pub compiled: Option<Vec<u8>>,
}

/// A Rust-implemented module exposed via `<name>`.
#[derive(Clone)]
pub struct ModuleExports {
    /// Module name (e.g., "csv", "json", "duckdb")
    pub name: String,
    /// Human-readable description of this module
    pub description: String,
    /// Exported sync functions: name → implementation
    pub exports: HashMap<String, ModuleFn>,
    /// Exported async functions: name → implementation
    pub async_exports: HashMap<String, AsyncModuleFn>,
    /// Function schemas for LSP + validation: name → schema
    pub schemas: HashMap<String, ModuleFunction>,
    /// Export visibility controls: name → visibility.
    pub export_visibility: HashMap<String, ModuleExportVisibility>,
    /// Shape source files bundled with this extension.
    /// Compiled and merged with core stdlib at startup.
    /// Vec of (filename, source_code) pairs.
    ///
    /// Legacy compatibility field. New code should use `module_artifacts`.
    pub shape_sources: Vec<(String, String)>,
    /// Bundled module artifacts (source/compiled/both).
    pub module_artifacts: Vec<ModuleArtifact>,
    /// Method intrinsics for fast dispatch on typed Objects.
    /// Outer key: type name (e.g., "DuckDbQuery")
    /// Inner key: method name (e.g., "build_sql")
    /// Dispatched BEFORE callable-property and UFCS fallback.
    pub method_intrinsics: HashMap<String, HashMap<String, ModuleFn>>,
    /// Type schemas to register in the VM's runtime TypeSchemaRegistry.
    /// Extensions can use this to declare types that the runtime can use
    /// for TypedObject creation and field validation.
    pub type_schemas: Vec<TypeSchema>,
}

impl ModuleExports {
    /// Create a new extension module.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            exports: HashMap::new(),
            async_exports: HashMap::new(),
            schemas: HashMap::new(),
            export_visibility: HashMap::new(),
            shape_sources: Vec::new(),
            module_artifacts: Vec::new(),
            method_intrinsics: HashMap::new(),
            type_schemas: Vec::new(),
        }
    }

    /// Register an exported function.
    pub fn add_function<F>(&mut self, name: impl Into<String>, f: F) -> &mut Self
    where
        F: for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<ValueWord, String>
            + Send
            + Sync
            + 'static,
    {
        let name = name.into();
        self.exports.insert(name.clone(), Arc::new(f));
        self.export_visibility.entry(name).or_default();
        self
    }

    /// Register an exported function with its schema.
    pub fn add_function_with_schema<F>(
        &mut self,
        name: impl Into<String>,
        f: F,
        schema: ModuleFunction,
    ) -> &mut Self
    where
        F: for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<ValueWord, String>
            + Send
            + Sync
            + 'static,
    {
        let name = name.into();
        self.exports.insert(name.clone(), Arc::new(f));
        self.schemas.insert(name.clone(), schema);
        self.export_visibility.entry(name).or_default();
        self
    }

    /// Register an async exported function.
    pub fn add_async_function<F, Fut>(&mut self, name: impl Into<String>, f: F) -> &mut Self
    where
        F: Fn(Vec<ValueWord>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ValueWord, String>> + Send + 'static,
    {
        let name = name.into();
        self.async_exports.insert(
            name.clone(),
            Arc::new(move |args: &[ValueWord]| {
                let owned_args = args.to_vec();
                Box::pin(f(owned_args))
            }),
        );
        self.export_visibility.entry(name).or_default();
        self
    }

    /// Register an async exported function with its schema.
    pub fn add_async_function_with_schema<F, Fut>(
        &mut self,
        name: impl Into<String>,
        f: F,
        schema: ModuleFunction,
    ) -> &mut Self
    where
        F: Fn(Vec<ValueWord>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ValueWord, String>> + Send + 'static,
    {
        let name = name.into();
        self.async_exports.insert(
            name.clone(),
            Arc::new(move |args: &[ValueWord]| {
                let owned_args = args.to_vec();
                Box::pin(f(owned_args))
            }),
        );
        self.schemas.insert(name.clone(), schema);
        self.export_visibility.entry(name).or_default();
        self
    }

    /// Set visibility for one export name.
    pub fn set_export_visibility(
        &mut self,
        name: impl Into<String>,
        visibility: ModuleExportVisibility,
    ) -> &mut Self {
        self.export_visibility.insert(name.into(), visibility);
        self
    }

    /// Resolve visibility for one export (defaults to Public).
    pub fn export_visibility(&self, name: &str) -> ModuleExportVisibility {
        self.export_visibility
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// Return true when the export can be called in the current compiler mode.
    pub fn is_export_available(&self, name: &str, comptime_mode: bool) -> bool {
        match self.export_visibility(name) {
            ModuleExportVisibility::Public => true,
            ModuleExportVisibility::ComptimeOnly => comptime_mode,
            ModuleExportVisibility::Internal => true,
        }
    }

    /// Return true when the export should appear in user-facing completion/hover surfaces.
    pub fn is_export_public_surface(&self, name: &str, comptime_mode: bool) -> bool {
        match self.export_visibility(name) {
            ModuleExportVisibility::Public => true,
            ModuleExportVisibility::ComptimeOnly => comptime_mode,
            ModuleExportVisibility::Internal => false,
        }
    }

    /// List exports available for the requested mode (sync + async).
    pub fn export_names_available(&self, comptime_mode: bool) -> Vec<&str> {
        self.export_names()
            .into_iter()
            .filter(|name| self.is_export_available(name, comptime_mode))
            .collect()
    }

    /// List user-facing exports for completion/hover (sync + async).
    pub fn export_names_public_surface(&self, comptime_mode: bool) -> Vec<&str> {
        self.export_names()
            .into_iter()
            .filter(|name| self.is_export_public_surface(name, comptime_mode))
            .collect()
    }

    /// Bundle a Shape source file with this extension.
    /// The source will be compiled and merged with stdlib at startup.
    pub fn add_shape_source(&mut self, filename: &str, source: &str) -> &mut Self {
        self.module_artifacts.push(ModuleArtifact {
            module_path: filename.to_string(),
            source: Some(source.to_string()),
            compiled: None,
        });
        self.shape_sources
            .push((filename.to_string(), source.to_string()));
        self
    }

    /// Register a bundled module artifact (source/compiled/both).
    pub fn add_shape_artifact(
        &mut self,
        module_path: impl Into<String>,
        source: Option<String>,
        compiled: Option<Vec<u8>>,
    ) -> &mut Self {
        self.module_artifacts.push(ModuleArtifact {
            module_path: module_path.into(),
            source,
            compiled,
        });
        self
    }

    /// Register a method intrinsic for fast dispatch on typed Objects.
    /// Called before callable-property and UFCS fallback in handle_object_method().
    pub fn add_intrinsic<F>(&mut self, type_name: &str, method_name: &str, f: F) -> &mut Self
    where
        F: for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<ValueWord, String>
            + Send
            + Sync
            + 'static,
    {
        self.method_intrinsics
            .entry(type_name.to_string())
            .or_default()
            .insert(method_name.to_string(), Arc::new(f));
        self
    }

    /// Register a type schema that the VM will add to its runtime registry.
    /// Returns the schema ID for reference.
    pub fn add_type_schema(&mut self, schema: TypeSchema) -> crate::type_schema::SchemaId {
        let id = schema.id;
        self.type_schemas.push(schema);
        id
    }

    /// Check if this module exports a given name (sync or async).
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.contains_key(name) || self.async_exports.contains_key(name)
    }

    /// Get a sync exported function by name.
    pub fn get_export(&self, name: &str) -> Option<&ModuleFn> {
        self.exports.get(name)
    }

    /// Get an async exported function by name.
    pub fn get_async_export(&self, name: &str) -> Option<&AsyncModuleFn> {
        self.async_exports.get(name)
    }

    /// Check if a function is async.
    pub fn is_async(&self, name: &str) -> bool {
        self.async_exports.contains_key(name)
    }

    /// Get the schema for an exported function.
    pub fn get_schema(&self, name: &str) -> Option<&ModuleFunction> {
        self.schemas.get(name)
    }

    /// List all export names (sync + async).
    pub fn export_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .exports
            .keys()
            .chain(self.async_exports.keys())
            .map(|s| s.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Convert this module's schema to a `ParsedModuleSchema` for the semantic
    /// analyzer, mirroring the conversion in `BytecodeExecutor::module_schemas()`.
    pub fn to_parsed_schema(&self) -> crate::extensions::ParsedModuleSchema {
        let functions = self
            .schemas
            .iter()
            .filter(|(name, _)| self.is_export_public_surface(name, false))
            .map(|(name, schema)| crate::extensions::ParsedModuleFunction {
                name: name.clone(),
                description: schema.description.clone(),
                params: schema.params.iter().map(|p| p.type_name.clone()).collect(),
                return_type: schema.return_type.clone(),
            })
            .collect();
        crate::extensions::ParsedModuleSchema {
            module_name: self.name.clone(),
            functions,
            artifacts: Vec::new(),
        }
    }

    /// Return `ParsedModuleSchema` entries for the VM-native stdlib modules
    /// (regex, http, crypto, env, log, json). Used by `SemanticAnalyzer::new()`
    /// to make these globals visible at compile time.
    pub fn stdlib_module_schemas() -> Vec<crate::extensions::ParsedModuleSchema> {
        vec![
            crate::stdlib::regex::create_regex_module().to_parsed_schema(),
            crate::stdlib::http::create_http_module().to_parsed_schema(),
            crate::stdlib::crypto::create_crypto_module().to_parsed_schema(),
            crate::stdlib::env::create_env_module().to_parsed_schema(),
            crate::stdlib::log::create_log_module().to_parsed_schema(),
            crate::stdlib::json::create_json_module().to_parsed_schema(),
            crate::stdlib::toml_module::create_toml_module().to_parsed_schema(),
            crate::stdlib::yaml::create_yaml_module().to_parsed_schema(),
            crate::stdlib::xml::create_xml_module().to_parsed_schema(),
            crate::stdlib::compress::create_compress_module().to_parsed_schema(),
            crate::stdlib::archive::create_archive_module().to_parsed_schema(),
            crate::stdlib::parallel::create_parallel_module().to_parsed_schema(),
            crate::stdlib::unicode::create_unicode_module().to_parsed_schema(),
        ]
    }
}

impl std::fmt::Debug for ModuleExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleExports")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("exports", &self.exports.keys().collect::<Vec<_>>())
            .field(
                "async_exports",
                &self.async_exports.keys().collect::<Vec<_>>(),
            )
            .field("schemas", &self.schemas.keys().collect::<Vec<_>>())
            .field(
                "shape_sources",
                &self
                    .shape_sources
                    .iter()
                    .map(|(f, _)| f)
                    .collect::<Vec<_>>(),
            )
            .field(
                "method_intrinsics",
                &self.method_intrinsics.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Registry of all extension modules.
///
/// Created at startup and populated from loaded plugin capabilities.
#[derive(Default)]
pub struct ModuleExportRegistry {
    modules: HashMap<String, ModuleExports>,
}

impl ModuleExportRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    /// Register a extension module.
    pub fn register(&mut self, module: ModuleExports) {
        self.modules.insert(module.name.clone(), module);
    }

    /// Get a module by name.
    pub fn get(&self, name: &str) -> Option<&ModuleExports> {
        self.modules.get(name)
    }

    /// Check if a module exists.
    pub fn has(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// List all registered module names.
    pub fn module_names(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }

    /// Get all registered modules.
    pub fn modules(&self) -> &HashMap<String, ModuleExports> {
        &self.modules
    }
}

impl std::fmt::Debug for ModuleExportRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleExportRegistry")
            .field("modules", &self.modules.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
#[path = "module_exports_tests.rs"]
mod tests;
