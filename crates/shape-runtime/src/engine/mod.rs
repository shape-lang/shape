//! Shape Engine - Unified execution interface
//!
//! This module provides a single, unified interface for executing all Shape code,
//! replacing the multiple specialized executors with one powerful engine.

// Submodules
mod builder;
mod execution;
mod query_extraction;
mod stdlib;
mod types;

// Re-export public types
pub use crate::query_result::QueryType;
pub use builder::ShapeEngineBuilder;
use shape_value::ValueWord;
pub use types::{
    EngineBootstrapState, ExecutionMetrics, ExecutionResult, ExecutionType, Message, MessageLevel,
};

use crate::Runtime;
use crate::data::DataFrame;
use shape_ast::error::{Result, ShapeError};

#[cfg(feature = "jit")]
use std::collections::HashMap;

use crate::hashing::HashDigest;
use crate::snapshot::{ContextSnapshot, ExecutionSnapshot, SemanticSnapshot, SnapshotStore};
use serde::Serialize;
use shape_ast::Program;
use shape_wire::WireValue;

/// Trait for evaluating individual expressions and statement blocks.
///
/// This is used by StreamExecutor, WindowExecutor, and JoinExecutor
/// to evaluate expressions without needing full program compilation.
/// shape-vm implements this for BytecodeExecutor.
pub trait ExpressionEvaluator: Send + Sync {
    /// Evaluate a slice of statements and return the result.
    fn eval_statements(
        &self,
        stmts: &[shape_ast::Statement],
        ctx: &mut crate::context::ExecutionContext,
    ) -> Result<ValueWord>;

    /// Evaluate a single expression and return the result.
    fn eval_expr(
        &self,
        expr: &shape_ast::Expr,
        ctx: &mut crate::context::ExecutionContext,
    ) -> Result<ValueWord>;
}

/// Result from ProgramExecutor::execute_program
pub struct ProgramExecutorResult {
    pub wire_value: WireValue,
    pub type_info: Option<shape_wire::metadata::TypeInfo>,
    pub execution_type: ExecutionType,
    pub content_json: Option<serde_json::Value>,
    pub content_html: Option<String>,
    pub content_terminal: Option<String>,
}

/// Trait for executing Shape programs
pub trait ProgramExecutor {
    fn execute_program(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<ProgramExecutorResult>;
}

/// The unified Shape execution engine
pub struct ShapeEngine {
    /// The runtime environment
    pub runtime: Runtime,
    /// Default data for expressions/assignments
    pub default_data: DataFrame,
    /// JIT compilation cache (source hash -> compiled program)
    #[cfg(feature = "jit")]
    pub(crate) jit_cache: HashMap<u64, ()>,
    /// Current source text for error messages (set before execution)
    pub(crate) current_source: Option<String>,
    /// Optional snapshot store for resumability
    pub(crate) snapshot_store: Option<SnapshotStore>,
    /// Last snapshot ID created
    pub(crate) last_snapshot: Option<HashDigest>,
    /// Script path for snapshot metadata
    pub(crate) script_path: Option<String>,
    /// Exported symbol names (persisted across REPL commands)
    pub(crate) exported_symbols: std::collections::HashSet<String>,
}

impl ShapeEngine {
    /// Create a new Shape engine
    pub fn new() -> Result<Self> {
        let mut runtime = Runtime::new_without_stdlib();
        runtime.enable_persistent_context_without_data();

        Ok(Self {
            runtime,
            default_data: DataFrame::default(),
            #[cfg(feature = "jit")]
            jit_cache: HashMap::new(),
            current_source: None,
            snapshot_store: None,
            last_snapshot: None,
            script_path: None,
            exported_symbols: std::collections::HashSet::new(),
        })
    }

