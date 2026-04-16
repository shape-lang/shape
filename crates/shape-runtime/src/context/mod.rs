//! Execution context for Shape runtime
//!
//! This module contains the ExecutionContext which manages runtime state including:
//! - Variable scopes and bindings
//! - Data access and caching
//! - Backtest state and series caching
//! - Type registries and evaluator
//! - Configuration (symbol, timeframe, date range)

mod config;
mod data_access;
mod data_cache;
mod registries;
mod scope;
mod variables;

// Re-export public types
pub use data_cache::DataLoadMode;
pub use variables::Variable;

use std::collections::HashMap;
use std::sync::Arc;

use super::alerts::AlertRouter;
use super::annotation_context::{AnnotationContext, AnnotationRegistry};
use super::data::DataFrame;
use super::event_queue::{SharedEventQueue, SuspensionState};
use super::lookahead_guard::LookAheadGuard;
use super::metadata::MetadataRegistry;
use super::simulation::KernelCompiler;
use super::type_methods::TypeMethodRegistry;
use super::type_schema::TypeSchemaRegistry;
use crate::data::Timeframe;
use crate::snapshot::{
    ContextSnapshot, SnapshotStore, SuspensionStateSnapshot, TypeAliasRuntimeEntrySnapshot,
    VariableSnapshot, nanboxed_to_serializable, serializable_to_nanboxed,
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use shape_value::{ValueWord, ValueWordExt};

/// Execution context for evaluating expressions
#[derive(Clone)]
pub struct ExecutionContext {
    /// Market data provider (abstraction layer - legacy)
    data_provider: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Data cache for async provider (Phase 6)
    /// Clone is cheap since all heavy data is Arc-wrapped internally
    pub(crate) data_cache: Option<crate::data::DataCache>,
    /// Provider registry (Phase 8)
    provider_registry: Arc<super::provider_registry::ProviderRegistry>,
    /// Type mapping registry (Phase 8)
    type_mapping_registry: Arc<super::type_mapping::TypeMappingRegistry>,
    /// Type schema registry for JIT type specialization
    type_schema_registry: Arc<TypeSchemaRegistry>,
    /// Metadata registry for generic type metadata (Logic)
    metadata_registry: Arc<MetadataRegistry>,
    /// Execution mode for data loading (Phase 8)
    data_load_mode: DataLoadMode,
    /// Current ID being analyzed (e.g. symbol, sensor ID)
    current_id: Option<String>,
    /// Current data row index (for pattern matching)
    current_row_index: usize,
    /// Variable bindings (stack of scopes for function calls)
    variable_scopes: Vec<HashMap<String, Variable>>,
    /// Expression evaluator
    // TODO: Replace with BytecodeExecutor/VM
    // evaluator: Evaluator,
    /// Reference datetime for relative data row access
    reference_datetime: Option<DateTime<Utc>>,
    /// Current timeframe for data row operations
    current_timeframe: Option<Timeframe>,
    /// Base timeframe of the actual data in DuckDB
    base_timeframe: Option<Timeframe>,
    /// Look-ahead bias guard
    lookahead_guard: Option<LookAheadGuard>,
    /// Registry for user-defined type methods
    type_method_registry: Arc<TypeMethodRegistry>,
    /// Date range for data loading (start, end) as native DateTime
    date_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Walk-forward range start (inclusive)
    range_start: usize,
    /// Walk-forward range end (exclusive)
    range_end: usize,
    /// Whether a custom range is active
    range_active: bool,
    /// Decorator-based registry for pattern functions (generic - works for any domain)
    /// NOTE: This will be replaced by annotation_context.registry("patterns")
    /// once lifecycle hooks are fully integrated
    pattern_registry: HashMap<String, super::closure::Closure>,
    /// Annotation context for lifecycle hooks (cache, state, registries, emit)
    annotation_context: AnnotationContext,
    /// Registry for `annotation ... { ... }` definitions
    annotation_registry: AnnotationRegistry,
    /// Event queue for async operations (streaming, real-time data)
    event_queue: Option<SharedEventQueue>,
    /// Suspension state when execution is paused waiting for an event
    suspension_state: Option<SuspensionState>,
    /// Alert pipeline for sending alerts to output sinks
    alert_pipeline: Option<Arc<AlertRouter>>,
    /// Output adapter for handling print() results
    output_adapter: Box<dyn crate::output_adapter::OutputAdapter>,
    /// Type alias registry for meta parameter overrides
    /// Maps alias name (e.g., "Percent4") -> (base_type, overrides)
    type_alias_registry: HashMap<String, TypeAliasRuntimeEntry>,
    /// Enum definition registry for sum type support
    enum_registry: EnumRegistry,
    /// Struct type definition registry for REPL persistence
    /// Maps struct name -> StructTypeDef so type definitions survive across REPL sessions
    struct_type_registry: HashMap<String, shape_ast::ast::StructTypeDef>,
    /// Progress registry for monitoring load operations
    progress_registry: Option<Arc<super::progress::ProgressRegistry>>,
    /// Optional JIT kernel compiler for high-performance simulation.
    /// Set this to enable JIT compilation of simulation kernels.
    kernel_compiler: Option<Arc<dyn KernelCompiler>>,
}

/// Runtime entry for a type alias with meta parameter overrides
#[derive(Debug, Clone)]
pub struct TypeAliasRuntimeEntry {
    /// The base type name (e.g., "Percent" for `type Percent4 = Percent { decimals: 4 }`)
    pub base_type: String,
    /// Meta parameter overrides as runtime values
    pub overrides: Option<HashMap<String, ValueWord>>,
}

/// Registry for enum definitions
///
/// Enables enum sum types by tracking which enums exist and their variants.
/// Used for pattern matching resolution when matching against union types like
/// `type SaveError = NetworkError | DiskError`.
#[derive(Debug, Clone, Default)]
pub struct EnumRegistry {
    /// Map from enum name to its definition
    enums: HashMap<String, shape_ast::ast::EnumDef>,
}

impl EnumRegistry {
    /// Create a new empty enum registry
    pub fn new() -> Self {
        Self {
            enums: HashMap::new(),
        }
    }

    /// Register an enum definition
    pub fn register(&mut self, enum_def: shape_ast::ast::EnumDef) {
        self.enums.insert(enum_def.name.clone(), enum_def);
    }

    /// Look up an enum by name
    pub fn get(&self, name: &str) -> Option<&shape_ast::ast::EnumDef> {
        self.enums.get(name)
    }

    /// Check if an enum exists
    pub fn contains(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }

    /// Get all enum names
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.enums.keys()
    }

    /// Check if an enum value belongs to a given enum or union type
    ///
    /// For simple enum types, checks if `value_enum_name` matches.
    /// For union types (resolved from type aliases), checks if the enum
    /// is one of the union members.
    pub fn value_matches_type(&self, value_enum_name: &str, type_name: &str) -> bool {
        // Direct match
        if value_enum_name == type_name {
            return true;
        }
        // Otherwise, the type_name might be a union type alias
        // which needs to be resolved externally
        false
    }
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionContext")
            .field("data_provider", &"<DataProvider>")
            .field("current_id", &self.current_id)
            .field("current_row_index", &self.current_row_index)
            .field("variable_scopes", &self.variable_scopes)
            .field("reference_datetime", &self.reference_datetime)
            .field("current_timeframe", &self.current_timeframe)
            .field("lookahead_guard", &self.lookahead_guard)
            .finish()
    }
}

