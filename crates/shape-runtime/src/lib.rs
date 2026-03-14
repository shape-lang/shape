// ShapeError carries location info for good diagnostics, making it larger than clippy's threshold.
#![allow(clippy::result_large_err)]

//! Runtime module for Shape execution
//!
//! This module contains the pattern matching engine, query executor,
//! and evaluation logic for the Shape language.
//!
//! Shape is a general-purpose scientific computing language for high-speed
//! time-series analysis. The runtime is domain-agnostic - all domain-specific
//! logic (finance, IoT, sensors, etc.) belongs in stdlib modules.

// Import from shape-value (moved there to break circular dependency)
pub use shape_value::AlignedVec;

pub mod alerts;
pub mod annotation_context;
pub mod arrow_c;
pub mod ast_extensions;
pub mod binary_reader;
pub mod blob_prefetch;
pub mod blob_store;
pub mod blob_wire_format;
pub mod builtin_metadata;
pub mod chart_detect;
pub mod closure;
pub mod code_search;
pub mod columnar_aggregations;
pub mod const_eval;
pub mod content_builders;
pub mod content_dispatch;
pub mod content_methods;
pub mod content_renderer;
pub mod context;
pub mod crypto;
pub mod data;
pub mod dependency_resolver;
pub mod distributed_gc;
pub mod doc_extract;
pub mod engine;
pub mod event_queue;
pub mod execution_proof;
pub mod extension_context;
pub mod extensions;
pub mod extensions_config;
pub mod frontmatter;
pub mod fuzzy;
pub mod fuzzy_property;
pub mod hashing;
pub mod intrinsics;
pub mod join_executor;
pub mod leakage;
pub mod lookahead_guard;
pub mod metadata;
pub mod module_bindings;
pub mod module_exports;
pub mod module_loader;
pub mod module_manifest;
pub mod multi_table;
pub mod multiple_testing;
pub mod native_resolution;
pub mod output_adapter;
pub mod package_bundle;
pub mod package_lock;
pub mod pattern_library;
pub mod pattern_state_machine;
pub mod plugins;
pub mod progress;
pub mod project;
#[cfg(all(test, feature = "deep-tests"))]
mod project_deep_tests;
pub mod provider_registry;
pub mod query_builder;
pub mod query_executor;
pub mod query_result;
pub mod renderers;
pub mod schema_cache;
pub mod schema_inference;
pub mod simd_comparisons;
pub mod simd_forward_fill;
pub mod simd_i64;
pub mod simd_rolling;
pub mod simd_statistics;
pub mod simulation;
pub mod snapshot;
pub mod state_diff;
pub mod statistics;
pub mod stdlib;
pub mod stdlib_io;
pub mod stdlib_metadata;
pub mod stdlib_time;
pub mod stream_executor;
pub mod sync_bridge;
pub mod time_window;
pub mod timeframe_utils;
pub mod type_mapping;
pub mod type_methods;
pub mod type_schema;
pub mod type_system;
pub mod visitor;
pub mod window_executor;
pub mod window_manager;
pub mod wire_conversion;