    /// Create engine with data
    pub fn with_data(data: DataFrame) -> Result<Self> {
        let mut runtime = Runtime::new_without_stdlib();
        runtime.enable_persistent_context(&data);
        Ok(Self {
            runtime,
            default_data: data,
            #[cfg(feature = "jit")]
            jit_cache: HashMap::new(),
            current_source: None,
            snapshot_store: None,
            last_snapshot: None,
            script_path: None,
            exported_symbols: std::collections::HashSet::new(),
        })
    }

    /// Create engine with async data provider (Phase 6)
    ///
    /// This constructor sets up the engine with an async data provider.
    /// Call `execute_async()` instead of `execute()` to use async prefetching.
    pub fn with_async_provider(provider: crate::data::SharedAsyncProvider) -> Result<Self> {
        let runtime_handle = tokio::runtime::Handle::try_current()
            .map_err(|_| ShapeError::RuntimeError {
                message: "No tokio runtime available. Ensure with_async_provider is called within a tokio context.".to_string(),
                location: None,
            })?;
        let mut runtime = Runtime::new_without_stdlib();

        // Create ExecutionContext with async provider
        let ctx = crate::context::ExecutionContext::with_async_provider(provider, runtime_handle);
        runtime.set_persistent_context(ctx);

        Ok(Self {
            runtime,
            default_data: DataFrame::default(),
            #[cfg(feature = "jit")]
            jit_cache: HashMap::new(),
            current_source: None,
            snapshot_store: None,
            last_snapshot: None,
            script_path: None,
            exported_symbols: std::collections::HashSet::new(),
        })
    }

    /// Initialize REPL mode
    ///
    /// Call this once after creating the engine and loading stdlib,
    /// but before executing any REPL commands. This configures output adapters
    /// for REPL-friendly display.
    pub fn init_repl(&mut self) {
        // Set REPL output adapter to preserve PrintResult spans
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_output_adapter(Box::new(crate::output_adapter::ReplAdapter));
        }
    }