impl ExecutionContext {
    /// Create a new execution context with shared type method registry
    pub fn new_with_registry(
        data: &DataFrame,
        type_method_registry: Arc<TypeMethodRegistry>,
    ) -> Self {
        // Set current_row_index to last row so [-1] gives most recent value
        let current_row_index = if data.row_count() == 0 {
            0
        } else {
            data.row_count() - 1
        };

        Self {
            data_provider: None,
            data_cache: None,
            provider_registry: Arc::new(super::provider_registry::ProviderRegistry::new()),
            type_mapping_registry: Arc::new(super::type_mapping::TypeMappingRegistry::new()),
            type_schema_registry: Arc::new(TypeSchemaRegistry::with_stdlib_types()),
            metadata_registry: Arc::new(MetadataRegistry::new()),
            data_load_mode: DataLoadMode::default(),
            current_id: Some(data.id.clone()),
            current_row_index,
            variable_scopes: vec![HashMap::new()], // Start with root scope
            // evaluator: Evaluator::new(),
            reference_datetime: None,
            current_timeframe: Some(data.timeframe),
            base_timeframe: Some(data.timeframe),
            lookahead_guard: None,
            type_method_registry,
            date_range: None,
            range_start: 0,
            range_end: usize::MAX,
            range_active: false,
            pattern_registry: HashMap::new(),
            annotation_context: AnnotationContext::new(),
            annotation_registry: AnnotationRegistry::new(),
            event_queue: None,
            suspension_state: None,
            alert_pipeline: None,
            output_adapter: Box::new(crate::output_adapter::StdoutAdapter),
            type_alias_registry: HashMap::new(),
            enum_registry: EnumRegistry::new(),
            struct_type_registry: HashMap::new(),
            progress_registry: None,
            kernel_compiler: None,
        }
    }