// Export commonly used types
pub use alerts::{Alert, AlertRouter, AlertSeverity, AlertSink};
pub use context::{DataLoadMode, ExecutionContext as Context};
pub use data::DataFrame;
pub use data::OwnedDataRow as RowValue;
pub use event_queue::{
    EventQueue, MemoryEventQueue, QueuedEvent, SharedEventQueue, SuspensionState, WaitCondition,
    create_event_queue,
};
pub use extensions::{
    ExtensionCapability, ExtensionDataSource, ExtensionLoader, ExtensionOutputSink,
    LoadedExtension, ParsedOutputField, ParsedOutputSchema, ParsedQueryParam, ParsedQuerySchema,
};
pub use extensions_config::{
    ExtensionEntry as GlobalExtensionEntry, ExtensionsConfig as GlobalExtensionsConfig,
    load_extensions_config, load_extensions_config_from,
};
pub use hashing::{HashDigest, combine_hashes, hash_bytes, hash_file, hash_string};
pub use intrinsics::{IntrinsicFn, IntrinsicsRegistry};
pub use join_executor::JoinExecutor;
pub use leakage::{LeakageDetector, LeakageReport, LeakageSeverity, LeakageType, LeakageWarning};
pub use module_bindings::ModuleBindingRegistry;
pub use module_exports::{
    FrameInfo, ModuleContext, ModuleExportRegistry, ModuleExports, ModuleFn, VmStateAccessor,
};
pub use multiple_testing::{MultipleTestingGuard, MultipleTestingStats, WarningLevel};
pub use progress::{
    LoadPhase, ProgressEvent, ProgressGranularity, ProgressHandle, ProgressRegistry,
};
pub use query_result::{AlertResult, QueryResult, QueryType};
use shape_value::ValueWord;
pub use shape_value::ValueWord as Value;
pub use stream_executor::{StreamEvent, StreamExecutor, StreamState};
pub use sync_bridge::{
    SyncDataProvider, block_on_shared, get_runtime_handle, initialize_shared_runtime,
};
pub use type_schema::{
    FieldDef, FieldType, SchemaId, TypeSchema, TypeSchemaBuilder, TypeSchemaRegistry,
};
pub use window_executor::WindowExecutor;
pub use wire_conversion::{
    nb_extract_typed_value, nb_to_envelope, nb_to_wire, nb_typed_value_to_envelope, wire_to_nb,
};

use self::type_methods::TypeMethodRegistry;
pub use error::{Result, ShapeError, SourceLocation};
use shape_ast::ast::{Program, Query};
pub use shape_ast::error;
use shape_wire::WireValue;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

/// The main runtime engine for Shape
pub struct Runtime {
    /// Module loader for .shape files
    module_loader: module_loader::ModuleLoader,
    /// Persistent execution context for REPL
    persistent_context: Option<context::ExecutionContext>,
    /// Shared type method registry for user-defined methods
    type_method_registry: Arc<TypeMethodRegistry>,
    /// Registry for annotation definitions (`annotation warmup() { ... }`, etc.)
    annotation_registry: Arc<RwLock<annotation_context::AnnotationRegistry>>,
    /// Module binding registry shared with VM/JIT execution pipelines.
    module_binding_registry: Arc<RwLock<module_bindings::ModuleBindingRegistry>>,
    /// Debug mode flag — enables verbose logging/tracing when true
    debug_mode: bool,
    /// Execution timeout — the executor can check elapsed time against this
    execution_timeout: Option<Duration>,
    /// Memory limit in bytes — allocation tracking can reference this
    memory_limit: Option<usize>,
    /// Last structured runtime error payload produced by execution.
    ///
    /// Hosts can consume this to render AnyError with target-specific
    /// renderers (ANSI/HTML/plain) without parsing flat strings.
    last_runtime_error: Option<WireValue>,
    /// Optional blob store for content-addressed function blobs.
    blob_store: Option<Arc<dyn crate::blob_store::BlobStore>>,
    /// Optional keychain for module signature verification.
    keychain: Option<crate::crypto::keychain::Keychain>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// Create a new runtime instance
    pub fn new() -> Self {
        Self::new_internal(true)
    }

    pub(crate) fn new_without_stdlib() -> Self {
        Self::new_internal(false)
    }

    fn new_internal(_load_stdlib: bool) -> Self {
        let module_loader = module_loader::ModuleLoader::new();
        let module_binding_registry =
            Arc::new(RwLock::new(module_bindings::ModuleBindingRegistry::new()));

        Self {
            module_loader,
            persistent_context: None,
            type_method_registry: Arc::new(TypeMethodRegistry::new()),
            annotation_registry: Arc::new(RwLock::new(
                annotation_context::AnnotationRegistry::new(),
            )),
            module_binding_registry,
            debug_mode: false,
            execution_timeout: None,
            memory_limit: None,
            last_runtime_error: None,
            blob_store: None,
            keychain: None,
        }
    }