    /// Capture semantic/runtime state after stdlib bootstrap.
    ///
    /// Call this on an engine that has already loaded stdlib.
    pub fn capture_bootstrap_state(&self) -> Result<EngineBootstrapState> {
        let context =
            self.runtime
                .persistent_context()
                .cloned()
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "No persistent context available for bootstrap capture".to_string(),
                    location: None,
                })?;
        Ok(EngineBootstrapState {
            semantic: SemanticSnapshot {
                exported_symbols: self.exported_symbols.clone(),
            },
            context,
        })
    }

    /// Apply a previously captured stdlib bootstrap state.
    pub fn apply_bootstrap_state(&mut self, state: &EngineBootstrapState) {
        self.exported_symbols = state.semantic.exported_symbols.clone();
        self.runtime.set_persistent_context(state.context.clone());
    }

    /// Set the script path for snapshot metadata.
    pub fn set_script_path(&mut self, path: impl Into<String>) {
        self.script_path = Some(path.into());
    }

    /// Get the current script path, if set.
    pub fn script_path(&self) -> Option<&str> {
        self.script_path.as_deref()
    }

    /// Enable snapshotting with a content-addressed store.
    pub fn enable_snapshot_store(&mut self, store: SnapshotStore) {
        self.snapshot_store = Some(store);
    }

    /// Get last snapshot ID, if any.
    pub fn last_snapshot(&self) -> Option<&HashDigest> {
        self.last_snapshot.as_ref()
    }

    /// Access the snapshot store (if configured).
    pub fn snapshot_store(&self) -> Option<&SnapshotStore> {
        self.snapshot_store.as_ref()
    }

    /// Store a serializable blob in the snapshot store and return its hash.
    pub fn store_snapshot_blob<T: Serialize>(&self, value: &T) -> Result<HashDigest> {
        let store = self
            .snapshot_store
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Snapshot store not configured".to_string(),
                location: None,
            })?;
        Ok(store.put_struct(value)?)
    }

    /// Create a snapshot of semantic/runtime state, with optional VM/bytecode hashes supplied by the executor.
    pub fn snapshot_with_hashes(
        &mut self,
        vm_hash: Option<HashDigest>,
        bytecode_hash: Option<HashDigest>,
    ) -> Result<HashDigest> {
        let store = self
            .snapshot_store
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Snapshot store not configured".to_string(),
                location: None,
            })?;

        let semantic = SemanticSnapshot {
            exported_symbols: self.exported_symbols.clone(),
        };
        let semantic_hash = store.put_struct(&semantic)?;

        let context = if let Some(ctx) = self.runtime.persistent_context() {
            ctx.snapshot(store)?
        } else {
            return Err(ShapeError::RuntimeError {
                message: "No persistent context for snapshot".to_string(),
                location: None,
            });
        };
        let context_hash = store.put_struct(&context)?;

        let snapshot = ExecutionSnapshot {
            version: crate::snapshot::SNAPSHOT_VERSION,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            semantic_hash,
            context_hash,
            vm_hash,
            bytecode_hash,
            script_path: self.script_path.clone(),
        };

        let snapshot_hash = store.put_snapshot(&snapshot)?;
        self.last_snapshot = Some(snapshot_hash.clone());
        Ok(snapshot_hash)
    }

    /// Load a snapshot and return its components (semantic/context + optional vm/bytecode hashes).
    pub fn load_snapshot(
        &self,
        snapshot_id: &HashDigest,
    ) -> Result<(
        SemanticSnapshot,
        ContextSnapshot,
        Option<HashDigest>,
        Option<HashDigest>,
    )> {
        let store = self
            .snapshot_store
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Snapshot store not configured".to_string(),
                location: None,
            })?;
        let snapshot = store.get_snapshot(snapshot_id)?;
        let semantic: SemanticSnapshot =
            store
                .get_struct(&snapshot.semantic_hash)
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("failed to deserialize SemanticSnapshot: {e}"),
                    location: None,
                })?;
        let context: ContextSnapshot =
            store
                .get_struct(&snapshot.context_hash)
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("failed to deserialize ContextSnapshot: {e}"),
                    location: None,
                })?;
        Ok((semantic, context, snapshot.vm_hash, snapshot.bytecode_hash))
    }

    /// Apply a semantic/context snapshot to the current engine.
    pub fn apply_snapshot(
        &mut self,
        semantic: SemanticSnapshot,
        context: ContextSnapshot,
    ) -> Result<()> {
        self.exported_symbols = semantic.exported_symbols;
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            let store = self
                .snapshot_store
                .as_ref()
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Snapshot store not configured".to_string(),
                    location: None,
                })?;
            ctx.restore_from_snapshot(context, store)?;
            Ok(())
        } else {
            Err(ShapeError::RuntimeError {
                message: "No persistent context for snapshot".to_string(),
                location: None,
            })
        }
    }

    /// Register extension module namespaces with the runtime.
    /// Must be called before execute() so the module loader recognizes modules like `duckdb`.
    pub fn register_extension_modules(
        &mut self,
        modules: &[crate::extensions::ParsedModuleSchema],
    ) {
        self.runtime.register_extension_module_artifacts(modules);
    }

    /// Set the current source text for error messages
    ///
    /// Call this before execute() to enable source-contextualized error messages.
    /// The source is used during bytecode compilation to populate debug info.
    pub fn set_source(&mut self, source: &str) {
        self.current_source = Some(source.to_string());
    }

    /// Get the current source text (if set)
    pub fn current_source(&self) -> Option<&str> {
        self.current_source.as_deref()
    }

    /// Register a data provider (Phase 8)
    ///
    /// Registers a named provider for runtime data access.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let adapter = Arc::new(DataFrameAdapter::new(...));
    /// engine.register_provider("data", adapter);
    /// ```
    pub fn register_provider(&mut self, name: &str, provider: crate::data::SharedAsyncProvider) {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.register_provider(name, provider);
        }
    }

    /// Set default data provider (Phase 8)
    ///
    /// Sets which provider to use for runtime data access when no provider is specified.
    pub fn set_default_provider(&mut self, name: &str) -> Result<()> {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_default_provider(name)
        } else {
            Err(ShapeError::RuntimeError {
                message: "No execution context available".to_string(),
                location: None,
            })
        }
    }

    /// Register a type mapping (Phase 8)
    ///
    /// Registers a type mapping that defines the expected DataFrame structure
    /// for a given type name. Type mappings enable validation and JIT optimization.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use shape_core::runtime::type_mapping::TypeMapping;
    ///
    /// // Register the Candle type (from stdlib)
    /// let candle_mapping = TypeMapping::new("Candle".to_string())
    ///     .add_field("timestamp", "timestamp")
    ///     .add_field("open", "open")
    ///     .add_field("high", "high")
    ///     .add_field("low", "low")
    ///     .add_field("close", "close")
    ///     .add_field("volume", "volume")
    ///     .add_required("timestamp")
    ///     .add_required("open")
    ///     .add_required("high")
    ///     .add_required("low")
    ///     .add_required("close");
    ///
    /// engine.register_type_mapping("Candle", candle_mapping);
    /// ```
    pub fn register_type_mapping(
        &mut self,
        type_name: &str,
        mapping: crate::type_mapping::TypeMapping,
    ) {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.register_type_mapping(type_name, mapping);
        }
    }

    /// Get the current runtime state (for REPL)
    pub fn get_runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Get mutable runtime (for REPL state updates)
    pub fn get_runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// Get the format hint for a variable (if any)
    ///
    /// Returns the format hint specified in the variable's type annotation.
    /// Example: `let rate: Number @ Percent = 0.05` → Some("Percent")
    pub fn get_variable_format_hint(&self, name: &str) -> Option<String> {
        self.runtime
            .persistent_context()
            .and_then(|ctx| ctx.get_variable_format_hint(name))
    }

    // ========================================================================
    // Format Execution (Shape Runtime Formats)
    // ========================================================================

    /// Format a value using Shape runtime format evaluation
    ///
    /// This uses the format definitions from stdlib (e.g., stdlib/core/formats.shape)
    /// instead of Rust fallback formatters.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to format (as f64 for numbers)
    /// * `type_name` - The Shape type name ("Number", "String", etc.)
    /// * `format_name` - Optional format name (e.g., "Percent", "Currency"). Uses default if None.
    /// * `params` - Format parameters as JSON (e.g., {"decimals": 1})
    ///
    /// # Returns
    ///
    /// Formatted string on success
    ///
    /// # Example
    ///
    /// ```ignore
    /// let formatted = engine.format_value_string(
    ///     0.1234,
    ///     "Number",
    ///     Some("Percent"),
    ///     &HashMap::new()
    /// )?;
    /// assert_eq!(formatted, "12.34%");
    /// ```
    pub fn format_value_string(
        &mut self,
        value: f64,
        type_name: &str,
        format_name: Option<&str>,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        use std::sync::Arc;

        // Resolve type aliases and merge meta parameter overrides
        let (resolved_type_name, merged_params) =
            self.resolve_type_alias_for_formatting(type_name, params)?;

        // Convert merged JSON params to runtime ValueWord values
        let param_values: std::collections::HashMap<String, ValueWord> = merged_params
            .iter()
            .map(|(k, v)| {
                let runtime_val = match v {
                    serde_json::Value::Number(n) => ValueWord::from_f64(n.as_f64().unwrap_or(0.0)),
                    serde_json::Value::String(s) => ValueWord::from_string(Arc::new(s.clone())),
                    serde_json::Value::Bool(b) => ValueWord::from_bool(*b),
                    _ => ValueWord::none(),
                };
                (k.clone(), runtime_val)
            })
            .collect();

        // Convert value to runtime ValueWord
        let runtime_value = ValueWord::from_f64(value);

        // Call format with resolved type name and merged parameters
        self.runtime.format_value(
            runtime_value,
            resolved_type_name.as_str(),
            format_name,
            param_values,
        )
    }

    /// Resolve type alias to base type and merge meta parameter overrides
    ///
    /// If type_name is an alias (e.g., "Percent4"), resolves to base type ("Percent")
    /// and merges stored parameter overrides with passed params.
    fn resolve_type_alias_for_formatting(
        &self,
        type_name: &str,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(String, std::collections::HashMap<String, serde_json::Value>)> {
        // Check if type_name is a type alias through the runtime context
        let resolved = self
            .runtime
            .persistent_context()
            .map(|ctx| ctx.resolve_type_for_format(type_name));

        if let Some((base_type, Some(overrides))) = resolved {
            if base_type != type_name {
                let mut merged = std::collections::HashMap::new();

                // First, add stored overrides from the alias (convert ValueWord to JSON)
                for (key, val) in overrides {
                    let json_val = if let Some(n) = val.as_f64() {
                        serde_json::json!(n)
                    } else if val.is_bool() {
                        serde_json::json!(val.as_bool())
                    } else {
                        // Skip non-primitive override values
                        continue;
                    };
                    merged.insert(key, json_val);
                }

                // Then, overlay with passed params (these take precedence)
                for (key, val) in params {
                    merged.insert(key.clone(), val.clone());
                }

                return Ok((base_type, merged));
            }
        }

        // Not an alias, use as-is
        Ok((type_name.to_string(), params.clone()))
    }

    // ========================================================================
    // Extension Management
    // ========================================================================

    /// Load a data source extension from a shared library
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the extension shared library (.so, .dll, .dylib)
    /// * `config` - Configuration value for the extension
    ///
    /// # Returns
    ///
    /// Information about the loaded extension
    ///
    /// # Safety
    ///
    /// Loading extensions executes arbitrary code. Only load from trusted sources.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let info = engine.load_extension(Path::new("./libshape_ext_csv.so"), &json!({}))?;
    /// println!("Loaded: {} v{}", info.name, info.version);
    /// ```
    pub fn load_extension(
        &mut self,
        path: &std::path::Path,
        config: &serde_json::Value,
    ) -> Result<crate::extensions::LoadedExtension> {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.load_extension(path, config)
        } else {
            Err(ShapeError::RuntimeError {
                message: "No execution context available for extension loading".to_string(),
                location: None,
            })
        }
    }

    /// Unload an extension by name
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name to unload
    ///
    /// # Returns
    ///
    /// true if plugin was unloaded, false if not found
    pub fn unload_extension(&mut self, name: &str) -> bool {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.unload_extension(name)
        } else {
            false
        }
    }

    /// List all loaded extension names
    pub fn list_extensions(&self) -> Vec<String> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.list_extensions()
        } else {
            Vec::new()
        }
    }

    /// Get query schema for an extension (for LSP autocomplete)
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name
    ///
    /// # Returns
    ///
    /// The query schema if extension exists
    pub fn get_extension_query_schema(
        &self,
        name: &str,
    ) -> Option<crate::extensions::ParsedQuerySchema> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.get_extension_query_schema(name)
        } else {
            None
        }
    }

    /// Get output schema for an extension (for LSP autocomplete)
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name
    ///
    /// # Returns
    ///
    /// The output schema if extension exists
    pub fn get_extension_output_schema(
        &self,
        name: &str,
    ) -> Option<crate::extensions::ParsedOutputSchema> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.get_extension_output_schema(name)
        } else {
            None
        }
    }

    /// Get an extension data source by name
    pub fn get_extension(
        &self,
        name: &str,
    ) -> Option<std::sync::Arc<crate::extensions::ExtensionDataSource>> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.get_extension(name)
        } else {
            None
        }
    }

    /// Get extension module schema by module namespace.
    pub fn get_extension_module_schema(
        &self,
        module_name: &str,
    ) -> Option<crate::extensions::ParsedModuleSchema> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.get_extension_module_schema(module_name)
        } else {
            None
        }
    }

    /// Build VM extension modules from loaded extension module capabilities.
    pub fn module_exports_from_extensions(&self) -> Vec<crate::module_exports::ModuleExports> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.module_exports_from_extensions()
        } else {
            Vec::new()
        }
    }

    /// Invoke one loaded module export via module namespace.
    pub fn invoke_extension_module_nb(
        &self,
        module_name: &str,
        function: &str,
        args: &[shape_value::ValueWord],
    ) -> Result<shape_value::ValueWord> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.invoke_extension_module_nb(module_name, function, args)
        } else {
            Err(shape_ast::error::ShapeError::RuntimeError {
                message: "No runtime context available".to_string(),
                location: None,
            })
        }
    }

    /// Invoke one loaded module export via module namespace.
    pub fn invoke_extension_module_wire(
        &self,
        module_name: &str,
        function: &str,
        args: &[shape_wire::WireValue],
    ) -> Result<shape_wire::WireValue> {
        if let Some(ctx) = self.runtime.persistent_context() {
            ctx.invoke_extension_module_wire(module_name, function, args)
        } else {
            Err(shape_ast::error::ShapeError::RuntimeError {
                message: "No runtime context available".to_string(),
                location: None,
            })
        }
    }

    // ========================================================================
    // Progress Tracking
    // ========================================================================

    /// Enable progress tracking and return the registry for subscriptions
    ///
    /// Call this before executing code that may report progress.
    /// The returned registry can be used to subscribe to progress events.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let registry = engine.enable_progress_tracking();
    /// let mut receiver = registry.subscribe();
    ///
    /// // In a separate task
    /// while let Ok(event) = receiver.recv().await {
    ///     println!("Progress: {:?}", event);
    /// }
    /// ```
    pub fn enable_progress_tracking(
        &mut self,
    ) -> std::sync::Arc<crate::progress::ProgressRegistry> {
        // ProgressRegistry::new() already returns Arc<Self>
        let registry = crate::progress::ProgressRegistry::new();
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_progress_registry(registry.clone());
        }
        registry
    }

    /// Get the current progress registry if enabled
    pub fn progress_registry(&self) -> Option<std::sync::Arc<crate::progress::ProgressRegistry>> {
        self.runtime
            .persistent_context()
            .and_then(|ctx| ctx.progress_registry())
            .cloned()
    }

    /// Check if there are pending progress events
    pub fn has_pending_progress(&self) -> bool {
        if let Some(registry) = self.progress_registry() {
            !registry.is_empty()
        } else {
            false
        }
    }

    /// Poll for progress events (non-blocking)
    ///
    /// Returns the next progress event if available, or None if queue is empty.
    pub fn poll_progress(&self) -> Option<crate::progress::ProgressEvent> {
        self.progress_registry()
            .and_then(|registry| registry.try_recv())
    }
}

impl Default for ShapeEngine {
    fn default() -> Self {
        Self::new().expect("Failed to create default Shape engine")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{ParsedModuleArtifact, ParsedModuleSchema};

    #[test]
    fn test_register_extension_modules_registers_module_loader_artifacts() {
        let mut engine = ShapeEngine::new().expect("engine should create");

        engine.register_extension_modules(&[ParsedModuleSchema {
            module_name: "duckdb".to_string(),
            functions: Vec::new(),
            artifacts: vec![ParsedModuleArtifact {
                module_path: "duckdb".to_string(),
                source: Some("pub fn connect(uri) { uri }".to_string()),
                compiled: None,
            }],
        }]);

        let mut loader = engine.runtime.configured_module_loader();
        let module = loader
            .load_module("duckdb")
            .expect("registered extension module artifact should load");
        assert!(
            module.exports.contains_key("connect"),
            "expected connect export"
        );
    }
}