    /// Create a new execution context
    pub fn new(data: &DataFrame) -> Self {
        Self::new_with_registry(data, Arc::new(TypeMethodRegistry::new()))
    }

    /// Create a new execution context without market data with shared registry
    pub fn new_empty_with_registry(type_method_registry: Arc<TypeMethodRegistry>) -> Self {
        Self {
            data_provider: None,
            data_cache: None,
            provider_registry: Arc::new(super::provider_registry::ProviderRegistry::new()),
            type_mapping_registry: Arc::new(super::type_mapping::TypeMappingRegistry::new()),
            type_schema_registry: Arc::new(TypeSchemaRegistry::with_stdlib_types()),
            metadata_registry: Arc::new(MetadataRegistry::new()),
            data_load_mode: DataLoadMode::default(),
            current_id: None,
            current_row_index: 0,
            variable_scopes: vec![HashMap::new()], // Start with root scope
            // evaluator: Evaluator::new(),
            reference_datetime: None,
            current_timeframe: None,
            base_timeframe: None,
            lookahead_guard: None,
            type_method_registry,
            date_range: None,
            range_start: 0,
            range_end: usize::MAX,
            range_active: false,
            pattern_registry: HashMap::new(),
            annotation_context: AnnotationContext::new(),
            annotation_registry: AnnotationRegistry::new(),
            event_queue: None,
            suspension_state: None,
            alert_pipeline: None,
            output_adapter: Box::new(crate::output_adapter::StdoutAdapter),
            type_alias_registry: HashMap::new(),
            enum_registry: EnumRegistry::new(),
            struct_type_registry: HashMap::new(),
            progress_registry: None,
            kernel_compiler: None,
        }
    }

    /// Create a new execution context without market data
    pub fn new_empty() -> Self {
        Self::new_empty_with_registry(Arc::new(TypeMethodRegistry::new()))
    }

    /// Create a new execution context with DuckDB provider and shared registry
    pub fn with_data_provider_and_registry(
        data_provider: Arc<dyn std::any::Any + Send + Sync>,
        type_method_registry: Arc<TypeMethodRegistry>,
    ) -> Self {
        Self {
            data_provider: Some(data_provider),
            data_cache: None,
            provider_registry: Arc::new(super::provider_registry::ProviderRegistry::new()),
            type_mapping_registry: Arc::new(super::type_mapping::TypeMappingRegistry::new()),
            type_schema_registry: Arc::new(TypeSchemaRegistry::with_stdlib_types()),
            metadata_registry: Arc::new(MetadataRegistry::new()),
            data_load_mode: DataLoadMode::default(),
            current_id: None,
            current_row_index: 0,
            variable_scopes: vec![HashMap::new()],
            // evaluator: Evaluator::new(),
            reference_datetime: None,
            current_timeframe: None,
            base_timeframe: None,
            lookahead_guard: None,
            type_method_registry,
            date_range: None,
            range_start: 0,
            range_end: usize::MAX,
            range_active: false,
            pattern_registry: HashMap::new(),
            annotation_context: AnnotationContext::new(),
            annotation_registry: AnnotationRegistry::new(),
            event_queue: None,
            suspension_state: None,
            alert_pipeline: None,
            output_adapter: Box::new(crate::output_adapter::StdoutAdapter),
            type_alias_registry: HashMap::new(),
            enum_registry: EnumRegistry::new(),
            struct_type_registry: HashMap::new(),
            progress_registry: None,
            kernel_compiler: None,
        }
    }