    pub fn annotation_registry(&self) -> Arc<RwLock<annotation_context::AnnotationRegistry>> {
        self.annotation_registry.clone()
    }

    pub fn enable_persistent_context(&mut self, data: &DataFrame) {
        self.persistent_context = Some(context::ExecutionContext::new_with_registry(
            data,
            self.type_method_registry.clone(),
        ));
    }

    pub fn enable_persistent_context_without_data(&mut self) {
        self.persistent_context = Some(context::ExecutionContext::new_empty_with_registry(
            self.type_method_registry.clone(),
        ));
    }

    pub fn set_persistent_context(&mut self, ctx: context::ExecutionContext) {
        self.persistent_context = Some(ctx);
    }

    pub fn persistent_context(&self) -> Option<&context::ExecutionContext> {
        self.persistent_context.as_ref()
    }

    pub fn persistent_context_mut(&mut self) -> Option<&mut context::ExecutionContext> {
        self.persistent_context.as_mut()
    }

    /// Store the last structured runtime error payload.
    pub fn set_last_runtime_error(&mut self, payload: Option<WireValue>) {
        self.last_runtime_error = payload;
    }

    /// Clear any stored structured runtime error payload.
    pub fn clear_last_runtime_error(&mut self) {
        self.last_runtime_error = None;
    }

    /// Borrow the last structured runtime error payload.
    pub fn last_runtime_error(&self) -> Option<&WireValue> {
        self.last_runtime_error.as_ref()
    }

    /// Take the last structured runtime error payload.
    pub fn take_last_runtime_error(&mut self) -> Option<WireValue> {
        self.last_runtime_error.take()
    }

    pub fn type_method_registry(&self) -> &Arc<TypeMethodRegistry> {
        &self.type_method_registry
    }

    /// Get the module-binding registry shared with VM/JIT execution.
    pub fn module_binding_registry(&self) -> Arc<RwLock<module_bindings::ModuleBindingRegistry>> {
        self.module_binding_registry.clone()
    }

    /// Add a module search path for imports
    ///
    /// This is useful when executing scripts - add the script's directory
    /// to the module search paths for resolution.
    pub fn add_module_path(&mut self, path: PathBuf) {
        self.module_loader.add_module_path(path);
    }

    /// Set the keychain for module signature verification.
    ///
    /// Propagates to the module loader so it can verify module signatures
    /// at load time.
    pub fn set_keychain(&mut self, keychain: crate::crypto::keychain::Keychain) {
        self.keychain = Some(keychain.clone());
        self.module_loader.set_keychain(keychain);
    }

    /// Set the blob store for content-addressed function blobs.
    ///
    /// Propagates to the module loader so it can lazily fetch blobs
    /// not found in inline caches.
    pub fn set_blob_store(&mut self, store: Arc<dyn crate::blob_store::BlobStore>) {
        self.blob_store = Some(store.clone());
        self.module_loader.set_blob_store(store);
    }

    /// Set the project root and prepend its configured module paths
    pub fn set_project_root(&mut self, root: &std::path::Path, extra_paths: &[PathBuf]) {
        self.module_loader.set_project_root(root, extra_paths);
    }

    /// Set resolved dependency paths for the module loader
    pub fn set_dependency_paths(
        &mut self,
        deps: std::collections::HashMap<String, std::path::PathBuf>,
    ) {
        self.module_loader.set_dependency_paths(deps);
    }

    /// Get the resolved dependency paths from the module loader.
    pub fn get_dependency_paths(&self) -> &std::collections::HashMap<String, std::path::PathBuf> {
        self.module_loader.get_dependency_paths()
    }

