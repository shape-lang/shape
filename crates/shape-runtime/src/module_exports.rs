//! Runtime module export bindings for Shape extensions.
//!
//! This module defines the in-process representation used by VM/LSP/CLI after
//! a plugin has been loaded through the ABI capability interfaces.

use crate::type_schema::{TypeSchema, TypeSchemaRegistry};
use shape_value::ValueWord;
use std::collections::HashMap;
use std::ffi::c_void;
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
///
/// **WB2.4 retain-on-read.** The `locals`, `upvalues`, and `args`
/// vectors hold owning shares of heap-tagged `ValueWord`s. Since
/// `ValueWord = u64`, the derived `Clone` on `Vec<ValueWord>` would
/// produce aliasing bit copies and the default `Vec` drop would leak
/// refcounts. The manual impls below `vw_clone` on copy and
/// `vw_drop_slice` on drop so each `FrameInfo` carries its own
/// refcount bumps.
#[derive(Debug)]
pub struct FrameInfo {
    pub function_id: Option<u16>,
    pub function_name: String,
    pub blob_hash: Option<[u8; 32]>,
    pub local_ip: usize,
    pub locals: Vec<ValueWord>,
    pub upvalues: Option<Vec<ValueWord>>,
    pub args: Vec<ValueWord>,
}

impl Clone for FrameInfo {
    fn clone(&self) -> Self {
        use shape_value::value_word_drop::vw_clone;
        let locals = self.locals.iter().map(|&b| vw_clone(b)).collect();
        let upvalues = self
            .upvalues
            .as_ref()
            .map(|v| v.iter().map(|&b| vw_clone(b)).collect());
        let args = self.args.iter().map(|&b| vw_clone(b)).collect();
        FrameInfo {
            function_id: self.function_id,
            function_name: self.function_name.clone(),
            blob_hash: self.blob_hash,
            local_ip: self.local_ip,
            locals,
            upvalues,
            args,
        }
    }
}