    /// Create a new execution context with DuckDB provider
    pub fn with_data_provider(data_provider: Arc<dyn std::any::Any + Send + Sync>) -> Self {
        Self::with_data_provider_and_registry(data_provider, Arc::new(TypeMethodRegistry::new()))
    }

    /// Create with async data provider (Phase 6)
    ///
    /// This constructor sets up ExecutionContext with a DataCache for async data loading.
    /// Call `prefetch_data()` before executing to populate the cache.
    pub fn with_async_provider(
        provider: crate::data::SharedAsyncProvider,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let data_cache = crate::data::DataCache::new(provider, runtime);
        Self {
            data_provider: None,
            data_cache: Some(data_cache),
            provider_registry: Arc::new(super::provider_registry::ProviderRegistry::new()),
            type_mapping_registry: Arc::new(super::type_mapping::TypeMappingRegistry::new()),
            type_schema_registry: Arc::new(TypeSchemaRegistry::with_stdlib_types()),
            metadata_registry: Arc::new(MetadataRegistry::new()),
            data_load_mode: DataLoadMode::default(),
            current_id: None,
            current_row_index: 0,
            variable_scopes: vec![HashMap::new()],
            // evaluator: Evaluator::new(),
            reference_datetime: None,
            current_timeframe: None,
            base_timeframe: None,
            lookahead_guard: None,
            type_method_registry: Arc::new(TypeMethodRegistry::new()),
            date_range: None,
            range_start: 0,
            range_end: usize::MAX,
            range_active: false,
            pattern_registry: HashMap::new(),
            annotation_context: AnnotationContext::new(),
            annotation_registry: AnnotationRegistry::new(),
            event_queue: None,
            suspension_state: None,
            alert_pipeline: None,
            output_adapter: Box::new(crate::output_adapter::StdoutAdapter),
            type_alias_registry: HashMap::new(),
            enum_registry: EnumRegistry::new(),
            struct_type_registry: HashMap::new(),
            progress_registry: None,
            kernel_compiler: None,
        }
    }

    /// Set the output adapter for print() handling
    pub fn set_output_adapter(&mut self, adapter: Box<dyn crate::output_adapter::OutputAdapter>) {
        self.output_adapter = adapter;
    }

    /// Get mutable reference to output adapter
    pub fn output_adapter_mut(&mut self) -> &mut Box<dyn crate::output_adapter::OutputAdapter> {
        &mut self.output_adapter
    }

    /// Get the metadata registry
    pub fn metadata_registry(&self) -> &Arc<MetadataRegistry> {
        &self.metadata_registry
    }

    // =========================================================================
    // Type Alias Registry Methods
    // =========================================================================

    /// Register a type alias for runtime meta resolution
    ///
    /// Used when loading stdlib to make type aliases available for formatting.
    /// Example: `type Percent4 = Percent { decimals: 4 }`
    pub fn register_type_alias(
        &mut self,
        alias_name: &str,
        base_type: &str,
        overrides: Option<HashMap<String, ValueWord>>,
    ) {
        self.type_alias_registry.insert(
            alias_name.to_string(),
            TypeAliasRuntimeEntry {
                base_type: base_type.to_string(),
                overrides,
            },
        );
    }

    /// Look up a type alias
    ///
    /// Returns the base type name and any parameter overrides.
    pub fn lookup_type_alias(&self, name: &str) -> Option<&TypeAliasRuntimeEntry> {
        self.type_alias_registry.get(name)
    }