    /// Register extension-provided module artifacts into the unified module loader.
    pub fn register_extension_module_artifacts(
        &mut self,
        modules: &[crate::extensions::ParsedModuleSchema],
    ) {
        for module in modules {
            for artifact in &module.artifacts {
                let code = match (&artifact.source, &artifact.compiled) {
                    (Some(source), Some(compiled)) => module_loader::ModuleCode::Both {
                        source: Arc::from(source.as_str()),
                        compiled: Arc::from(compiled.clone()),
                    },
                    (Some(source), None) => {
                        module_loader::ModuleCode::Source(Arc::from(source.as_str()))
                    }
                    (None, Some(compiled)) => {
                        module_loader::ModuleCode::Compiled(Arc::from(compiled.clone()))
                    }
                    (None, None) => continue,
                };
                self.module_loader
                    .register_extension_module(artifact.module_path.clone(), code);
            }
        }
    }

    /// Build a fresh module loader with the same search/dependency settings.
    ///
    /// This is used by external executors (VM/JIT) so import resolution stays
    /// aligned with runtime configuration.
    pub fn configured_module_loader(&self) -> module_loader::ModuleLoader {
        let mut loader = self.module_loader.clone_without_cache();
        if let Some(ref store) = self.blob_store {
            loader.set_blob_store(store.clone());
        }
        if let Some(ref kc) = self.keychain {
            loader.set_keychain(kc.clone());
        }
        loader
    }

    /// Load `std::core::*` modules via the unified module loader and register them in runtime context.
    ///
    /// This is the canonical stdlib bootstrap path used by the engine and CLI.
    pub fn load_core_stdlib_into_context(&mut self, data: &DataFrame) -> Result<()> {
        let module_paths = self.module_loader.list_core_stdlib_module_imports()?;

        for module_path in module_paths {
            // Skip the prelude module — it contains re-export imports that reference
            // non-exported symbols (traits, constants). Prelude injection is handled
            // separately by prepend_prelude_items() in the bytecode executor.
            if module_path == "std::core::prelude" {
                continue;
            }

            let resolved = self.module_loader.resolve_module_path(&module_path).ok();
            let context_dir = resolved
                .as_ref()
                .and_then(|path| path.parent().map(|p| p.to_path_buf()));
            let module = self.module_loader.load_module(&module_path)?;
            self.load_program_with_context(&module.ast, data, context_dir.as_ref())?;
        }

        Ok(())
    }

    pub fn load_program(&mut self, program: &Program, data: &DataFrame) -> Result<()> {
        self.load_program_with_context(program, data, None)
    }

    pub(crate) fn load_program_with_context(
        &mut self,
        program: &Program,
        data: &DataFrame,
        context_dir: Option<&PathBuf>,
    ) -> Result<()> {
        let mut persistent_ctx = self.persistent_context.take();

        let result = if let Some(ref mut ctx) = persistent_ctx {
            if data.row_count() > 0 {
                ctx.update_data(data);
            }
            self.process_program_items(program, ctx, context_dir)
        } else {
            let mut ctx = context::ExecutionContext::new_with_registry(
                data,
                self.type_method_registry.clone(),
            );
            self.process_program_items(program, &mut ctx, context_dir)
        };

        self.persistent_context = persistent_ctx;
        result
    }