impl Drop for FrameInfo {
    fn drop(&mut self) {
        use shape_value::value_word_drop::vw_drop_slice;
        vw_drop_slice(&self.locals);
        if let Some(ref ups) = self.upvalues {
            vw_drop_slice(ups);
        }
        vw_drop_slice(&self.args);
    }
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

/// Check permission and enforce filesystem path scope constraints.
///
/// After verifying the base permission (`FsRead`, `FsWrite`, or `FsScoped`),
/// checks `ScopeConstraints::allowed_paths` when present. If the scope
/// constraints list paths, the target path must match at least one (prefix
/// match). An empty `allowed_paths` list means all paths are permitted.
pub fn check_fs_permission(
    ctx: &ModuleContext,
    permission: shape_abi_v1::Permission,
    path: &str,
) -> Result<(), String> {
    check_permission(ctx, permission)?;

    if let Some(ref constraints) = ctx.scope_constraints {
        if !constraints.allowed_paths.is_empty() {
            let target = std::path::Path::new(path);
            let allowed = constraints.allowed_paths.iter().any(|pattern| {
                // Support glob-style prefix matching: "/data/**" matches
                // anything under /data/, and "/tmp/*" matches direct children.
                let pattern = pattern.trim_end_matches("**").trim_end_matches('*');
                let prefix = std::path::Path::new(pattern.trim_end_matches('/'));
                target.starts_with(prefix)
            });
            if !allowed {
                return Err(format!(
                    "Scope constraint denied: path '{}' is not in allowed paths",
                    path
                ));
            }
        }
    }
    Ok(())
}

/// Check permission and enforce network host scope constraints.
///
/// After verifying the base permission (`NetConnect`, `NetListen`, or
/// `NetScoped`), checks `ScopeConstraints::allowed_hosts` when present.
/// If the scope constraints list hosts, the target address must match at
/// least one (supports `host:port` and `*.domain.com` wildcards).
pub fn check_net_permission(
    ctx: &ModuleContext,
    permission: shape_abi_v1::Permission,
    address: &str,
) -> Result<(), String> {
    check_permission(ctx, permission)?;

    if let Some(ref constraints) = ctx.scope_constraints {
        if !constraints.allowed_hosts.is_empty() {
            // Extract host (and optional port) from the address.
            let target_host = address.split(':').next().unwrap_or(address);
            let allowed = constraints.allowed_hosts.iter().any(|pattern| {
                let pattern_host = pattern.split(':').next().unwrap_or(pattern);
                // Wildcard: *.example.com matches sub.example.com
                if let Some(suffix) = pattern_host.strip_prefix("*.") {
                    target_host.ends_with(suffix) && target_host.len() > suffix.len()
                } else {
                    // Exact host match (port part is ignored for scope check)
                    target_host == pattern_host
                }
            });
            if !allowed {
                return Err(format!(
                    "Scope constraint denied: address '{}' is not in allowed hosts",
                    address
                ));
            }
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

/// One entry in the VM's per-process module-function table
/// (`module_fn_table`), indexed by `ValueWord::ModuleFunction(u32)`.
///
/// Phase 4c.4: the legacy `ModuleFn` ABI escape hatch was deleted. All
/// stdlib and test fixtures route through the typed registry.
///
/// - [`Self::Typed`]: synchronous typed-return native function. The
///   body returns [`crate::typed_module_exports::TypedReturn`] directly;
///   marshalling to `ValueWord` happens at the dispatch boundary.
/// - [`Self::TypedAsync`]: async typed-return native function. The body
///   returns a future of `TypedReturn`; the synchronous dispatch path
///   blocks on the future and marshals at the boundary.
#[derive(Clone)]
pub enum ModuleFnEntry {
    Typed(crate::typed_module_exports::TypedModuleFunction),
    TypedAsync(crate::typed_module_exports::TypedModuleAsyncFunction),
}

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
    /// Typed-return ABI registry (Phase 4b).
    ///
    /// Authoritative registry for native-module function bodies. Every
    /// export here declares its return type via
    /// [`crate::typed_module_exports::TypedReturn`] / [`crate::typed_module_exports::ConcreteType`];
    /// marshalling to `ValueWord` happens at the dispatch boundary
    /// inside the VM, not in the body. Phase 4c.4 deleted the legacy
    /// `exports`/`async_exports` `ModuleFn` parallel registry — every
    /// callable function body lives here.
    pub typed_exports: crate::typed_module_exports::TypedModuleExports,
}

impl ModuleExports {
    /// Create a new extension module.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            schemas: HashMap::new(),
            export_visibility: HashMap::new(),
            shape_sources: Vec::new(),
            module_artifacts: Vec::new(),
            method_intrinsics: HashMap::new(),
            type_schemas: Vec::new(),
            typed_exports: crate::typed_module_exports::TypedModuleExports::new(),
        }
    }

    /// Mutable access to the typed-return registry. Used by
    /// [`crate::typed_module_exports::register_typed_function`] to record
    /// the typed-body entry.
    pub fn typed_exports_mut(
        &mut self,
    ) -> &mut crate::typed_module_exports::TypedModuleExports {
        &mut self.typed_exports
    }

    /// Read-only access to the typed-return registry.
    pub fn typed_exports(&self) -> &crate::typed_module_exports::TypedModuleExports {
        &self.typed_exports
    }

    /// Register only the LSP/validation schema and visibility for an
    /// exported name. The actual function body lives in `typed_exports`
    /// and is dispatched directly via `ModuleFnEntry::Typed` /
    /// `ModuleFnEntry::TypedAsync` — see
    /// `register_typed_function`/`register_typed_async_function` and the
    /// test-only `register_test_function*` helpers.
    pub fn add_schema_only(
        &mut self,
        name: impl Into<String>,
        schema: ModuleFunction,
    ) -> &mut Self {
        let name = name.into();
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
        self.typed_exports.functions.contains_key(name)
            || self.typed_exports.async_functions.contains_key(name)
    }

    /// Invoke a sync export by name through the typed registry, marshalling
    /// the resulting `TypedReturn` to a `ValueWord` at the boundary.
    ///
    /// Returns `None` if the export doesn't exist (or is async). Used by
    /// stdlib-internal tests that previously did
    /// `module.get_export(name).unwrap()(&args, &ctx)` — the typed
    /// migration removes the legacy `ModuleFn` accessor in favor of this
    /// dispatch helper.
    pub fn invoke_export(
        &self,
        name: &str,
        args: &[ValueWord],
        ctx: &ModuleContext,
    ) -> Option<Result<ValueWord, String>> {
        let typed = self.typed_exports.functions.get(name)?;
        let typed_result = (typed.invoke)(args, ctx);
        Some(typed_result.map(|t| t.into_value_word()))
    }

    /// Check if a function is async.
    pub fn is_async(&self, name: &str) -> bool {
        self.typed_exports.async_functions.contains_key(name)
    }

    /// Get the schema for an exported function.
    pub fn get_schema(&self, name: &str) -> Option<&ModuleFunction> {
        self.schemas.get(name)
    }

    /// List all export names (sync + async).
    pub fn export_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .typed_exports
            .functions
            .keys()
            .chain(self.typed_exports.async_functions.keys())
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

    /// Return `ParsedModuleSchema` entries for all shipped native stdlib modules.
    /// Used during engine initialization to make these globals visible at compile time.
    pub fn stdlib_module_schemas() -> Vec<crate::extensions::ParsedModuleSchema> {
        crate::stdlib::all_stdlib_modules()
            .into_iter()
            .map(|m| m.to_parsed_schema())
            .collect()
    }
}

impl std::fmt::Debug for ModuleExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleExports")
            .field("name", &self.name)
            .field("description", &self.description)
            .field(
                "typed_exports",
                &self
                    .typed_exports
                    .functions
                    .keys()
                    .chain(self.typed_exports.async_functions.keys())
                    .collect::<Vec<_>>(),
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
/// Lookup is by canonical path only (e.g. `"std::core::json"`).
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
        let canonical = module.name.clone();
        self.modules.insert(canonical, module);
    }

    /// Get a module by canonical name.
    pub fn get(&self, name: &str) -> Option<&ModuleExports> {
        self.modules.get(name)
    }

    /// Check if a module exists by canonical name.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
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