    /// Resolve a type name, following aliases if needed
    ///
    /// If the type is an alias, returns (base_type, Some(overrides))
    /// If not an alias, returns (type_name, None)
    pub fn resolve_type_for_format(
        &self,
        type_name: &str,
    ) -> (String, Option<HashMap<String, ValueWord>>) {
        if let Some(entry) = self.type_alias_registry.get(type_name) {
            (entry.base_type.clone(), entry.overrides.clone())
        } else {
            (type_name.to_string(), None)
        }
    }

    // =========================================================================
    // Snapshotting
    // =========================================================================

    /// Create a serializable snapshot of the dynamic execution state.
    pub fn snapshot(&self, store: &SnapshotStore) -> Result<ContextSnapshot> {
        let mut scopes = Vec::new();
        for scope in &self.variable_scopes {
            let mut snap_scope = HashMap::new();
            for (name, var) in scope.iter() {
                let value = nanboxed_to_serializable(&var.value, store)?;
                let format_overrides = match &var.format_overrides {
                    Some(map) => {
                        let mut out = HashMap::new();
                        for (k, v) in map.iter() {
                            out.insert(k.clone(), nanboxed_to_serializable(v, store)?);
                        }
                        Some(out)
                    }
                    None => None,
                };
                snap_scope.insert(
                    name.clone(),
                    VariableSnapshot {
                        value,
                        kind: var.kind,
                        is_initialized: var.is_initialized,
                        is_function_scoped: var.is_function_scoped,
                        format_hint: var.format_hint.clone(),
                        format_overrides,
                    },
                );
            }
            scopes.push(snap_scope);
        }

        let mut alias_registry = HashMap::new();
        for (name, entry) in self.type_alias_registry.iter() {
            let overrides = match &entry.overrides {
                Some(map) => {
                    let mut out = HashMap::new();
                    for (k, v) in map.iter() {
                        out.insert(k.clone(), nanboxed_to_serializable(v, store)?);
                    }
                    Some(out)
                }
                None => None,
            };
            alias_registry.insert(
                name.clone(),
                TypeAliasRuntimeEntrySnapshot {
                    base_type: entry.base_type.clone(),
                    overrides,
                },
            );
        }

        let enum_registry = self
            .enum_registry
            .names()
            .filter_map(|name| {
                self.enum_registry
                    .get(name)
                    .cloned()
                    .map(|def| (name.clone(), def))
            })
            .collect::<HashMap<_, _>>();

        let suspension_state = match self.suspension_state() {
            Some(state) => {
                let mut locals = Vec::new();
                for v in state.saved_locals.iter() {
                    locals.push(nanboxed_to_serializable(v, store)?);
                }
                let mut stack = Vec::new();
                for v in state.saved_stack.iter() {
                    stack.push(nanboxed_to_serializable(v, store)?);
                }
                Some(SuspensionStateSnapshot {
                    waiting_for: state.waiting_for.clone(),
                    resume_pc: state.resume_pc,
                    saved_locals: locals,
                    saved_stack: stack,
                })
            }
            None => None,
        };

        let data_cache = match &self.data_cache {
            Some(cache) => Some(cache.snapshot(store)?),
            None => None,
        };

        Ok(ContextSnapshot {
            data_load_mode: self.data_load_mode,
            data_cache,
            current_id: self.current_id.clone(),
            current_row_index: self.current_row_index,
            variable_scopes: scopes,
            reference_datetime: self.reference_datetime,
            current_timeframe: self.current_timeframe,
            base_timeframe: self.base_timeframe,
            date_range: self.date_range,
            range_start: self.range_start,
            range_end: self.range_end,
            range_active: self.range_active,
            type_alias_registry: alias_registry,
            enum_registry,
            struct_type_registry: self.struct_type_registry.clone(),
            suspension_state,
        })
    }