    fn process_program_items(
        &mut self,
        program: &Program,
        ctx: &mut context::ExecutionContext,
        context_dir: Option<&PathBuf>,
    ) -> Result<()> {
        for item in &program.items {
            match item {
                shape_ast::ast::Item::Import(import, _) => {
                    let module = self
                        .module_loader
                        .load_module_with_context(&import.from, context_dir)?;

                    match &import.items {
                        shape_ast::ast::ImportItems::Named(imports) => {
                            for import_spec in imports {
                                if let Some(export) = module.exports.get(&import_spec.name) {
                                    if import_spec.is_annotation {
                                        continue;
                                    }
                                    let var_name =
                                        import_spec.alias.as_ref().unwrap_or(&import_spec.name);
                                    match export {
                                        module_loader::Export::Function(_) => {
                                            // Function exports are registered by the VM
                                        }
                                        module_loader::Export::Value(value) => {
                                            if ctx.get_variable(var_name)?.is_none() {
                                                ctx.set_variable(var_name, value.clone())?;
                                            }
                                            self.module_binding_registry
                                                .write()
                                                .unwrap()
                                                .register_const(var_name, value.clone())?;
                                        }
                                        _ => {}
                                    }
                                } else {
                                    return Err(ShapeError::ModuleError {
                                        message: format!(
                                            "Export '{}' not found in module '{}'",
                                            import_spec.name, import.from
                                        ),
                                        module_path: Some(import.from.clone().into()),
                                    });
                                }
                            }
                        }
                        shape_ast::ast::ImportItems::Namespace { .. } => {
                            // Namespace imports for extension modules are handled by the VM
                        }
                    }
                }
                shape_ast::ast::Item::Export(export, _) => {
                    match &export.item {
                        shape_ast::ast::ExportItem::Function(_) => {
                            // Function exports are registered by the VM
                        }
                        shape_ast::ast::ExportItem::BuiltinFunction(_)
                        | shape_ast::ast::ExportItem::BuiltinType(_)
                        | shape_ast::ast::ExportItem::Annotation(_) => {}
                        shape_ast::ast::ExportItem::Named(specs) => {
                            for spec in specs {
                                if let Ok(value) = ctx.get_variable(&spec.name) {
                                    if value.is_none() {
                                        return Err(ShapeError::RuntimeError {
                                            message: format!(
                                                "Cannot export undefined variable '{}'",
                                                spec.name
                                            ),
                                            location: None,
                                        });
                                    }
                                }
                            }
                        }
                        shape_ast::ast::ExportItem::TypeAlias(alias_def) => {
                            let overrides = HashMap::new();
                            if let Some(ref overrides_ast) = alias_def.meta_param_overrides {
                                for (_key, _expr) in overrides_ast {
                                    // TODO: Use const_eval for simple literal evaluation
                                }
                            }

                            let base_type = match &alias_def.type_annotation {
                                shape_ast::ast::TypeAnnotation::Basic(n) => n.clone(),
                                shape_ast::ast::TypeAnnotation::Reference(n) => n.to_string(),
                                _ => "any".to_string(),
                            };

                            ctx.register_type_alias(&alias_def.name, &base_type, Some(overrides));
                        }
                        shape_ast::ast::ExportItem::Enum(enum_def) => {
                            ctx.register_enum(enum_def.clone());
                        }
                        shape_ast::ast::ExportItem::Struct(struct_def) => {
                            ctx.register_struct_type(struct_def.clone());
                        }
                        shape_ast::ast::ExportItem::Interface(_)
                        | shape_ast::ast::ExportItem::Trait(_) => {
                            // Type definitions handled at compile time
                        }
                        shape_ast::ast::ExportItem::ForeignFunction(_) => {
                            // Foreign function exports are registered by the VM
                        }
                    }
                }
                shape_ast::ast::Item::Function(_function, _) => {
                    // Functions are registered by the VM
                }
                shape_ast::ast::Item::TypeAlias(alias_def, _) => {
                    let overrides = HashMap::new();
                    if let Some(ref overrides_ast) = alias_def.meta_param_overrides {
                        for (_key, _expr) in overrides_ast {
                            // TODO: Use const_eval for simple literal evaluation
                        }
                    }

                    let base_type = match &alias_def.type_annotation {
                        shape_ast::ast::TypeAnnotation::Basic(n) => n.clone(),
                        shape_ast::ast::TypeAnnotation::Reference(n) => n.to_string(),
                        _ => "any".to_string(),
                    };

                    ctx.register_type_alias(&alias_def.name, &base_type, Some(overrides));
                }
                shape_ast::ast::Item::Interface(_, _) => {}
                shape_ast::ast::Item::Trait(_, _) => {}
                shape_ast::ast::Item::Impl(_, _) => {}
                shape_ast::ast::Item::Enum(enum_def, _) => {
                    ctx.register_enum(enum_def.clone());
                }
                shape_ast::ast::Item::StructType(struct_def, _) => {
                    ctx.register_struct_type(struct_def.clone());
                }
                shape_ast::ast::Item::Extend(extend_stmt, _) => {
                    let registry = ctx.type_method_registry();
                    for method in &extend_stmt.methods {
                        registry.register_method(&extend_stmt.type_name, method.clone());
                    }
                }
                shape_ast::ast::Item::AnnotationDef(ann_def, _) => {
                    self.annotation_registry
                        .write()
                        .unwrap()
                        .register(ann_def.clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn execute_query(
        &mut self,
        query: &shape_ast::ast::Item,
        data: &DataFrame,
    ) -> Result<QueryResult> {
        let mut persistent_ctx = self.persistent_context.take();
        let result = if let Some(ref mut ctx) = persistent_ctx {
            ctx.update_data(data);
            self.execute_query_with_context(query, ctx)
        } else {
            let mut ctx = context::ExecutionContext::new_with_registry(
                data,
                self.type_method_registry.clone(),
            );
            self.execute_query_with_context(query, &mut ctx)
        };
        self.persistent_context = persistent_ctx;
        result
    }

    fn execute_query_with_context(
        &mut self,
        query: &shape_ast::ast::Item,
        ctx: &mut context::ExecutionContext,
    ) -> Result<QueryResult> {
        let id = ctx.get_current_id().unwrap_or_default();
        let timeframe = ctx
            .get_current_timeframe()
            .map(|t| t.to_string())
            .unwrap_or_default();

        match query {
            shape_ast::ast::Item::Query(q, _) => match q {
                Query::Backtest(_) => Err(ShapeError::RuntimeError {
                    message: "Backtesting not supported in core runtime.".to_string(),
                    location: None,
                }),
                Query::Alert(_alert_query) => {
                    let alert_id = format!("alert_{}", chrono::Utc::now().timestamp_micros());
                    Ok(
                        QueryResult::new(QueryType::Alert, id, timeframe).with_alert(AlertResult {
                            id: alert_id,
                            active: false,
                            message: "Alert triggered".to_string(),
                            level: "info".to_string(),
                            timestamp: chrono::Utc::now(),
                        }),
                    )
                }
                Query::With(with_query) => {
                    for cte in &with_query.ctes {
                        let cte_result = self.execute_query_with_context(
                            &shape_ast::ast::Item::Query(
                                (*cte.query).clone(),
                                shape_ast::ast::Span::DUMMY,
                            ),
                            ctx,
                        )?;
                        let value = cte_result.value.unwrap_or(ValueWord::none());
                        ctx.set_variable_nb(&cte.name, value)?;
                    }
                    self.execute_query_with_context(
                        &shape_ast::ast::Item::Query(
                            (*with_query.query).clone(),
                            shape_ast::ast::Span::DUMMY,
                        ),
                        ctx,
                    )
                }
            },
            shape_ast::ast::Item::Expression(_, _) => {
                Ok(QueryResult::new(QueryType::Value, id, timeframe).with_value(ValueWord::none()))
            }
            shape_ast::ast::Item::VariableDecl(var_decl, _) => {
                ctx.declare_pattern(&var_decl.pattern, var_decl.kind, ValueWord::none())?;
                Ok(QueryResult::new(QueryType::Value, id, timeframe).with_value(ValueWord::none()))
            }
            shape_ast::ast::Item::Assignment(assignment, _) => {
                ctx.set_pattern(&assignment.pattern, ValueWord::none())?;
                Ok(QueryResult::new(QueryType::Value, id, timeframe).with_value(ValueWord::none()))
            }
            shape_ast::ast::Item::Statement(_, _) => {
                Ok(QueryResult::new(QueryType::Value, id, timeframe).with_value(ValueWord::none()))
            }
            _ => Err(ShapeError::RuntimeError {
                message: format!("Unsupported item for query execution: {:?}", query),
                location: None,
            }),
        }
    }

    pub fn execute_without_data(&mut self, item: &shape_ast::ast::Item) -> Result<QueryResult> {
        let mut persistent_ctx = self.persistent_context.take();

        let result = if let Some(ref mut ctx) = persistent_ctx {
            match item {
                shape_ast::ast::Item::Expression(_, _) => {
                    Ok(
                        QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                            .with_value(ValueWord::none()),
                    )
                }
                shape_ast::ast::Item::Statement(_, _) => {
                    Ok(
                        QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                            .with_value(ValueWord::none()),
                    )
                }
                shape_ast::ast::Item::VariableDecl(var_decl, _) => {
                    ctx.declare_pattern(&var_decl.pattern, var_decl.kind, ValueWord::none())?;
                    Ok(
                        QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                            .with_value(ValueWord::none()),
                    )
                }
                shape_ast::ast::Item::Assignment(assignment, _) => {
                    ctx.set_pattern(&assignment.pattern, ValueWord::none())?;
                    Ok(
                        QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                            .with_value(ValueWord::none()),
                    )
                }
                shape_ast::ast::Item::TypeAlias(_, _) => {
                    self.process_program_items(
                        &Program {
                            items: vec![item.clone()],
                            docs: shape_ast::ast::ProgramDocs::default(),
                        },
                        ctx,
                        None,
                    )?;
                    Ok(
                        QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                            .with_value(ValueWord::none()),
                    )
                }
                _ => Err(ShapeError::RuntimeError {
                    message: format!("Operation requires context: {:?}", item),
                    location: None,
                }),
            }
        } else {
            let mut ctx = context::ExecutionContext::new_empty_with_registry(
                self.type_method_registry.clone(),
            );
            self.process_program_items(
                &Program {
                    items: vec![item.clone()],
                    docs: shape_ast::ast::ProgramDocs::default(),
                },
                &mut ctx,
                None,
            )?;
            persistent_ctx = Some(ctx);
            Ok(
                QueryResult::new(QueryType::Value, "".to_string(), "".to_string())
                    .with_value(ValueWord::none()),
            )
        };

        self.persistent_context = persistent_ctx;
        result
    }

    /// Format a value using Shape format definitions from stdlib
    ///
    /// Currently a placeholder until VM-based format execution is implemented.
    pub fn format_value(
        &mut self,
        _value: Value,
        type_name: &str,
        format_name: Option<&str>,
        _param_overrides: std::collections::HashMap<String, Value>,
    ) -> Result<String> {
        if let Some(name) = format_name {
            Ok(format!("<formatted {} as {}>", type_name, name))
        } else {
            Ok(format!("<formatted {}>", type_name))
        }
    }

    /// Enable or disable debug mode.
    ///
    /// When enabled, the runtime produces verbose tracing output via `tracing`
    /// and enables any debug-only code paths in the executor.
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
        if enabled {
            tracing::debug!("Shape runtime debug mode enabled");
        }
    }

    /// Query whether debug mode is active.
    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }

    /// Set the maximum wall-clock duration for a single execution.
    ///
    /// The executor can periodically check elapsed time against this limit
    /// and abort with a timeout error if exceeded.
    pub fn set_execution_timeout(&mut self, timeout: Duration) {
        self.execution_timeout = Some(timeout);
    }

    /// Query the configured execution timeout, if any.
    pub fn execution_timeout(&self) -> Option<Duration> {
        self.execution_timeout
    }

    /// Set a memory limit (in bytes) for the runtime.
    ///
    /// Allocation tracking can reference this value to decide when to refuse
    /// new allocations or trigger garbage collection.
    pub fn set_memory_limit(&mut self, limit: usize) {
        self.memory_limit = Some(limit);
    }

    /// Query the configured memory limit, if any.
    pub fn memory_limit(&self) -> Option<usize> {
        self.memory_limit
    }
}