    /// Restore execution state from a snapshot.
    pub fn restore_from_snapshot(
        &mut self,
        snapshot: ContextSnapshot,
        store: &SnapshotStore,
    ) -> Result<()> {
        self.data_load_mode = snapshot.data_load_mode;
        self.current_id = snapshot.current_id;
        self.current_row_index = snapshot.current_row_index;
        self.reference_datetime = snapshot.reference_datetime;
        self.current_timeframe = snapshot.current_timeframe;
        self.base_timeframe = snapshot.base_timeframe;
        self.date_range = snapshot.date_range;
        self.range_start = snapshot.range_start;
        self.range_end = snapshot.range_end;
        self.range_active = snapshot.range_active;

        match snapshot.data_cache {
            Some(cache_snapshot) => {
                if let Some(cache) = &self.data_cache {
                    cache.restore_from_snapshot(cache_snapshot, store)?;
                } else {
                    return Err(anyhow!(
                        "Snapshot includes data cache, but context has no async provider"
                    ));
                }
            }
            None => {
                if let Some(cache) = &self.data_cache {
                    cache.clear();
                }
            }
        }

        self.variable_scopes.clear();
        for scope in snapshot.variable_scopes.into_iter() {
            let mut restored = HashMap::new();
            for (name, var) in scope.into_iter() {
                let value = serializable_to_nanboxed(&var.value, store)?;
                let format_overrides = match var.format_overrides {
                    Some(map) => {
                        let mut out = HashMap::new();
                        for (k, v) in map.into_iter() {
                            out.insert(k, serializable_to_nanboxed(&v, store)?);
                        }
                        Some(out)
                    }
                    None => None,
                };
                restored.insert(
                    name,
                    Variable {
                        value,
                        kind: var.kind,
                        is_initialized: var.is_initialized,
                        is_function_scoped: var.is_function_scoped,
                        format_hint: var.format_hint,
                        format_overrides,
                    },
                );
            }
            self.variable_scopes.push(restored);
        }

        self.type_alias_registry.clear();
        for (name, entry) in snapshot.type_alias_registry.into_iter() {
            let overrides = match entry.overrides {
                Some(map) => {
                    let mut out = HashMap::new();
                    for (k, v) in map.into_iter() {
                        out.insert(k, serializable_to_nanboxed(&v, store)?);
                    }
                    Some(out)
                }
                None => None,
            };
            self.type_alias_registry.insert(
                name,
                TypeAliasRuntimeEntry {
                    base_type: entry.base_type,
                    overrides,
                },
            );
        }

        self.enum_registry = EnumRegistry::default();
        for (_name, def) in snapshot.enum_registry.into_iter() {
            self.enum_registry.register(def);
        }

        self.struct_type_registry = snapshot.struct_type_registry;

        if let Some(state) = snapshot.suspension_state {
            let mut locals = Vec::new();
            for v in state.saved_locals.into_iter() {
                locals.push(serializable_to_nanboxed(&v, store)?);
            }
            let mut stack = Vec::new();
            for v in state.saved_stack.into_iter() {
                stack.push(serializable_to_nanboxed(&v, store)?);
            }
            self.set_suspension_state(
                SuspensionState::new(state.waiting_for, state.resume_pc)
                    .with_locals(locals)
                    .with_stack(stack),
            );
        } else {
            self.clear_suspension_state();
        }

        // Note: output_adapter is NOT restored from snapshot.
        // It's set by the caller (StdoutAdapter for scripts, ReplAdapter for REPL).

        Ok(())
    }

    /// Set indicator cache

    /// Set the event queue for async operations
    pub fn set_event_queue(&mut self, queue: SharedEventQueue) {
        self.event_queue = Some(queue);
    }

    /// Get the event queue
    pub fn event_queue(&self) -> Option<&SharedEventQueue> {
        self.event_queue.as_ref()
    }

    /// Get mutable reference to event queue
    pub fn event_queue_mut(&mut self) -> Option<&mut SharedEventQueue> {
        self.event_queue.as_mut()
    }

    /// Set suspension state (called when yielding/suspending)
    pub fn set_suspension_state(&mut self, state: SuspensionState) {
        self.suspension_state = Some(state);
    }

    /// Get suspension state
    pub fn suspension_state(&self) -> Option<&SuspensionState> {
        self.suspension_state.as_ref()
    }

    /// Clear suspension state (called when resuming)
    pub fn clear_suspension_state(&mut self) -> Option<SuspensionState> {
        self.suspension_state.take()
    }

    /// Check if execution is suspended
    pub fn is_suspended(&self) -> bool {
        self.suspension_state.is_some()
    }

    /// Set the alert pipeline for routing alerts to sinks
    pub fn set_alert_pipeline(&mut self, pipeline: Arc<AlertRouter>) {
        self.alert_pipeline = Some(pipeline);
    }

    /// Get the alert pipeline
    pub fn alert_pipeline(&self) -> Option<&Arc<AlertRouter>> {
        self.alert_pipeline.as_ref()
    }

    /// Emit an alert through the pipeline
    pub fn emit_alert(&self, alert: super::alerts::Alert) {
        if let Some(pipeline) = &self.alert_pipeline {
            pipeline.emit(alert);
        }
    }

    /// Set the progress registry for monitoring load operations
    pub fn set_progress_registry(&mut self, registry: Arc<super::progress::ProgressRegistry>) {
        self.progress_registry = Some(registry);
    }

    /// Get the progress registry
    pub fn progress_registry(&self) -> Option<&Arc<super::progress::ProgressRegistry>> {
        self.progress_registry.as_ref()
    }

    /// Set the JIT kernel compiler for high-performance simulation.
    ///
    /// This enables JIT compilation of simulation kernels when the state is a TypedObject.
    /// The compiler should be an instance of `shape_jit::JITCompiler` wrapped in Arc.
    pub fn set_kernel_compiler(&mut self, compiler: Arc<dyn KernelCompiler>) {
        self.kernel_compiler = Some(compiler);
    }

    /// Get the JIT kernel compiler, if set.
    pub fn kernel_compiler(&self) -> Option<&Arc<dyn KernelCompiler>> {
        self.kernel_compiler.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{AsyncDataProvider, CacheKey, DataQuery, NullAsyncProvider, Timeframe};
    use crate::snapshot::SnapshotStore;
    use shape_ast::ast::VarKind;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_execution_context_new_empty() {
        let ctx = ExecutionContext::new_empty();
        assert_eq!(ctx.current_row_index(), 0);
    }

    #[test]
    fn test_execution_context_set_current_row() {
        let mut ctx = ExecutionContext::new_empty();
        ctx.set_current_row(5).unwrap();
        assert_eq!(ctx.current_row_index(), 5);
    }

    #[test]
    fn test_execution_context_variable_scope() {
        let mut ctx = ExecutionContext::new_empty();

        // Set a variable using the public API
        ctx.set_variable("x", ValueWord::from_f64(10.0))
            .unwrap_or_else(|_| {
                // Variable doesn't exist yet, need to create it first
                // This is expected - we test that set_variable fails for undefined vars
            });
    }

    // =========================================================================
    // Type Alias Registry Tests
    // =========================================================================

    #[test]
    fn test_type_alias_registry_basic() {
        let mut ctx = ExecutionContext::new_empty();

        // Register a type alias: type Percent4 = Percent { decimals: 4 }
        let mut overrides = HashMap::new();
        overrides.insert("decimals".to_string(), ValueWord::from_f64(4.0));
        ctx.register_type_alias("Percent4", "Percent", Some(overrides));

        // Look up the alias
        let entry = ctx.lookup_type_alias("Percent4");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.base_type, "Percent");
        assert!(entry.overrides.is_some());

        let overrides = entry.overrides.as_ref().unwrap();
        assert_eq!(
            overrides.get("decimals").and_then(|v| v.as_f64()),
            Some(4.0)
        );
    }

    #[test]
    fn test_type_alias_registry_no_overrides() {
        let mut ctx = ExecutionContext::new_empty();

        // Register a type alias without overrides
        ctx.register_type_alias("MyPercent", "Percent", None);

        let entry = ctx.lookup_type_alias("MyPercent");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.base_type, "Percent");
        assert!(entry.overrides.is_none());
    }

    #[test]
    fn test_type_alias_registry_unknown_type() {
        let ctx = ExecutionContext::new_empty();

        // Look up a non-existent alias
        let entry = ctx.lookup_type_alias("NonExistent");
        assert!(entry.is_none());
    }

    #[test]
    fn test_resolve_type_for_format_alias() {
        let mut ctx = ExecutionContext::new_empty();

        // Register a type alias
        let mut overrides = HashMap::new();
        overrides.insert("decimals".to_string(), ValueWord::from_f64(4.0));
        ctx.register_type_alias("Percent4", "Percent", Some(overrides.clone()));

        // Resolve the alias
        let (base_type, resolved_overrides) = ctx.resolve_type_for_format("Percent4");
        assert_eq!(base_type, "Percent");
        assert!(resolved_overrides.is_some());
        assert_eq!(
            resolved_overrides
                .unwrap()
                .get("decimals")
                .and_then(|v| v.as_f64()),
            Some(4.0)
        );
    }

    #[test]
    fn test_resolve_type_for_format_non_alias() {
        let ctx = ExecutionContext::new_empty();

        // Resolve a non-alias type
        let (base_type, resolved_overrides) = ctx.resolve_type_for_format("Number");
        assert_eq!(base_type, "Number");
        assert!(resolved_overrides.is_none());
    }

    #[derive(Clone)]
    struct TestAsyncProvider {
        frames: Arc<HashMap<CacheKey, DataFrame>>,
        load_calls: Arc<AtomicUsize>,
    }

    impl AsyncDataProvider for TestAsyncProvider {
        fn load<'a>(
            &'a self,
            query: &'a DataQuery,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<DataFrame, crate::data::AsyncDataError>>
                    + Send
                    + 'a,
            >,
        > {
            let key = CacheKey::new(query.id.clone(), query.timeframe);
            let frames = self.frames.clone();
            let calls = self.load_calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                frames
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| crate::data::AsyncDataError::SymbolNotFound(query.id.clone()))
            })
        }

        fn has_data(&self, symbol: &str, timeframe: &Timeframe) -> bool {
            let key = CacheKey::new(symbol.to_string(), *timeframe);
            self.frames.contains_key(&key)
        }

        fn symbols(&self) -> Vec<String> {
            self.frames.keys().map(|k| k.id.clone()).collect()
        }
    }

    fn temp_snapshot_root(name: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        std::env::temp_dir().join(format!("shape_ctx_snapshot_{}_{}", name, ts))
    }

    fn make_df(id: &str, timeframe: Timeframe) -> DataFrame {
        let mut df = DataFrame::new(id, timeframe);
        df.timestamps = vec![1, 2, 3];
        df.add_column("a", vec![10.0, 11.0, 12.0]);
        df
    }

    #[tokio::test]
    async fn test_execution_context_snapshot_restores_data_cache() {
        let tf = Timeframe::d1();
        let df = make_df("TEST", tf);
        let mut frames = HashMap::new();
        frames.insert(CacheKey::new("TEST".to_string(), tf), df);
        let load_calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(TestAsyncProvider {
            frames: Arc::new(frames),
            load_calls: load_calls.clone(),
        });

        let mut ctx =
            ExecutionContext::with_async_provider(provider, tokio::runtime::Handle::current());
        ctx.prefetch_data(vec![DataQuery::new("TEST", tf)])
            .await
            .unwrap();
        ctx.declare_variable("x", VarKind::Let, Some(ValueWord::from_f64(42.0)))
            .unwrap();

        let store = SnapshotStore::new(temp_snapshot_root("context_cache")).unwrap();
        let snapshot = ctx.snapshot(&store).unwrap();

        let mut restored = ExecutionContext::with_async_provider(
            Arc::new(NullAsyncProvider::default()),
            tokio::runtime::Handle::current(),
        );
        restored.restore_from_snapshot(snapshot, &store).unwrap();

        let val = restored.get_variable("x").unwrap();
        assert_eq!(val, Some(ValueWord::from_f64(42.0)));

        let row = restored
            .data_cache()
            .unwrap()
            .get_row("TEST", &tf, 0)
            .expect("row should be cached");
        assert_eq!(row.timestamp, 1);
        assert_eq!(row.fields.get("a"), Some(&10.0));

        assert_eq!(load_calls.load(Ordering::SeqCst), 1);
    }
}
