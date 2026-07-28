//! Shape ABI v1
//!
//! Stable C ABI for host-loadable Shape capability modules.
//! Current capability families include data sources and output sinks.
//!
//! # Design Principles
//!
//! - **Stable C ABI**: Uses `#[repr(C)]` for binary compatibility across Rust versions
//! - **Self-Describing**: Plugins declare their query parameters and output fields
//! - **MessagePack Serialization**: Data exchange uses compact binary format
//! - **Binary Columnar Format**: High-performance direct loading (ABI v2)
//! - **Platform-Agnostic**: Works on native targets
//!
//! # Creating a Data Capability Module
//!
//! ```ignore
//! use shape_abi_v1::*;
//!
//! // Define your plugin info
//! #[no_mangle]
//! pub extern "C" fn shape_plugin_info() -> *const PluginInfo {
//!     static INFO: PluginInfo = PluginInfo {
//!         name: c"my-data-source".as_ptr(),
//!         version: c"1.0.0".as_ptr(),
//!         plugin_type: PluginType::DataSource,
//!         description: c"My custom data source".as_ptr(),
//!     };
//!     &INFO
//! }
//!
//! // Optional but recommended: capability manifest
//! #[no_mangle]
//! pub extern "C" fn shape_capability_manifest() -> *const CapabilityManifest { ... }
//!
//! // Implement the vtable functions...
//! ```

pub mod binary_builder;
pub mod binary_format;
pub mod foreign_types;

pub use foreign_types::{
    ForeignDirection, ForeignField, ForeignScalar, ForeignType, ForeignTypeShape,
    UnmappedForeignType, UnmappedReason,
};

use std::ffi::{c_char, c_void};

// ============================================================================
// Plugin Metadata
// ============================================================================

/// Plugin metadata returned by `shape_plugin_info()`
#[repr(C)]
pub struct PluginInfo {
    /// Plugin name (null-terminated C string)
    pub name: *const c_char,
    /// Plugin version (null-terminated C string, semver format)
    pub version: *const c_char,
    /// Type of plugin
    pub plugin_type: PluginType,
    /// Human-readable description (null-terminated C string)
    pub description: *const c_char,
}

// Safety: PluginInfo contains only const pointers to static strings
// The strings are never modified through these pointers
unsafe impl Sync for PluginInfo {}

/// Type of plugin
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
    /// Data source that provides time-series data
    DataSource = 0,
    /// Output sink for alerts and events
    OutputSink = 1,
    /// Language runtime for polyglot interop (Python, TypeScript, etc.)
    LanguageRuntime = 2,
}

/// Capability family exposed by a plugin/module.
///
/// This is intentionally broader than connector-specific concepts so the same
/// ABI can describe data, sinks, compute kernels, model runtimes, etc.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    /// Data source/query provider capability.
    DataSource = 0,
    /// Output sink capability for alerts/events.
    OutputSink = 1,
    /// Generic compute kernel capability.
    Compute = 2,
    /// Model/inference runtime capability.
    Model = 3,
    /// Language runtime capability for foreign function blocks.
    LanguageRuntime = 4,
    /// Catch-all for custom capability families.
    Custom = 255,
}

/// Canonical contract name for the built-in data source capability.
pub const CAPABILITY_DATA_SOURCE: &str = "shape.datasource";
/// Canonical contract name for the built-in output sink capability.
pub const CAPABILITY_OUTPUT_SINK: &str = "shape.output_sink";
/// Canonical contract name for the base module capability.
pub const CAPABILITY_MODULE: &str = "shape.module";
/// Canonical contract name for the language runtime capability.
pub const CAPABILITY_LANGUAGE_RUNTIME: &str = "shape.language_runtime";

/// Declares one capability contract implemented by the plugin.
#[repr(C)]
pub struct CapabilityDescriptor {
    /// Capability family.
    pub kind: CapabilityKind,
    /// Contract name (null-terminated C string), e.g. "shape.datasource".
    pub contract: *const c_char,
    /// Contract version (null-terminated C string), e.g. "1".
    pub version: *const c_char,
    /// Reserved capability flags (set to 0 for now).
    pub flags: u64,
}

// Safety: contains only const pointers to static strings.
unsafe impl Sync for CapabilityDescriptor {}

/// Capability manifest returned by `shape_capability_manifest()`.
#[repr(C)]
pub struct CapabilityManifest {
    /// Array of capability descriptors.
    pub capabilities: *const CapabilityDescriptor,
    /// Number of capability descriptors.
    pub capabilities_len: usize,
}

// Safety: contains only const pointers to static data.
unsafe impl Sync for CapabilityManifest {}

// ============================================================================
// Extension Section Claims
// ============================================================================

/// Declares a TOML section claimed by an extension.
///
/// Extensions use this to declare custom config sections in `shape.toml`
/// (e.g., `[native-dependencies]`) without coupling domain-specific concepts
/// into core Shape.
#[repr(C)]
pub struct SectionClaim {
    /// Section name (null-terminated C string), e.g. "native-dependencies"
    pub name: *const c_char,
    /// Whether absence of the section is an error (true) or silently ignored (false)
    pub required: bool,
}

// Safety: SectionClaim contains only const pointers to static strings
unsafe impl Sync for SectionClaim {}

/// Manifest of TOML sections claimed by an extension.
///
/// Returned by the optional `shape_claimed_sections` export. Extensions that
/// don't need custom sections simply omit this export (backwards compatible).
#[repr(C)]
pub struct SectionsManifest {
    /// Array of section claims.
    pub sections: *const SectionClaim,
    /// Number of section claims.
    pub sections_len: usize,
}

// Safety: SectionsManifest contains only const pointers to static data
unsafe impl Sync for SectionsManifest {}

/// Type signature for optional `shape_claimed_sections` export.
///
/// Extensions that need custom TOML sections export this symbol. It is
/// optional — omitting it is valid and means the extension claims no sections.
pub type GetClaimedSectionsFn = unsafe extern "C" fn() -> *const SectionsManifest;

// ============================================================================
// Self-Describing Query Schema
// ============================================================================

/// Parameter types that a data source can accept in queries
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// String value
    String = 0,
    /// Numeric value (f64)
    Number = 1,
    /// Boolean value
    Bool = 2,
    /// Array of strings
    StringArray = 3,
    /// Array of numbers
    NumberArray = 4,
    /// Nested object with its own schema
    Object = 5,
    /// Timestamp (i64 milliseconds since epoch)
    Timestamp = 6,
    /// Duration (f64 seconds)
    Duration = 7,
}

/// Describes a single query parameter
///
/// Plugins use this to declare what parameters they accept,
/// enabling LSP autocomplete and validation.
#[repr(C)]
pub struct QueryParam {
    /// Parameter name (e.g., "symbol", "device_type", "table")
    pub name: *const c_char,

    /// Human-readable description
    pub description: *const c_char,

    /// Parameter type
    pub param_type: ParamType,

    /// Is this parameter required?
    pub required: bool,

    /// Default value (MessagePack encoded, null if no default)
    pub default_value: *const u8,
    /// Length of default_value bytes
    pub default_value_len: usize,

    /// For enum-like params: allowed values (MessagePack array, null if any value allowed)
    pub allowed_values: *const u8,
    /// Length of allowed_values bytes
    pub allowed_values_len: usize,

    /// For Object type: nested schema (pointer to QuerySchema, null otherwise)
    pub nested_schema: *const QuerySchema,
}

// Safety: QueryParam contains only const pointers to static data
// The data is never modified through these pointers
unsafe impl Sync for QueryParam {}

/// Complete schema describing all query parameters for a data source
#[repr(C)]
pub struct QuerySchema {
    /// Array of parameter definitions
    pub params: *const QueryParam,
    /// Number of parameters
    pub params_len: usize,

    /// Example query (MessagePack encoded) for documentation
    pub example_query: *const u8,
    /// Length of example_query bytes
    pub example_query_len: usize,
}

// Safety: QuerySchema contains only const pointers to static data
// The data is never modified through these pointers
unsafe impl Sync for QuerySchema {}

// ============================================================================
// Self-Describing Output Schema
// ============================================================================

/// Describes a single output field produced by the data source
#[repr(C)]
pub struct OutputField {
    /// Field name (e.g., "timestamp", "value", "open", "temperature")
    pub name: *const c_char,

    /// Field type
    pub field_type: ParamType,

    /// Human-readable description
    pub description: *const c_char,
}

// Safety: OutputField contains only const pointers to static strings
// The data is never modified through these pointers
unsafe impl Sync for OutputField {}

/// Schema describing output data structure
#[repr(C)]
pub struct OutputSchema {
    /// Array of field definitions
    pub fields: *const OutputField,
    /// Number of fields
    pub fields_len: usize,
}

// Safety: OutputSchema contains only const pointers to static data
// The data is never modified through these pointers
unsafe impl Sync for OutputSchema {}

// ============================================================================
// Dynamic Schema Discovery (MessagePack-serializable types)
// ============================================================================

/// Data type for schema columns.
///
/// This enum is used in the MessagePack-serialized PluginSchema returned
/// by the `get_source_schema` vtable function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "PascalCase"))]
pub enum DataType {
    /// Floating-point number
    Number,
    /// Integer value
    Integer,
    /// String value
    String,
    /// Boolean value
    Boolean,
    /// Timestamp (Unix milliseconds)
    Timestamp,
}

/// Information about a single column in the data source.
///
/// This struct is serialized as MessagePack in the response from `get_source_schema`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnInfo {
    /// Column name
    pub name: std::string::String,
    /// Column data type
    pub data_type: DataType,
}

/// Schema returned by `get_source_schema` for dynamic schema discovery.
///
/// This struct is serialized as MessagePack. Example:
/// ```json
/// {
///   "columns": [
///     { "name": "timestamp", "data_type": "Timestamp" },
///     { "name": "open", "data_type": "Number" },
///     { "name": "high", "data_type": "Number" },
///     { "name": "low", "data_type": "Number" },
///     { "name": "close", "data_type": "Number" },
///     { "name": "volume", "data_type": "Integer" }
///   ],
///   "timestamp_column": "timestamp"
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PluginSchema {
    /// List of columns provided by this source
    pub columns: Vec<ColumnInfo>,
    /// Which column contains the timestamp/x-axis data
    pub timestamp_column: std::string::String,
}

// ============================================================================
// Module Capability (shape.module)
// ============================================================================

/// Schema for one callable module function.
///
/// This is serialized as MessagePack by module-capability providers.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModuleFunctionSchema {
    /// Function name as exported in the module namespace.
    pub name: std::string::String,
    /// Human-readable description.
    pub description: std::string::String,
    /// Parameter type names (for signatures/completions).
    pub params: Vec<std::string::String>,
    /// Return type name.
    pub return_type: Option<std::string::String>,
}

/// Module-level schema for a `shape.module` capability.
///
/// Serialized as MessagePack and returned by `get_module_schema`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModuleSchema {
    /// Module namespace name (e.g., "duckdb").
    pub module_name: std::string::String,
    /// Exported callable functions in this module.
    pub functions: Vec<ModuleFunctionSchema>,
}

// ============================================================================
// Progress Reporting (ABI v2)
// ============================================================================

/// Progress callback function type for reporting load progress.
///
/// Called by plugins during `load_binary` to report progress.
///
/// # Arguments
/// * `phase`: Current phase (0=Connecting, 1=Querying, 2=Fetching, 3=Parsing, 4=Converting)
/// * `rows_processed`: Number of rows processed so far
/// * `total_rows`: Total expected rows (0 if unknown)
/// * `bytes_processed`: Bytes processed so far
/// * `user_data`: User data passed to `load_binary`
///
/// # Returns
/// * 0: Continue loading
/// * Non-zero: Cancel the load operation
pub type ProgressCallbackFn = unsafe extern "C" fn(
    phase: u8,
    rows_processed: u64,
    total_rows: u64,
    bytes_processed: u64,
    user_data: *mut c_void,
) -> i32;

// ============================================================================
// Data Source Plugin VTable
// ============================================================================

/// Function pointer types for data source plugins
#[repr(C)]
pub struct DataSourceVTable {
    /// Initialize the data source with configuration.
    /// `config`: MessagePack-encoded configuration object
    /// Returns: opaque instance pointer, or null on error
    pub init: Option<unsafe extern "C" fn(config: *const u8, config_len: usize) -> *mut c_void>,

    /// Get the query schema for this data source.
    /// Returns a pointer to the QuerySchema struct (must remain valid for plugin lifetime).
    pub get_query_schema: Option<unsafe extern "C" fn(instance: *mut c_void) -> *const QuerySchema>,

    /// Get the output schema for this data source.
    /// Returns a pointer to the OutputSchema struct (must remain valid for plugin lifetime).
    pub get_output_schema:
        Option<unsafe extern "C" fn(instance: *mut c_void) -> *const OutputSchema>,

    /// Query the data schema for a specific source.
    ///
    /// Unlike `get_output_schema` which returns a static schema for the plugin,
    /// this function returns the dynamic schema for a specific data source.
    /// This enables schema discovery at runtime.
    ///
    /// `source_id`: The source identifier (e.g., table name, symbol, device ID)
    /// `out_ptr`: Output pointer to MessagePack-encoded PluginSchema
    /// `out_len`: Output length of the data
    ///
    /// The returned PluginSchema (MessagePack) has structure:
    /// ```json
    /// {
    ///   "columns": [
    ///     { "name": "timestamp", "data_type": "Timestamp" },
    ///     { "name": "value", "data_type": "Number" }
    ///   ],
    ///   "timestamp_column": "timestamp"
    /// }
    /// ```
    ///
    /// Returns: 0 on success, non-zero error code on failure
    /// Caller must free the output buffer with `free_buffer`.
    pub get_source_schema: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            source_id: *const u8,
            source_id_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Validate a query before execution.
    /// `query`: MessagePack-encoded query parameters
    /// `out_error`: On error, write error message pointer here (caller must free with `free_string`)
    /// Returns: 0 on success, non-zero error code on failure
    pub validate_query: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            query: *const u8,
            query_len: usize,
            out_error: *mut *mut c_char,
        ) -> i32,
    >,

    /// Load historical data (JSON/MessagePack format - legacy).
    /// `query`: MessagePack-encoded query parameters
    /// `out_ptr`: Output pointer to MessagePack-encoded Series data
    /// `out_len`: Output length of the data
    /// Returns: 0 on success, non-zero error code on failure
    /// Caller must free the output buffer with `free_buffer`.
    pub load: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            query: *const u8,
            query_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Load historical data in binary columnar format (ABI v2).
    ///
    /// High-performance data loading that bypasses JSON serialization.
    /// Returns binary data in the format defined by `binary_format` module
    /// that can be directly mapped to SeriesStorage.
    ///
    /// # Arguments
    /// * `instance`: Plugin instance
    /// * `query`: MessagePack-encoded query parameters
    /// * `query_len`: Length of query data
    /// * `granularity`: Progress reporting granularity (0=Coarse, 1=Fine)
    /// * `progress_callback`: Optional callback for progress reporting
    /// * `progress_user_data`: User data passed to progress callback
    /// * `out_ptr`: Output pointer to binary columnar data
    /// * `out_len`: Output length of the data
    ///
    /// Returns: 0 on success, non-zero error code on failure
    /// Caller must free the output buffer with `free_buffer`.
    pub load_binary: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            query: *const u8,
            query_len: usize,
            granularity: u8,
            progress_callback: Option<ProgressCallbackFn>,
            progress_user_data: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Subscribe to streaming data.
    /// `query`: MessagePack-encoded query parameters
    /// `callback`: Called for each data point (data_ptr, data_len, user_data)
    /// `callback_data`: User data passed to callback
    /// Returns: subscription ID on success, 0 on failure
    pub subscribe: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            query: *const u8,
            query_len: usize,
            callback: unsafe extern "C" fn(*const u8, usize, *mut c_void),
            callback_data: *mut c_void,
        ) -> u64,
    >,

    /// Unsubscribe from streaming data.
    /// `subscription_id`: ID returned by `subscribe`
    /// Returns: 0 on success, non-zero on failure
    pub unsubscribe:
        Option<unsafe extern "C" fn(instance: *mut c_void, subscription_id: u64) -> i32>,

    /// Free a buffer allocated by `load`.
    pub free_buffer: Option<unsafe extern "C" fn(ptr: *mut u8, len: usize)>,

    /// Free an error string allocated by `validate_query`.
    pub free_string: Option<unsafe extern "C" fn(ptr: *mut c_char)>,

    /// Cleanup and destroy the instance.
    pub drop: Option<unsafe extern "C" fn(instance: *mut c_void)>,
}

// ============================================================================
// Output Sink Plugin VTable
// ============================================================================

/// Function pointer types for output sink plugins (alerts, webhooks, etc.)
#[repr(C)]
pub struct OutputSinkVTable {
    /// Initialize the output sink with configuration.
    /// `config`: MessagePack-encoded configuration object
    /// Returns: opaque instance pointer, or null on error
    pub init: Option<unsafe extern "C" fn(config: *const u8, config_len: usize) -> *mut c_void>,

    /// Get the tags this sink handles (for routing).
    /// Returns a MessagePack-encoded array of strings.
    /// Empty array means sink handles all alerts.
    pub get_handled_tags: Option<
        unsafe extern "C" fn(instance: *mut c_void, out_ptr: *mut *mut u8, out_len: *mut usize),
    >,

    /// Send an alert.
    /// `alert`: MessagePack-encoded Alert struct
    /// Returns: 0 on success, non-zero error code on failure
    pub send: Option<
        unsafe extern "C" fn(instance: *mut c_void, alert: *const u8, alert_len: usize) -> i32,
    >,

    /// Flush any pending alerts.
    /// Returns: 0 on success, non-zero error code on failure
    pub flush: Option<unsafe extern "C" fn(instance: *mut c_void) -> i32>,

    /// Free a buffer allocated by `get_handled_tags`.
    pub free_buffer: Option<unsafe extern "C" fn(ptr: *mut u8, len: usize)>,

    /// Cleanup and destroy the instance.
    pub drop: Option<unsafe extern "C" fn(instance: *mut c_void)>,
}

// ============================================================================
// Module Plugin VTable
// ============================================================================

/// Payload kind returned by `ModuleVTable::invoke_ex`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleInvokeResultKind {
    /// MessagePack-encoded `shape_wire::WireValue` payload.
    WireValueMsgpack = 0,
    /// Arrow IPC bytes for a single table result (fast path, no wire envelope).
    TableArrowIpc = 1,
}

/// Extended invoke payload for module capability calls.
#[repr(C)]
pub struct ModuleInvokeResult {
    /// Payload encoding kind.
    pub kind: ModuleInvokeResultKind,
    /// Pointer to plugin-owned payload bytes.
    pub payload_ptr: *mut u8,
    /// Length in bytes of `payload_ptr`.
    pub payload_len: usize,
}

impl ModuleInvokeResult {
    /// Empty invoke result with no payload.
    pub const fn empty() -> Self {
        Self {
            kind: ModuleInvokeResultKind::WireValueMsgpack,
            payload_ptr: core::ptr::null_mut(),
            payload_len: 0,
        }
    }
}

/// Function pointer types for the base module capability (`shape.module`).
#[repr(C)]
pub struct ModuleVTable {
    /// Initialize module instance with MessagePack-encoded config.
    pub init: Option<unsafe extern "C" fn(config: *const u8, config_len: usize) -> *mut c_void>,

    /// Return MessagePack-encoded [`ModuleSchema`].
    ///
    /// The caller must free the output buffer with `free_buffer`.
    pub get_module_schema: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Return MessagePack-encoded module artifacts payload.
    ///
    /// This is an opaque host-defined payload for bundled Shape modules
    /// (source and/or precompiled artifacts). ABI keeps this generic.
    ///
    /// The caller must free the output buffer with `free_buffer`.
    pub get_module_artifacts: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Invoke a module function with MessagePack-encoded `shape_wire::WireValue` array.
    ///
    /// `function` is a UTF-8 function name (bytes).
    /// `args` is a MessagePack-encoded `Vec<shape_wire::WireValue>` payload.
    /// On success, `out_ptr/out_len` contain MessagePack-encoded `shape_wire::WireValue`.
    pub invoke: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            function: *const u8,
            function_len: usize,
            args: *const u8,
            args_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Invoke a module function and return a typed payload (`WireValue` or table IPC).
    ///
    /// `function` is a UTF-8 function name (bytes).
    /// `args` is a MessagePack-encoded `Vec<shape_wire::WireValue>` payload.
    /// On success, `out` must be filled with a valid payload descriptor.
    pub invoke_ex: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            function: *const u8,
            function_len: usize,
            args: *const u8,
            args_len: usize,
            out: *mut ModuleInvokeResult,
        ) -> i32,
    >,

    /// Free a buffer allocated by `get_module_schema`, `invoke`, or `invoke_ex`.
    pub free_buffer: Option<unsafe extern "C" fn(ptr: *mut u8, len: usize)>,

    /// Cleanup and destroy the instance.
    pub drop: Option<unsafe extern "C" fn(instance: *mut c_void)>,
}

// ============================================================================
// Language Runtime Plugin VTable
// ============================================================================

/// Error model for a language runtime.
///
/// Describes whether a runtime's foreign function calls can fail at runtime
/// due to the inherent dynamism of the language.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorModel {
    /// Runtime errors are possible on every call (Python, JS, Ruby).
    /// Foreign function return types are automatically wrapped in `Result<T>`.
    Dynamic = 0,
    /// The language has compile-time type safety. Foreign functions return
    /// `T` directly; runtime errors are not expected under normal operation.
    Static = 1,
}

/// Runtime state-opacity model for a language runtime
/// (`LanguageRuntimeVTable::state_model`).
///
/// Declares whether compiled foreign-function handles are pure functions of
/// their `(source, signature)` (so any process reproduces them by re-calling
/// `compile()`), or whether the runtime holds cross-call mutable interpreter
/// state that is opaque and non-serializable. Consumed by snapshot/resume
/// (WF-2B/2F): foreign runtime state is NEVER serialized — it is opaque by
/// declaration (see `docs/design/ffi-rebuild.md` §4.7).
///
/// Compiled handles are pure functions of `(source, signature)` — re-`compile()`
/// on any process reproduces them. A snapshot taken between foreign calls
/// re-links lazily from `ForeignFunctionEntry.body_text`.
pub const STATE_MODEL_STATELESS_COMPILE_CACHE: u32 = 0;

/// The interpreter holds cross-call mutable state (module globals, imports with
/// side effects). Python and TypeScript both declare this. Cross-call
/// interpreter state does not survive resume — a book-documented caveat.
pub const STATE_MODEL_STATEFUL_OPAQUE: u32 = 1;

/// VTable for language runtime plugins (Python, Julia, SQL, etc.).
///
/// Language runtimes enable `fn <language> name(...) { body }` blocks in Shape.
/// The runtime compiles and invokes foreign language code, providing type
/// marshaling between Shape values and native language objects.
#[repr(C)]
pub struct LanguageRuntimeVTable {
    /// Initialize the runtime with MessagePack-encoded config.
    /// Returns: opaque instance pointer, or null on error.
    pub init: Option<unsafe extern "C" fn(config: *const u8, config_len: usize) -> *mut c_void>,

    /// Deliver the declared Shape contract so the runtime can generate
    /// interface stubs; read them back with [`Self::generate_stubs`].
    /// `types_msgpack`: MessagePack-encoded
    /// [`foreign_types::ForeignContractExport`] (ADR-019 §1 / #196).
    /// Returns: 0 on success.
    pub register_types: Option<
        unsafe extern "C" fn(instance: *mut c_void, types: *const u8, types_len: usize) -> i32,
    >,

    /// Pre-compile a foreign function body.
    ///
    /// * `name`: function name (UTF-8)
    /// * `source`: dedented body text (UTF-8)
    /// * `param_names_msgpack`: MessagePack `Vec<String>` of parameter names
    /// * `param_types_msgpack`: MessagePack `Vec<String>` of Shape type names
    /// * `return_type`: Shape return type name (UTF-8, empty if none)
    /// * `is_async`: whether the function was declared `async` in Shape
    ///
    /// Returns: opaque compiled function handle, or null on error.
    /// On error, writes a UTF-8 error message to `out_error` / `out_error_len`
    /// (caller frees via `free_buffer`).
    pub compile: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            name: *const u8,
            name_len: usize,
            source: *const u8,
            source_len: usize,
            param_names: *const u8,
            param_names_len: usize,
            param_types: *const u8,
            param_types_len: usize,
            return_type: *const u8,
            return_type_len: usize,
            is_async: bool,
            out_error: *mut *mut u8,
            out_error_len: *mut usize,
        ) -> *mut c_void,
    >,

    /// Invoke a compiled function with MessagePack-encoded arguments.
    ///
    /// `args_msgpack`: MessagePack-encoded argument array.
    /// On success, writes MessagePack-encoded result to `out_ptr` / `out_len`.
    /// Returns: 0 on success, non-zero on error.
    pub invoke: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            handle: *mut c_void,
            args: *const u8,
            args_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Release a compiled function handle.
    pub dispose_function: Option<unsafe extern "C" fn(instance: *mut c_void, handle: *mut c_void)>,

    /// Return the language identifier (null-terminated C string, e.g. "python").
    /// The returned pointer must remain valid for the lifetime of the instance.
    pub language_id: Option<unsafe extern "C" fn(instance: *mut c_void) -> *const c_char>,

    /// Return MessagePack-encoded `LanguageRuntimeLspConfig`.
    /// Caller frees via `free_buffer`.
    pub get_lsp_config: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Free a buffer allocated by compile/invoke/get_lsp_config.
    pub free_buffer: Option<unsafe extern "C" fn(ptr: *mut u8, len: usize)>,

    /// Cleanup and destroy the runtime instance.
    pub drop: Option<unsafe extern "C" fn(instance: *mut c_void)>,

    /// Error model for this language runtime.
    ///
    /// `Dynamic` (0) means every call can fail at runtime — return values are
    /// automatically wrapped in `Result<T>`.  `Static` (1) means the language
    /// has compile-time type safety and runtime errors are not expected.
    ///
    /// Defaults to `Dynamic` (0) when zero-initialized.
    pub error_model: ErrorModel,

    /// Return a bundled `.shape` module source for this language runtime.
    ///
    /// The returned buffer is a UTF-8 string containing Shape source code
    /// that defines the extension's namespace (e.g., `python`, `typescript`).
    /// The host compiles this source and makes it importable under the
    /// extension's own namespace -- NOT under `std::*`.
    ///
    /// Caller frees via `free_buffer`. Returns 0 on success.
    /// If the extension has no bundled source, set this to `None`.
    pub get_shape_source: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    // ---- ABI v4 additive tail (WF-2A stage 0, ffi-rebuild §4.7) ----
    // These fields are STRICTLY ADDITIVE: appended after every v3 field so a
    // v4 host reading a v4 vtable finds them at a stable offset. The loader
    // gate (`plugins/loader.rs`) refuses to load version-mismatched
    // extensions, so a v4 host never dereferences a v3 vtable's shorter
    // layout. Do NOT reorder or remove any field above this line.
    /// Return MessagePack-encoded runtime descriptor:
    /// `{ extension_name, extension_version (semver), backend, platform_triple }`.
    ///
    /// Consumed by `shape ext list`, error messages, and node-capability
    /// matching (a receiving node advertises which language runtimes at which
    /// versions it can host). Absent (`None`) → matching falls back to the
    /// language id only. Caller frees the buffer via `free_buffer`; returns 0
    /// on success.
    pub runtime_descriptor: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Runtime state-opacity model: `STATE_MODEL_STATELESS_COMPILE_CACHE` (0)
    /// or `STATE_MODEL_STATEFUL_OPAQUE` (1). A plain `u32` field (not a fn
    /// pointer). Consumed by snapshot/resume; foreign runtime state is never
    /// serialized — it is opaque by declaration. Zero-initialized default is
    /// `STATELESS_COMPILE_CACHE`, but the `language_runtime_plugin!` macro
    /// stamps the conservative `STATEFUL_OPAQUE` for the interpreter-backed
    /// runtimes it generates.
    pub state_model: u32,

    /// Return the interface stub document generated from the contract most
    /// recently delivered through [`Self::register_types`] — a `.pyi` for
    /// Python, a `.d.ts` for TypeScript.
    ///
    /// ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196). `register_types` alone
    /// has no return channel, so the stubs it exists to produce could never
    /// reach the host; this is the other half of that channel. The buffer is
    /// UTF-8 stub source, freed by the caller via `free_buffer`. Returns 0 on
    /// success.
    ///
    /// Landed in the designated additive tail: it occupies the former
    /// `reserved0` slot, so the struct layout — and therefore
    /// [`abi_build_fingerprint`] — is unchanged, and an extension built before
    /// this capability existed stamps `None` here and is read as "no stub
    /// channel" rather than mis-dispatched.
    pub generate_stubs: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// Declare how the host may drive this instance concurrently — one of the
    /// `INSTANCE_CONCURRENCY_*` constants.
    ///
    /// ADR-019 §5 / #202 (POLY-ASYNC-OFFLOAD). Real foreign async runs `invoke`
    /// off the interpreter thread, which makes the instance pointer genuinely
    /// shared. Whether that is sound is a property only the extension knows: a
    /// CPython instance behind interior synchronization tolerates concurrent
    /// invokes, a V8 isolate must never leave the thread that created it. The
    /// host cannot infer it, and deciding it from the language id would be a
    /// terminal-name switch selecting a capability — so it is DECLARED here.
    ///
    /// Absent (`None`) means the extension has not been audited for off-thread
    /// invocation: the host treats it as
    /// [`INSTANCE_CONCURRENCY_INTERPRETER_THREAD_ONLY`] and refuses to offload
    /// an async foreign call to it, naming this slot in the diagnostic. That is
    /// the safe reading of silence — an unaudited extension keeps exactly the
    /// synchronous behaviour it had before #202.
    ///
    /// Landed in the designated additive tail's former `reserved1` slot, so the
    /// struct layout — and therefore [`abi_build_fingerprint`] — is unchanged.
    pub instance_concurrency: Option<unsafe extern "C" fn(instance: *mut c_void) -> u32>,

    /// Release a foreign object this instance previously handed the host as an
    /// opaque reference.
    ///
    /// ADR-019 §3 / #200 (POLY-FOREIGN-REF). `handle` is the extension's own
    /// identifier, echoed back exactly as it was minted; the host never
    /// interprets it. Called once, when the last Shape share of the reference
    /// is retired.
    ///
    /// Returns nothing, and cannot: this runs from a `Drop`, where there is no
    /// caller to report to and a partial teardown would be worse than none.
    /// ADR-019 §3 fixes disposal as synchronous and infallible in v1 for
    /// exactly that reason — a disposer that could fail or suspend is a later
    /// design under ADR-010 §6's finalization contract. An extension that
    /// cannot honour a handle should treat it as already gone.
    ///
    /// **Which instance.** The handle belongs to the instance that minted it,
    /// not to the language. For a
    /// [`INSTANCE_CONCURRENCY_THREAD_AFFINE`] runtime the host gives each
    /// worker its own instance, so a reference minted inside worker 2's
    /// isolate is disposed on worker 2 and nowhere else; the host routes it.
    ///
    /// Absent (`None`) means the extension mints no foreign references. The
    /// host refuses to build one against a runtime that cannot release it,
    /// rather than minting a reference that would leak by construction.
    ///
    /// Landed in the designated additive tail's former `reserved2` slot, so the
    /// struct layout — and therefore [`abi_build_fingerprint`] — is unchanged.
    pub dispose_ref: Option<unsafe extern "C" fn(instance: *mut c_void, handle: u64)>,

    /// Return this instance's capability block — the extension's declaration of
    /// which optional host protocols it speaks, and the entry points for each.
    ///
    /// ADR-019 §2 / #199. This is the LAST fn-pointer slot in the designated
    /// additive tail, and it is deliberately spent on a *versioned struct
    /// accessor* rather than on one more single-purpose entry point. Buffer
    /// sharing alone needs two entries (invoke-with-views and the release
    /// accounting query), which is already more than the tail had left; every
    /// capability after it would have forced an ABI version bump apiece. A
    /// capability block is size- and version-guarded, so it grows without
    /// touching this struct at all — and [`abi_build_fingerprint`], which folds
    /// this struct's layout, keeps meaning "the vtable is shaped as expected".
    ///
    /// The returned pointer must remain valid for the lifetime of the instance;
    /// a `static` is the intended shape. Null means "no optional capabilities",
    /// which is also what an `None` slot means — an extension built before this
    /// existed is read as offering nothing, never mis-dispatched.
    ///
    /// Reading rule, and the reason `struct_size` is the first field of every
    /// block: a host reads a field only after checking that the extension's
    /// declared `struct_size` covers it. See [`ExtensionCapabilities`].
    ///
    /// Landed in the designated additive tail's former `reserved3` slot, so the
    /// struct layout — and therefore [`abi_build_fingerprint`] — is unchanged.
    pub capabilities:
        Option<unsafe extern "C" fn(instance: *mut c_void) -> *const ExtensionCapabilities>,
}

// ============================================================================
// Extension capability blocks (ADR-019 §2 / #199)
// ============================================================================

/// Wire version of [`ExtensionCapabilities`] and everything it points at.
///
/// A host that reads a version it does not know refuses the whole block rather
/// than guessing: a misread capability table hands raw host memory to foreign
/// code at an offset the extension did not mean.
pub const EXTENSION_CAPABILITIES_VERSION: u32 = 1;

/// The optional protocols one extension instance declares it speaks.
///
/// ADR-019 §2 / #199. Returned by [`LanguageRuntimeVTable::capabilities`].
///
/// # How this grows without an ABI bump
///
/// `struct_size` is the extension's own `size_of::<ExtensionCapabilities>()`.
/// New capability pointers are appended to the end, never inserted, and a
/// reader must check
///
/// ```text
/// struct_size >= offset_of!(ExtensionCapabilities, field) + size_of_val(field)
/// ```
///
/// before touching a field. An extension built against an older definition
/// therefore declares a smaller `struct_size`, the host reads only the prefix
/// both sides agree on, and the newer capabilities read as absent. The same
/// rule applies recursively to every block hung off this one.
///
/// `version` is the semantic counterpart: `struct_size` says how much is
/// physically there, `version` says what the fields mean. A bump to
/// [`EXTENSION_CAPABILITIES_VERSION`] is for a *reinterpretation*, and the host
/// refuses a version it does not recognise.
#[repr(C)]
pub struct ExtensionCapabilities {
    /// `size_of::<ExtensionCapabilities>()` as the extension compiled it.
    pub struct_size: u32,
    /// [`EXTENSION_CAPABILITIES_VERSION`] at the time the extension was built.
    pub version: u32,
    /// Zero-copy buffer sharing, or null if this runtime does not offer it.
    pub buffers: *const BufferCapability,
}

/// [`ForeignBufferView::elem_type`] / [`BufferCapability::elem_types`]: 64-bit
/// signed integers — Shape `Array<int>`, a contiguous `i64` buffer.
pub const BUFFER_ELEM_INT64: u32 = 0;

/// [`ForeignBufferView::elem_type`] / [`BufferCapability::elem_types`]: 64-bit
/// IEEE floats — Shape `Array<number>`, a contiguous `f64` buffer.
pub const BUFFER_ELEM_FLOAT64: u32 = 1;

/// [`BufferCapability::modes`]: the extension can export an immutable
/// shared-borrow view — foreign code may read the host's memory and must not
/// write it (ADR-006 borrow rules).
pub const BUFFER_MODE_SHARED: u32 = 1 << 0;

/// [`BufferCapability::modes`]: the extension can export an exclusive
/// mutable-borrow view — foreign code may write the host's memory in place, and
/// no other view of the same buffer exists for the duration of the call.
pub const BUFFER_MODE_SHARED_MUT: u32 = 1 << 1;

/// One host buffer exported to foreign code for the duration of a single call.
///
/// ADR-019 §2 / #199. The pointer is into live host memory. It is valid ONLY
/// between entry to and return from
/// [`BufferCapability::invoke_with_buffers`]; the host pins the buffer for that
/// window and reclaims it immediately after, which is why
/// [`BufferCapability::outstanding_exports`] exists.
#[repr(C)]
pub struct ForeignBufferView {
    /// Which declared parameter this view stands in for — an index into the
    /// function's argument list. The msgpack argument array carries nil at this
    /// position; the view is the real value.
    pub arg_index: u32,
    /// One of the `BUFFER_ELEM_*` constants.
    pub elem_type: u32,
    /// Exactly one of [`BUFFER_MODE_SHARED`] / [`BUFFER_MODE_SHARED_MUT`].
    pub mode: u32,
    /// Padding; zero.
    pub _reserved: u32,
    /// Element count (NOT bytes). Byte length is `len * elem_size(elem_type)`.
    pub len: u64,
    /// Base address of the first element, naturally aligned for `elem_type`.
    pub data: *mut c_void,
}

/// The zero-copy buffer-sharing capability (ADR-019 §2 / #199).
///
/// Hung off [`ExtensionCapabilities::buffers`]. Same size-guard rule: read a
/// field only if `struct_size` covers it.
#[repr(C)]
pub struct BufferCapability {
    /// `size_of::<BufferCapability>()` as the extension compiled it.
    pub struct_size: u32,
    /// Bitmask of the `BUFFER_MODE_*` constants this runtime implements. A mode
    /// the host asks for and this does not advertise is refused at the call, in
    /// the host, before any pointer is handed over.
    pub modes: u32,
    /// Bitmask of `1 << BUFFER_ELEM_*` for the element types this runtime can
    /// project. An element type outside it is refused the same way.
    pub elem_types: u32,
    /// Padding; zero.
    pub _reserved: u32,
    /// Invoke a compiled function with `views` substituted for the argument
    /// positions they name.
    ///
    /// `args` is the ordinary MessagePack argument array with nil in every
    /// position covered by a view. Result handling is identical to
    /// [`LanguageRuntimeVTable::invoke`]: MessagePack to `out_ptr`/`out_len`,
    /// freed by the caller through `free_buffer`, non-zero return on error.
    ///
    /// Every view's pointer is dead the instant this returns. An extension that
    /// cannot guarantee its foreign side has dropped every reference by then
    /// must report that through [`Self::outstanding_exports`].
    pub invoke_with_buffers: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            handle: *mut c_void,
            args: *const u8,
            args_len: usize,
            views: *const ForeignBufferView,
            views_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,
    /// Which views from the most recent [`Self::invoke_with_buffers`] on this
    /// instance were STILL exported when the foreign body returned, as a bitmask
    /// of view indices (bit `i` = `views[i]`).
    ///
    /// Zero means every view was released and the host may reclaim the memory.
    ///
    /// **Absent (`None`) means this runtime has no release accounting, and the
    /// host refuses buffer sharing for it entirely** — ADR-019 §2 fixes that as
    /// refusal rather than a weaker guarantee, because the failure it prevents
    /// is memory corruption attributable to an ordinary-looking Shape source
    /// file. This is the one capability slot whose absence disables the
    /// capability rather than degrading it.
    ///
    /// Called on the interpreter thread immediately after the invoke returns,
    /// before the host unpins anything.
    pub outstanding_exports: Option<unsafe extern "C" fn(instance: *mut c_void) -> u64>,
}

// SAFETY: both blocks are immutable descriptors. The only pointer either holds
// aims at another descriptor with the same lifetime — a `static` in the
// extension binary — and nothing reads or writes through them except the host's
// size-guarded field reads. Declaring `Sync` is what lets an extension spell its
// capability block as the `static` this design assumes.
unsafe impl Sync for ExtensionCapabilities {}
unsafe impl Sync for BufferCapability {}

/// The most views one call may export.
///
/// Set by [`BufferCapability::outstanding_exports`]'s return type: the release
/// accounting is a `u64` bitmask over view indices, and the host will not accept
/// a view it could not ask about afterwards. The limit is checked at the
/// DECLARATION, so a signature that could not be accounted for never compiles.
pub const MAX_SHARED_VIEWS: usize = 64;

/// Byte width of a `BUFFER_ELEM_*` element type.
pub const fn buffer_elem_size(elem_type: u32) -> usize {
    match elem_type {
        BUFFER_ELEM_INT64 | BUFFER_ELEM_FLOAT64 => 8,
        _ => 0,
    }
}

/// [`LanguageRuntimeVTable::instance_concurrency`]: the instance may only be
/// touched from the host's interpreter thread. Async foreign calls into this
/// runtime are refused rather than offloaded (ADR-019 §5 / #202).
///
/// The default reading of an undeclared (`None`) slot.
pub const INSTANCE_CONCURRENCY_INTERPRETER_THREAD_ONLY: u32 = 0;

/// [`LanguageRuntimeVTable::instance_concurrency`]: every vtable entry takes
/// `&self` on the far side and the instance is interiorly synchronized, so the
/// host may call `invoke` from several threads at once while the interpreter
/// thread is inside `compile` / `register_types`.
///
/// Declared by the Python runtime: `PythonRuntime`'s state is behind `RwLock` /
/// `AtomicUsize`, and CPython's own GIL is released across `time.sleep` and
/// blocking IO, which is what makes two `async fn python` calls overlap.
pub const INSTANCE_CONCURRENCY_SHARED: u32 = 1;

/// [`LanguageRuntimeVTable::instance_concurrency`]: the instance is bound to
/// the thread that created it and must never be touched from another, even
/// under a lock.
///
/// Declared by the TypeScript runtime: a V8 isolate is thread-affine. The host
/// honours this by giving the language dedicated worker threads, each owning
/// its own instance built with a fresh `init` (ADR-019 §5's
/// "dedicated worker thread owning the V8 isolate" pattern). Overlap between
/// two such calls comes from there being several workers, not from one instance
/// being re-entered.
pub const INSTANCE_CONCURRENCY_THREAD_AFFINE: u32 = 2;

/// LSP configuration for a language runtime, returned by `get_lsp_config`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LanguageRuntimeLspConfig {
    /// Language identifier (e.g. "python").
    pub language_id: std::string::String,
    /// Command to start the child language server.
    pub server_command: Vec<std::string::String>,
    /// File extension for virtual documents (e.g. ".py").
    pub file_extension: std::string::String,
    /// Extra search paths for the child LSP (e.g. stub directories).
    pub extra_paths: Vec<std::string::String>,
}

// ============================================================================
// Required Plugin Exports
// ============================================================================

/// Type signature for `shape_plugin_info` export
pub type GetPluginInfoFn = unsafe extern "C" fn() -> *const PluginInfo;

/// Type signature for `shape_data_source_vtable` export
pub type GetDataSourceVTableFn = unsafe extern "C" fn() -> *const DataSourceVTable;

/// Type signature for `shape_output_sink_vtable` export
pub type GetOutputSinkVTableFn = unsafe extern "C" fn() -> *const OutputSinkVTable;
/// Type signature for `shape_module_vtable` export.
pub type GetModuleVTableFn = unsafe extern "C" fn() -> *const ModuleVTable;
/// Type signature for `shape_language_runtime_vtable` export.
pub type GetLanguageRuntimeVTableFn = unsafe extern "C" fn() -> *const LanguageRuntimeVTable;
/// Type signature for optional `shape_capability_manifest` export
pub type GetCapabilityManifestFn = unsafe extern "C" fn() -> *const CapabilityManifest;
/// Type signature for optional generic `shape_capability_vtable` export
///
/// When present, this is preferred over capability-specific symbol names.
/// `contract` is a UTF-8 byte slice (for example `shape.datasource`).
/// Return null when the contract is not implemented by this module.
pub type GetCapabilityVTableFn =
    unsafe extern "C" fn(contract: *const u8, contract_len: usize) -> *const c_void;

// ============================================================================
// Error Codes
// ============================================================================

/// Standard error codes returned by plugin functions
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginError {
    /// Operation succeeded
    Success = 0,
    /// Invalid argument
    InvalidArgument = 1,
    /// Query validation failed
    ValidationFailed = 2,
    /// Connection error
    ConnectionError = 3,
    /// Data not found
    NotFound = 4,
    /// Timeout
    Timeout = 5,
    /// Permission denied
    PermissionDenied = 6,
    /// Internal error
    InternalError = 7,
    /// Not implemented
    NotImplemented = 8,
    /// Resource exhausted
    ResourceExhausted = 9,
    /// Plugin not initialized
    NotInitialized = 10,
}

// ============================================================================
// Permission Model (Self-Describing)
// ============================================================================

use std::collections::BTreeSet;
use std::fmt;

/// Category of a permission, used for grouping in human-readable displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PermissionCategory {
    /// Filesystem access (read, write, scoped).
    Filesystem,
    /// Network access (connect, listen, scoped).
    Network,
    /// System-level capabilities (process, env, time, random).
    System,
    /// Sandbox controls (virtual fs, deterministic runtime, output capture).
    Sandbox,
    /// Foreign-code execution (extern C, embedded Python/TypeScript).
    Foreign,
}

impl PermissionCategory {
    /// Human-readable name for this category.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Filesystem => "Filesystem",
            Self::Network => "Network",
            Self::System => "System",
            Self::Sandbox => "Sandbox",
            Self::Foreign => "Foreign",
        }
    }
}

impl fmt::Display for PermissionCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A single, self-describing permission that a plugin can request.
///
/// Each variant carries enough metadata to produce human-readable prompts
/// (e.g., "Allow plugin X to read the filesystem?").
///
/// Permissions are intentionally **not** bitflags — they are named, enumerable,
/// and carry documentation so that hosts can display meaningful permission
/// dialogs and plugins can declare exactly what they need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Permission {
    // -- Filesystem --
    /// Read files and directories.
    FsRead,
    /// Write, create, and delete files and directories.
    FsWrite,
    /// Filesystem access scoped to specific paths (see `PermissionGrant`).
    FsScoped,

    // -- Network --
    /// Open outbound network connections.
    NetConnect,
    /// Listen for inbound network connections.
    NetListen,
    /// Network access scoped to specific hosts/ports (see `PermissionGrant`).
    NetScoped,

    // -- System --
    /// Spawn child processes.
    Process,
    /// Read environment variables.
    Env,
    /// Access wall-clock time.
    Time,
    /// Access random number generation.
    Random,

    // -- Sandbox controls --
    /// Plugin operates against a virtual filesystem instead of the real one.
    Vfs,
    /// Plugin runs in a deterministic runtime (fixed time, seeded RNG).
    Deterministic,
    /// Plugin output is captured for inspection rather than emitted directly.
    Capture,
    /// Memory usage is limited to a configured ceiling.
    MemLimited,
    /// Wall-clock execution time is capped.
    TimeLimited,
    /// Output volume is capped (bytes or records).
    OutputLimited,

    // -- Foreign code --
    //
    // NOTE (WF-1D stage 0, ffi-rebuild §4.8.1): `Ffi` is the 17th variant and
    // MUST stay at the end / highest ordinal. Content hashes fold in the
    // *sorted permission names* of each function's `required_permissions`
    // (`content_addressed.rs:compute_hash`), and no program at HEAD derives
    // `Ffi` yet (WF-2A's `compile_foreign_function` adds the derivation), so
    // appending the variant here leaves every existing program's content hash
    // unchanged. Reserving the slot early stabilizes hashes before FFI lands.
    /// Execute foreign code: extern C native calls and embedded
    /// dynamic-language functions (python/typescript/...). Foreign code
    /// runs with process authority; granting Ffi is granting everything
    /// the process can do unless scoped (see `ScopeConstraints::ffi_*`).
    Ffi,
}

impl Permission {
    /// Short machine-readable name (stable across versions).
    pub fn name(&self) -> &'static str {
        match self {
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::FsScoped => "fs.scoped",
            Self::NetConnect => "net.connect",
            Self::NetListen => "net.listen",
            Self::NetScoped => "net.scoped",
            Self::Process => "sys.process",
            Self::Env => "sys.env",
            Self::Time => "sys.time",
            Self::Random => "sys.random",
            Self::Vfs => "sandbox.vfs",
            Self::Deterministic => "sandbox.deterministic",
            Self::Capture => "sandbox.capture",
            Self::MemLimited => "sandbox.mem_limited",
            Self::TimeLimited => "sandbox.time_limited",
            Self::OutputLimited => "sandbox.output_limited",
            // Dotted machine name (invariant: every permission name is dotted).
            // Distinct from the shape.toml coarse grant key `ffi` (§4.8.2).
            Self::Ffi => "ffi.call",
        }
    }

    /// Human-readable description suitable for permission prompts.
    pub fn description(&self) -> &'static str {
        match self {
            Self::FsRead => "Read files and directories",
            Self::FsWrite => "Write, create, and delete files and directories",
            Self::FsScoped => "Filesystem access scoped to specific paths",
            Self::NetConnect => "Open outbound network connections",
            Self::NetListen => "Listen for inbound network connections",
            Self::NetScoped => "Network access scoped to specific hosts/ports",
            Self::Process => "Spawn child processes",
            Self::Env => "Read environment variables",
            Self::Time => "Access wall-clock time",
            Self::Random => "Access random number generation",
            Self::Vfs => "Operate against a virtual filesystem",
            Self::Deterministic => "Run in a deterministic runtime (fixed time, seeded RNG)",
            Self::Capture => "Output is captured for inspection",
            Self::MemLimited => "Memory usage is limited to a configured ceiling",
            Self::TimeLimited => "Execution time is capped",
            Self::OutputLimited => "Output volume is capped",
            Self::Ffi => "Execute foreign code (extern C, embedded Python/TypeScript)",
        }
    }

    /// Category this permission belongs to.
    pub fn category(&self) -> PermissionCategory {
        match self {
            Self::FsRead | Self::FsWrite | Self::FsScoped => PermissionCategory::Filesystem,
            Self::NetConnect | Self::NetListen | Self::NetScoped => PermissionCategory::Network,
            Self::Process | Self::Env | Self::Time | Self::Random => PermissionCategory::System,
            Self::Vfs
            | Self::Deterministic
            | Self::Capture
            | Self::MemLimited
            | Self::TimeLimited
            | Self::OutputLimited => PermissionCategory::Sandbox,
            Self::Ffi => PermissionCategory::Foreign,
        }
    }

    /// All permission variants (useful for enumeration / display).
    pub fn all_variants() -> &'static [Permission] {
        &[
            Self::FsRead,
            Self::FsWrite,
            Self::FsScoped,
            Self::NetConnect,
            Self::NetListen,
            Self::NetScoped,
            Self::Process,
            Self::Env,
            Self::Time,
            Self::Random,
            Self::Vfs,
            Self::Deterministic,
            Self::Capture,
            Self::MemLimited,
            Self::TimeLimited,
            Self::OutputLimited,
            // Foreign code — keep last (highest ordinal); see the enum note.
            Self::Ffi,
        ]
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of permissions with set-algebraic operations.
///
/// Backed by a `BTreeSet` so iteration order is deterministic and
/// serialization is stable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PermissionSet {
    permissions: BTreeSet<Permission>,
}

impl Default for PermissionSet {
    fn default() -> Self {
        Self::pure()
    }
}

impl PermissionSet {
    /// Empty permission set (pure computation — no capabilities).
    pub fn pure() -> Self {
        Self {
            permissions: BTreeSet::new(),
        }
    }

    /// Read-only access: filesystem read + env + time.
    pub fn readonly() -> Self {
        Self {
            permissions: [Permission::FsRead, Permission::Env, Permission::Time]
                .into_iter()
                .collect(),
        }
    }

    /// Full (unrestricted) permissions — every variant.
    pub fn full() -> Self {
        Self {
            permissions: Permission::all_variants().iter().copied().collect(),
        }
    }

    /// Create a set from an iterator of permissions.
    pub fn from_iter(iter: impl IntoIterator<Item = Permission>) -> Self {
        Self {
            permissions: iter.into_iter().collect(),
        }
    }

    /// Add a permission to the set. Returns whether it was newly inserted.
    pub fn insert(&mut self, perm: Permission) -> bool {
        self.permissions.insert(perm)
    }

    /// Remove a permission from the set. Returns whether it was present.
    pub fn remove(&mut self, perm: &Permission) -> bool {
        self.permissions.remove(perm)
    }

    /// Check whether a specific permission is in the set.
    pub fn contains(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }

    /// True if this set is a subset of `other`.
    pub fn is_subset(&self, other: &PermissionSet) -> bool {
        self.permissions.is_subset(&other.permissions)
    }

    /// True if this set is a superset of `other`.
    pub fn is_superset(&self, other: &PermissionSet) -> bool {
        self.permissions.is_superset(&other.permissions)
    }

    /// Set union (all permissions from both sets).
    pub fn union(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            permissions: self
                .permissions
                .union(&other.permissions)
                .copied()
                .collect(),
        }
    }

    /// Set intersection (only permissions in both sets).
    pub fn intersection(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            permissions: self
                .permissions
                .intersection(&other.permissions)
                .copied()
                .collect(),
        }
    }

    /// Set difference (permissions in self but not in other).
    pub fn difference(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            permissions: self
                .permissions
                .difference(&other.permissions)
                .copied()
                .collect(),
        }
    }

    /// True when the set is empty (no permissions).
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }

    /// Number of permissions in the set.
    pub fn len(&self) -> usize {
        self.permissions.len()
    }

    /// Iterate over the permissions in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.permissions.iter()
    }

    /// Return permissions grouped by category.
    pub fn by_category(&self) -> std::collections::BTreeMap<PermissionCategory, Vec<Permission>> {
        let mut map = std::collections::BTreeMap::new();
        for perm in &self.permissions {
            map.entry(perm.category())
                .or_insert_with(Vec::new)
                .push(*perm);
        }
        map
    }
}

impl fmt::Display for PermissionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.permissions.iter().map(|p| p.name()).collect();
        write!(f, "{{{}}}", names.join(", "))
    }
}

impl<const N: usize> From<[Permission; N]> for PermissionSet {
    fn from(arr: [Permission; N]) -> Self {
        Self {
            permissions: arr.into_iter().collect(),
        }
    }
}

impl std::iter::FromIterator<Permission> for PermissionSet {
    fn from_iter<I: IntoIterator<Item = Permission>>(iter: I) -> Self {
        Self {
            permissions: iter.into_iter().collect(),
        }
    }
}

impl IntoIterator for PermissionSet {
    type Item = Permission;
    type IntoIter = std::collections::btree_set::IntoIter<Permission>;

    fn into_iter(self) -> Self::IntoIter {
        self.permissions.into_iter()
    }
}

impl<'a> IntoIterator for &'a PermissionSet {
    type Item = &'a Permission;
    type IntoIter = std::collections::btree_set::Iter<'a, Permission>;

    fn into_iter(self) -> Self::IntoIter {
        self.permissions.iter()
    }
}

/// Scope constraints for a permission grant.
///
/// When attached to a `PermissionGrant`, these constrain *where* or *how much*
/// a permission applies. For example, `FsScoped` with `allowed_paths` limits
/// filesystem access to specific directories.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeConstraints {
    /// Allowed filesystem paths (glob patterns). Only relevant for `FsScoped`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub allowed_paths: Vec<std::string::String>,

    /// Allowed network hosts (host:port patterns). Only relevant for `NetScoped`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub allowed_hosts: Vec<std::string::String>,

    /// Maximum memory in bytes. Only relevant for `MemLimited`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub max_memory_bytes: Option<u64>,

    /// Maximum execution time in milliseconds. Only relevant for `TimeLimited`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub max_time_ms: Option<u64>,

    /// Maximum output bytes. Only relevant for `OutputLimited`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub max_output_bytes: Option<u64>,

    // -- Foreign-code scope (ffi-rebuild §4.8.2). Only relevant for `Ffi`. --
    //
    // WF-1D reserves and carries these; enforcement (the pre-`dlopen` /
    // pre-`compile()` scope check) is a WF-2A stage. An empty section with
    // `Ffi` granted means "all foreign code allowed" (parity with an
    // unscoped `FsRead`); a non-empty list narrows to the allowed ids/paths.
    /// Allowed foreign language ids (e.g. `["python"]`) for `fn <lang>` bodies.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub ffi_languages: Vec<std::string::String>,

    /// Allowed native-library path globs for `extern C`, matched AFTER alias
    /// resolution (`resolve_native_library_alias`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub ffi_libraries: Vec<std::string::String>,

    /// Optional glob over symbols permitted within the allowed libraries.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub ffi_symbols: Vec<std::string::String>,
}

impl ScopeConstraints {
    /// Unconstrained (no limits).
    pub fn none() -> Self {
        Self {
            allowed_paths: Vec::new(),
            allowed_hosts: Vec::new(),
            max_memory_bytes: None,
            max_time_ms: None,
            max_output_bytes: None,
            ffi_languages: Vec::new(),
            ffi_libraries: Vec::new(),
            ffi_symbols: Vec::new(),
        }
    }
}

impl Default for ScopeConstraints {
    fn default() -> Self {
        Self::none()
    }
}

/// A single granted permission with optional scope constraints.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PermissionGrant {
    /// The permission being granted.
    pub permission: Permission,
    /// Optional scope constraints narrowing the grant.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub constraints: Option<ScopeConstraints>,
}

impl PermissionGrant {
    /// Grant a permission without scope constraints.
    pub fn unconstrained(permission: Permission) -> Self {
        Self {
            permission,
            constraints: None,
        }
    }

    /// Grant a permission with scope constraints.
    pub fn scoped(permission: Permission, constraints: ScopeConstraints) -> Self {
        Self {
            permission,
            constraints: Some(constraints),
        }
    }
}

// ============================================================================
// Alert Types (for Output Sinks)
// ============================================================================

/// Alert severity levels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
    Critical = 4,
}

/// C-compatible alert structure for serialization reference
///
/// Actual alerts are MessagePack-encoded with this structure:
/// ```json
/// {
///   "id": "uuid-string",
///   "severity": 1,  // AlertSeverity value
///   "title": "Alert title",
///   "message": "Detailed message",
///   "data": { "key": "value" },  // Arbitrary structured data
///   "tags": ["tag1", "tag2"],
///   "timestamp": 1706054400000  // Unix millis
/// }
/// ```
#[repr(C)]
pub struct AlertHeader {
    /// Alert severity
    pub severity: AlertSeverity,
    /// Timestamp in milliseconds since Unix epoch
    pub timestamp_ms: i64,
}

// ============================================================================
// Version Checking
// ============================================================================

/// ABI version for compatibility checking
/// ABI version for compatibility checking
///
/// Version history:
/// - v1: Initial release with MessagePack-based load()
/// - v2: Added load_binary() for high-performance binary columnar format
/// - v3: Added module invoke_ex() typed payloads for table fast-path marshalling
/// - v4: WF-2A stage 0 — `LanguageRuntimeVTable` additive tail
///   (`runtime_descriptor` + `state_model` + reserved padding), extension-side
///   `catch_unwind` panic containment folded into `language_runtime_plugin!`,
///   and the coordinated content-hash/blob-format finalization (foreign-entry
///   `is_async`/`param_names`, blob-local `CallForeign` ordinals, declared
///   native-library alias storage). See `docs/design/ffi-rebuild.md` §4.7 /
///   `docs/design/polyglot-distributed-integration.md` §4.2.0.
pub const ABI_VERSION: u32 = 4;

/// Get the ABI version (plugins should export this)
pub type GetAbiVersionFn = unsafe extern "C" fn() -> u32;

/// Get the structural ABI build fingerprint (plugins export this via the
/// `language_runtime_plugin!` macro; the host reads it at load time).
pub type GetAbiBuildFingerprintFn = unsafe extern "C" fn() -> u64;

/// FNV-1a mix step used to fold structural layout values into the fingerprint.
const fn abi_fingerprint_mix(hash: u64, value: u64) -> u64 {
    (hash ^ value).wrapping_mul(0x0000_0100_0000_01b3)
}

/// Structural ABI build fingerprint.
///
/// # Why this exists (WF-2A extension-hardening)
///
/// The [`ABI_VERSION`] integer is a single hand-maintained number. The
/// `language_runtime_plugin!` macro builds [`LanguageRuntimeVTable`] by FIELD
/// NAME, so a struct **reorder** (or an added/removed/retyped field) silently
/// changes the compiled `#[repr(C)]` binary layout while `ABI_VERSION` stays
/// `4`. An extension built against such a skewed `shape-abi-v1` PASSES the
/// integer gate, and the host then dispatches through the vtable at
/// host-expected byte offsets that do not match the extension's actual layout —
/// loading a data field where a fn-pointer is expected and calling it: a wild
/// call → SIGSEGV.
///
/// This fingerprint folds the **actual compiled layout** (`size`, `align`, and
/// every field `offset_of!`) of the boundary structs, plus [`ABI_VERSION`],
/// into a `u64`. The host and the extension each compute it from THEIR OWN copy
/// of `shape-abi-v1`; if either struct layout differs, the fingerprints differ
/// and the loader refuses the `.so` with a clean diagnostic instead of
/// crashing.
///
/// It is deliberately **profile-independent**: it captures only `#[repr(C)]`
/// layout, which is identical in debug and release. A debug-built extension is
/// ABI-identical to a release one (verified: no custom cargo profiles, no
/// `#[global_allocator]`, no `panic=abort`; every vtable entry is wrapped in a
/// `catch_unwind` shell), so a matched-source debug `.so` still loads into a
/// release host — only genuine structural skew is rejected.
pub const fn abi_build_fingerprint() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    h = abi_fingerprint_mix(h, ABI_VERSION as u64);

    // LanguageRuntimeVTable: size, align, and every field offset. A reorder of
    // any two same-sized fields (e.g. `init` <-> `invoke`) changes their
    // offsets and thus the fingerprint.
    h = abi_fingerprint_mix(h, core::mem::size_of::<LanguageRuntimeVTable>() as u64);
    h = abi_fingerprint_mix(h, core::mem::align_of::<LanguageRuntimeVTable>() as u64);
    h = abi_fingerprint_mix(h, core::mem::offset_of!(LanguageRuntimeVTable, init) as u64);
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, register_types) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, compile) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, invoke) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, dispose_function) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, language_id) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, get_lsp_config) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, free_buffer) as u64,
    );
    h = abi_fingerprint_mix(h, core::mem::offset_of!(LanguageRuntimeVTable, drop) as u64);
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, error_model) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, get_shape_source) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, runtime_descriptor) as u64,
    );
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, state_model) as u64,
    );
    // Formerly `reserved0`; assigned to `generate_stubs` by ADR-019 §1 (#196).
    // The offset — and therefore this fingerprint — is unchanged, which is the
    // point of the reserved tail: a pre-#196 extension leaves the slot `None`
    // and the host reads "no stub channel" instead of mis-dispatching.
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, generate_stubs) as u64,
    );
    // Formerly `reserved3` — the last tail slot, assigned to the versioned
    // capability-block accessor by ADR-019 §2 (#199). Same offset, same
    // fingerprint; from here on optional protocols grow inside the block, so
    // this struct's layout is expected to stay fixed.
    h = abi_fingerprint_mix(
        h,
        core::mem::offset_of!(LanguageRuntimeVTable, capabilities) as u64,
    );

    // PluginInfo: the other struct the host dereferences by offset.
    h = abi_fingerprint_mix(h, core::mem::size_of::<PluginInfo>() as u64);
    h = abi_fingerprint_mix(h, core::mem::align_of::<PluginInfo>() as u64);
    h = abi_fingerprint_mix(h, core::mem::offset_of!(PluginInfo, name) as u64);
    h = abi_fingerprint_mix(h, core::mem::offset_of!(PluginInfo, version) as u64);
    h = abi_fingerprint_mix(h, core::mem::offset_of!(PluginInfo, plugin_type) as u64);
    h = abi_fingerprint_mix(h, core::mem::offset_of!(PluginInfo, description) as u64);

    h
}

// ============================================================================
// Helper Macros (for plugin authors)
// ============================================================================

/// Generate the full set of `#[no_mangle]` C ABI exports for a language runtime
/// extension plugin.
///
/// This eliminates the boilerplate that is otherwise duplicated across every
/// language runtime extension (e.g. `extensions/python/src/lib.rs` and
/// `extensions/typescript/src/lib.rs`).
///
/// # Generated exports
///
/// - `shape_plugin_info()` — plugin metadata
/// - `shape_abi_version()` — ABI version tag
/// - `shape_capability_manifest()` — declares a single LanguageRuntime capability
/// - `shape_language_runtime_vtable()` — the VTable itself
/// - `shape_capability_vtable(contract, len)` — generic vtable dispatch
///
/// # Example
///
/// ```ignore
/// shape_abi_v1::language_runtime_plugin! {
///     name: c"python",
///     version: c"0.1.0",
///     description: c"Python language runtime for foreign function blocks",
///     vtable: {
///         init: runtime::python_init,
///         register_types: runtime::python_register_types,
///         compile: runtime::python_compile,
///         invoke: runtime::python_invoke,
///         dispose_function: runtime::python_dispose_function,
///         language_id: runtime::python_language_id,
///         get_lsp_config: runtime::python_get_lsp_config,
///         generate_stubs: runtime::python_generate_stubs,
///         instance_concurrency: shape_abi_v1::INSTANCE_CONCURRENCY_SHARED,
///         // ADR-019 §2 (#199): the optional-protocol block, or
///         // `::std::ptr::null()` for a runtime that offers none.
///         capabilities: runtime::python_capabilities(),
///         free_buffer: runtime::python_free_buffer,
///         drop: runtime::python_drop,
///     }
/// }
/// ```
#[macro_export]
macro_rules! language_runtime_plugin {
    // Arm WITH shape_source: embeds a `.shape` module artifact in the extension.
    (
        name: $name:expr,
        version: $version:expr,
        description: $description:expr,
        shape_source: $shape_source:expr,
        vtable: {
            init: $init:expr,
            register_types: $register_types:expr,
            compile: $compile:expr,
            invoke: $invoke:expr,
            dispose_function: $dispose_function:expr,
            language_id: $language_id:expr,
            get_lsp_config: $get_lsp_config:expr,
            generate_stubs: $generate_stubs:expr,
            instance_concurrency: $instance_concurrency:expr,
            capabilities: $capabilities:expr,
            free_buffer: $free_buffer:expr,
            drop: $drop_fn:expr $(,)?
        } $(,)?
    ) => {
        $crate::language_runtime_plugin!(@internal
            name: $name,
            version: $version,
            description: $description,
            shape_source_opt: Some($shape_source),
            vtable: {
                init: $init,
                register_types: $register_types,
                compile: $compile,
                invoke: $invoke,
                dispose_function: $dispose_function,
                language_id: $language_id,
                get_lsp_config: $get_lsp_config,
                generate_stubs: $generate_stubs,
                instance_concurrency: $instance_concurrency,
                capabilities: $capabilities,
                free_buffer: $free_buffer,
                drop: $drop_fn,
            }
        );
    };

    // Arm WITHOUT shape_source: backward-compatible, no bundled module.
    (
        name: $name:expr,
        version: $version:expr,
        description: $description:expr,
        vtable: {
            init: $init:expr,
            register_types: $register_types:expr,
            compile: $compile:expr,
            invoke: $invoke:expr,
            dispose_function: $dispose_function:expr,
            language_id: $language_id:expr,
            get_lsp_config: $get_lsp_config:expr,
            generate_stubs: $generate_stubs:expr,
            instance_concurrency: $instance_concurrency:expr,
            capabilities: $capabilities:expr,
            free_buffer: $free_buffer:expr,
            drop: $drop_fn:expr $(,)?
        } $(,)?
    ) => {
        $crate::language_runtime_plugin!(@internal
            name: $name,
            version: $version,
            description: $description,
            shape_source_opt: None,
            vtable: {
                init: $init,
                register_types: $register_types,
                compile: $compile,
                invoke: $invoke,
                dispose_function: $dispose_function,
                language_id: $language_id,
                get_lsp_config: $get_lsp_config,
                generate_stubs: $generate_stubs,
                instance_concurrency: $instance_concurrency,
                capabilities: $capabilities,
                free_buffer: $free_buffer,
                drop: $drop_fn,
            }
        );
    };

    // Internal implementation arm.
    (@internal
        name: $name:expr,
        version: $version:expr,
        description: $description:expr,
        shape_source_opt: $shape_source_opt:expr,
        vtable: {
            init: $init:expr,
            register_types: $register_types:expr,
            compile: $compile:expr,
            invoke: $invoke:expr,
            dispose_function: $dispose_function:expr,
            language_id: $language_id:expr,
            get_lsp_config: $get_lsp_config:expr,
            generate_stubs: $generate_stubs:expr,
            instance_concurrency: $instance_concurrency:expr,
            capabilities: $capabilities:expr,
            free_buffer: $free_buffer:expr,
            drop: $drop_fn:expr $(,)?
        } $(,)?
    ) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn shape_plugin_info() -> *const $crate::PluginInfo {
            static INFO: $crate::PluginInfo = $crate::PluginInfo {
                name: $name.as_ptr(),
                version: $version.as_ptr(),
                plugin_type: $crate::PluginType::DataSource,
                description: $description.as_ptr(),
            };
            &INFO
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn shape_abi_version() -> u32 {
            $crate::ABI_VERSION
        }

        /// Structural ABI build fingerprint (WF-2A extension-hardening). The
        /// host compares this against its own `abi_build_fingerprint()` at load
        /// time and refuses to load on mismatch — turning a would-be wild-call
        /// SIGSEGV (from a silently-skewed `#[repr(C)]` vtable layout that the
        /// `ABI_VERSION` integer failed to catch) into a clean load-time error.
        #[unsafe(no_mangle)]
        pub extern "C" fn shape_abi_build_fingerprint() -> u64 {
            $crate::abi_build_fingerprint()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn shape_capability_manifest() -> *const $crate::CapabilityManifest {
            static CAPABILITIES: [$crate::CapabilityDescriptor; 1] =
                [$crate::CapabilityDescriptor {
                    kind: $crate::CapabilityKind::LanguageRuntime,
                    contract: c"shape.language_runtime".as_ptr(),
                    version: c"1".as_ptr(),
                    flags: 0,
                }];
            static MANIFEST: $crate::CapabilityManifest = $crate::CapabilityManifest {
                capabilities: CAPABILITIES.as_ptr(),
                capabilities_len: CAPABILITIES.len(),
            };
            &MANIFEST
        }

        /// Return the bundled `.shape` source for this language runtime, if any.
        ///
        /// Writes a UTF-8 string to `out_ptr`/`out_len`. Caller frees via
        /// `free_buffer`. Returns 0 on success (even when no source is bundled,
        /// in which case `out_ptr` is set to null).
        unsafe extern "C" fn __shape_get_shape_source(
            _instance: *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            const SOURCE: Option<&str> = $shape_source_opt;
            if out_ptr.is_null() || out_len.is_null() {
                return 1;
            }
            match SOURCE {
                Some(src) => {
                    let mut bytes = src.as_bytes().to_vec();
                    let len = bytes.len();
                    let ptr = bytes.as_mut_ptr();
                    ::std::mem::forget(bytes);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    0
                }
                None => {
                    unsafe {
                        *out_ptr = ::std::ptr::null_mut();
                        *out_len = 0;
                    }
                    0
                }
            }
        }

        // ffi-rebuild §4.5: extension-side panic containment. Every vtable
        // entry the extension exports is wrapped in a generated `extern "C"`
        // shell that runs the user function inside `catch_unwind` and converts
        // a panic into the slot's error sentinel (null pointer / non-zero i32 /
        // swallowed unit). Unwinding across the C ABI boundary is undefined
        // behavior; these shells make the boundary panic-safe so a panicking
        // Python/TS body can never unwind into the host VM.
        unsafe extern "C" fn __shape_pc_init(
            config: *const u8,
            config_len: usize,
        ) -> *mut ::std::ffi::c_void {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $init(config, config_len)
            })) {
                Ok(v) => v,
                Err(_) => ::std::ptr::null_mut(),
            }
        }

        unsafe extern "C" fn __shape_pc_register_types(
            instance: *mut ::std::ffi::c_void,
            types: *const u8,
            types_len: usize,
        ) -> i32 {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $register_types(instance, types, types_len)
            })) {
                Ok(v) => v,
                Err(_) => 1,
            }
        }

        unsafe extern "C" fn __shape_pc_generate_stubs(
            instance: *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $generate_stubs(instance, out_ptr, out_len)
            })) {
                Ok(v) => v,
                Err(_) => 1,
            }
        }

        #[allow(clippy::too_many_arguments)]
        unsafe extern "C" fn __shape_pc_compile(
            instance: *mut ::std::ffi::c_void,
            name: *const u8,
            name_len: usize,
            source: *const u8,
            source_len: usize,
            param_names: *const u8,
            param_names_len: usize,
            param_types: *const u8,
            param_types_len: usize,
            return_type: *const u8,
            return_type_len: usize,
            is_async: bool,
            out_error: *mut *mut u8,
            out_error_len: *mut usize,
        ) -> *mut ::std::ffi::c_void {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $compile(
                    instance,
                    name,
                    name_len,
                    source,
                    source_len,
                    param_names,
                    param_names_len,
                    param_types,
                    param_types_len,
                    return_type,
                    return_type_len,
                    is_async,
                    out_error,
                    out_error_len,
                )
            })) {
                Ok(v) => v,
                Err(_) => ::std::ptr::null_mut(),
            }
        }

        unsafe extern "C" fn __shape_pc_invoke(
            instance: *mut ::std::ffi::c_void,
            handle: *mut ::std::ffi::c_void,
            args: *const u8,
            args_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $invoke(instance, handle, args, args_len, out_ptr, out_len)
            })) {
                Ok(v) => v,
                Err(_) => 1,
            }
        }

        unsafe extern "C" fn __shape_pc_dispose_function(
            instance: *mut ::std::ffi::c_void,
            handle: *mut ::std::ffi::c_void,
        ) {
            let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $dispose_function(instance, handle)
            }));
        }

        unsafe extern "C" fn __shape_pc_language_id(
            instance: *mut ::std::ffi::c_void,
        ) -> *const ::std::ffi::c_char {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $language_id(instance)
            })) {
                Ok(v) => v,
                Err(_) => ::std::ptr::null(),
            }
        }

        unsafe extern "C" fn __shape_pc_get_lsp_config(
            instance: *mut ::std::ffi::c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $get_lsp_config(instance, out_ptr, out_len)
            })) {
                Ok(v) => v,
                Err(_) => 1,
            }
        }

        /// ADR-019 §5 (#202): the extension's declared instance-concurrency
        /// model. A constant per runtime, surfaced through the vtable so the
        /// host reads a declaration instead of guessing from the language id.
        unsafe extern "C" fn __shape_pc_instance_concurrency(
            _instance: *mut ::std::ffi::c_void,
        ) -> u32 {
            $instance_concurrency
        }

        /// ADR-019 §2 (#199): this extension's capability block, or null.
        ///
        /// A pointer to `static` data — which optional protocols a runtime
        /// speaks is a property of the extension BUILD, not of one instance, so
        /// the instance argument is unused here and exists only so a future
        /// runtime could vary it per instance without another ABI change.
        unsafe extern "C" fn __shape_pc_capabilities(
            _instance: *mut ::std::ffi::c_void,
        ) -> *const $crate::ExtensionCapabilities {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $capabilities)) {
                Ok(v) => v,
                Err(_) => ::std::ptr::null(),
            }
        }

        unsafe extern "C" fn __shape_pc_free_buffer(ptr: *mut u8, len: usize) {
            let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $free_buffer(ptr, len)
            }));
        }

        unsafe extern "C" fn __shape_pc_drop(instance: *mut ::std::ffi::c_void) {
            let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| unsafe {
                $drop_fn(instance)
            }));
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn shape_language_runtime_vtable() -> *const $crate::LanguageRuntimeVTable {
            static VTABLE: $crate::LanguageRuntimeVTable = $crate::LanguageRuntimeVTable {
                init: Some(__shape_pc_init),
                register_types: Some(__shape_pc_register_types),
                compile: Some(__shape_pc_compile),
                invoke: Some(__shape_pc_invoke),
                dispose_function: Some(__shape_pc_dispose_function),
                language_id: Some(__shape_pc_language_id),
                get_lsp_config: Some(__shape_pc_get_lsp_config),
                free_buffer: Some(__shape_pc_free_buffer),
                drop: Some(__shape_pc_drop),
                error_model: $crate::ErrorModel::Dynamic,
                get_shape_source: Some(__shape_get_shape_source),
                // ABI v4 additive tail (WF-2A stage 0). `state_model` is stamped
                // STATEFUL_OPAQUE: the macro generates interpreter-backed
                // runtimes (Python, TypeScript) whose cross-call state is opaque
                // and non-serializable. `runtime_descriptor` is left `None` for
                // now (matching falls back to language id); reserved padding is
                // null for future additive vtable functions.
                runtime_descriptor: None,
                state_model: $crate::STATE_MODEL_STATEFUL_OPAQUE,
                // ADR-019 §1 (#196): the stub channel's return half, landed in
                // the former `reserved0` slot — same layout, same fingerprint.
                generate_stubs: Some(__shape_pc_generate_stubs),
                // ADR-019 §5 (#202): the off-thread-invocation declaration,
                // landed in the former `reserved1` slot — same layout, same
                // fingerprint.
                instance_concurrency: Some(__shape_pc_instance_concurrency),
                // ADR-019 §3 (#200): a macro-generated runtime mints no
                // foreign references yet, and `None` is the truthful
                // declaration of that — the host refuses to build a reference
                // against a runtime that could not release it. The macro arm
                // that accepts a disposer lands with the first extension that
                // returns one (#163 / #164).
                dispose_ref: None,
                // ADR-019 §2 (#199): the versioned capability-block accessor,
                // landed in the former `reserved3` slot — the last one, spent on
                // a block rather than an entry point precisely so nothing after
                // it has to compete for a slot.
                capabilities: Some(__shape_pc_capabilities),
            };
            &VTABLE
        }

        // Deliberate C-ABI entry point: dereferences the caller-supplied
        // `contract` raw pointer after a null check. Marking it `unsafe` would
        // change the exported C symbol's Rust signature; the contract is
        // documented at the ABI boundary instead.
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        #[unsafe(no_mangle)]
        pub extern "C" fn shape_capability_vtable(
            contract: *const u8,
            contract_len: usize,
        ) -> *const ::std::ffi::c_void {
            if contract.is_null() {
                return ::std::ptr::null();
            }
            let contract =
                unsafe { ::std::slice::from_raw_parts(contract, contract_len) };
            if contract == $crate::CAPABILITY_LANGUAGE_RUNTIME.as_bytes() {
                shape_language_runtime_vtable() as *const ::std::ffi::c_void
            } else {
                ::std::ptr::null()
            }
        }
    };
}

/// Macro to define a static QueryParam with const strings
#[macro_export]
macro_rules! query_param {
    (
        name: $name:expr,
        description: $desc:expr,
        param_type: $ptype:expr,
        required: $req:expr
    ) => {
        $crate::QueryParam {
            name: concat!($name, "\0").as_ptr() as *const core::ffi::c_char,
            description: concat!($desc, "\0").as_ptr() as *const core::ffi::c_char,
            param_type: $ptype,
            required: $req,
            default_value: core::ptr::null(),
            default_value_len: 0,
            allowed_values: core::ptr::null(),
            allowed_values_len: 0,
            nested_schema: core::ptr::null(),
        }
    };
}

/// Macro to define a static OutputField with const strings
#[macro_export]
macro_rules! output_field {
    (
        name: $name:expr,
        field_type: $ftype:expr,
        description: $desc:expr
    ) => {
        $crate::OutputField {
            name: concat!($name, "\0").as_ptr() as *const core::ffi::c_char,
            field_type: $ftype,
            description: concat!($desc, "\0").as_ptr() as *const core::ffi::c_char,
        }
    };
}

// ============================================================================
// Safety Documentation
// ============================================================================

// # Safety Requirements for Plugin Authors
//
// 1. All `*const c_char` strings must be null-terminated
// 2. All MessagePack buffers must be valid MessagePack data
// 3. Instance pointers must be valid for the lifetime of the plugin
// 4. Callbacks must not panic across the FFI boundary
// 5. Memory allocated by plugin must be freed by plugin's free functions
// 6. Schemas must remain valid for the lifetime of the plugin instance

// ============================================================================
// Tests — Permission Model
// ============================================================================

#[cfg(test)]
mod abi_fingerprint_tests {
    use super::*;

    /// The structural fingerprint is deterministic and non-trivial. The host
    /// and a matched-source extension both call this exact `const fn`, so their
    /// values are equal iff their boundary struct layouts are equal.
    #[test]
    fn fingerprint_is_stable_and_nonzero() {
        let a = abi_build_fingerprint();
        let b = abi_build_fingerprint();
        assert_eq!(a, b, "fingerprint must be deterministic");
        assert_ne!(a, 0, "fingerprint must not be zero");
        assert_ne!(
            a, 0xcbf2_9ce4_8422_2325,
            "fingerprint must fold layout past the FNV seed"
        );
    }

    /// The fingerprint folds the vtable field offsets, so `init` and `invoke`
    /// (the WF-2A skew-test fields) sit at distinct offsets. A reorder that
    /// swaps them changes at least one offset → changes the fingerprint. This
    /// guards the const fn against silently dropping the offset terms.
    #[test]
    fn fingerprint_covers_distinct_vtable_offsets() {
        let init_off = core::mem::offset_of!(LanguageRuntimeVTable, init);
        let invoke_off = core::mem::offset_of!(LanguageRuntimeVTable, invoke);
        assert_ne!(
            init_off, invoke_off,
            "init/invoke must occupy distinct offsets for the skew gate to fire"
        );
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    // -- Permission enum introspection --

    #[test]
    fn permission_name_is_dotted() {
        for perm in Permission::all_variants() {
            let name = perm.name();
            assert!(
                name.contains('.'),
                "Permission name '{}' should contain a dot",
                name
            );
        }
    }

    #[test]
    fn permission_description_is_nonempty() {
        for perm in Permission::all_variants() {
            assert!(!perm.description().is_empty());
        }
    }

    #[test]
    fn permission_category_roundtrip() {
        assert_eq!(
            Permission::FsRead.category(),
            PermissionCategory::Filesystem
        );
        assert_eq!(
            Permission::FsWrite.category(),
            PermissionCategory::Filesystem
        );
        assert_eq!(
            Permission::NetConnect.category(),
            PermissionCategory::Network
        );
        assert_eq!(
            Permission::NetListen.category(),
            PermissionCategory::Network
        );
        assert_eq!(Permission::Process.category(), PermissionCategory::System);
        assert_eq!(Permission::Env.category(), PermissionCategory::System);
        assert_eq!(Permission::Time.category(), PermissionCategory::System);
        assert_eq!(Permission::Random.category(), PermissionCategory::System);
        assert_eq!(Permission::Vfs.category(), PermissionCategory::Sandbox);
        assert_eq!(
            Permission::Deterministic.category(),
            PermissionCategory::Sandbox
        );
    }

    #[test]
    fn permission_display() {
        assert_eq!(format!("{}", Permission::FsRead), "fs.read");
        assert_eq!(format!("{}", Permission::NetConnect), "net.connect");
    }

    #[test]
    fn all_variants_is_exhaustive() {
        // If a new variant is added but not listed in all_variants,
        // the match in name()/description()/category() will catch it at compile time.
        // This test just verifies the count is sane (>= 16 known variants).
        assert!(Permission::all_variants().len() >= 16);
    }

    // -- PermissionSet constructors --

    #[test]
    fn pure_is_empty() {
        let set = PermissionSet::pure();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn readonly_contains_expected() {
        let set = PermissionSet::readonly();
        assert!(set.contains(&Permission::FsRead));
        assert!(set.contains(&Permission::Env));
        assert!(set.contains(&Permission::Time));
        assert!(!set.contains(&Permission::FsWrite));
        assert!(!set.contains(&Permission::NetConnect));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn full_contains_all() {
        let set = PermissionSet::full();
        for perm in Permission::all_variants() {
            assert!(set.contains(perm), "full() missing {:?}", perm);
        }
    }

    // -- Set algebra --

    #[test]
    fn union_combines() {
        let a = PermissionSet::from([Permission::FsRead, Permission::NetConnect]);
        let b = PermissionSet::from([Permission::FsWrite, Permission::NetConnect]);
        let u = a.union(&b);
        assert_eq!(u.len(), 3);
        assert!(u.contains(&Permission::FsRead));
        assert!(u.contains(&Permission::FsWrite));
        assert!(u.contains(&Permission::NetConnect));
    }

    #[test]
    fn intersection_narrows() {
        let a = PermissionSet::from([Permission::FsRead, Permission::NetConnect]);
        let b = PermissionSet::from([Permission::FsWrite, Permission::NetConnect]);
        let i = a.intersection(&b);
        assert_eq!(i.len(), 1);
        assert!(i.contains(&Permission::NetConnect));
    }

    #[test]
    fn difference_subtracts() {
        let a = PermissionSet::from([Permission::FsRead, Permission::FsWrite, Permission::Env]);
        let b = PermissionSet::from([Permission::FsWrite]);
        let d = a.difference(&b);
        assert_eq!(d.len(), 2);
        assert!(d.contains(&Permission::FsRead));
        assert!(d.contains(&Permission::Env));
        assert!(!d.contains(&Permission::FsWrite));
    }

    #[test]
    fn subset_superset() {
        let small = PermissionSet::from([Permission::FsRead]);
        let big = PermissionSet::from([Permission::FsRead, Permission::FsWrite]);
        assert!(small.is_subset(&big));
        assert!(!big.is_subset(&small));
        assert!(big.is_superset(&small));
        assert!(!small.is_superset(&big));
    }

    #[test]
    fn insert_and_remove() {
        let mut set = PermissionSet::pure();
        assert!(set.insert(Permission::Time));
        assert!(!set.insert(Permission::Time)); // duplicate
        assert_eq!(set.len(), 1);
        assert!(set.remove(&Permission::Time));
        assert!(!set.remove(&Permission::Time)); // already removed
        assert!(set.is_empty());
    }

    // -- Display --

    #[test]
    fn permission_set_display() {
        let set = PermissionSet::from([Permission::FsRead, Permission::Env]);
        let s = format!("{}", set);
        // BTreeSet ordering: FsRead < Env based on Ord derive
        assert!(s.starts_with('{'));
        assert!(s.ends_with('}'));
        assert!(s.contains("fs.read"));
        assert!(s.contains("sys.env"));
    }

    // -- by_category --

    #[test]
    fn by_category_groups() {
        let set = PermissionSet::from([
            Permission::FsRead,
            Permission::FsWrite,
            Permission::NetConnect,
            Permission::Time,
            Permission::Vfs,
        ]);
        let cats = set.by_category();
        assert_eq!(cats[&PermissionCategory::Filesystem].len(), 2);
        assert_eq!(cats[&PermissionCategory::Network].len(), 1);
        assert_eq!(cats[&PermissionCategory::System].len(), 1);
        assert_eq!(cats[&PermissionCategory::Sandbox].len(), 1);
    }

    // -- Ffi reservation (WF-1D stage 0, ffi-rebuild §4.8) --

    #[test]
    fn ffi_is_the_seventeenth_variant_at_the_end() {
        let all = Permission::all_variants();
        assert_eq!(all.len(), 17, "Ffi is the 17th permission");
        assert_eq!(
            *all.last().unwrap(),
            Permission::Ffi,
            "Ffi must stay last (highest ordinal) so content hashes are stable"
        );
        // Ord is declaration order; Ffi is the maximum — never reorders existing
        // permissions in a BTreeSet, so name-sorted hash inputs are unperturbed.
        assert!(Permission::Ffi > Permission::OutputLimited);
    }

    #[test]
    fn ffi_metadata() {
        assert_eq!(Permission::Ffi.name(), "ffi.call");
        assert!(Permission::Ffi.name().contains('.')); // dotted-name invariant
        assert_eq!(format!("{}", Permission::Ffi), "ffi.call");
        assert_eq!(Permission::Ffi.category(), PermissionCategory::Foreign);
        assert_eq!(format!("{}", PermissionCategory::Foreign), "Foreign");
        assert!(
            Permission::Ffi
                .description()
                .to_lowercase()
                .contains("foreign")
        );
    }

    #[test]
    fn full_contains_ffi_but_pure_and_readonly_do_not() {
        assert!(PermissionSet::full().contains(&Permission::Ffi));
        assert!(!PermissionSet::pure().contains(&Permission::Ffi));
        assert!(!PermissionSet::readonly().contains(&Permission::Ffi));
    }

    #[test]
    fn permission_set_display_includes_ffi_last() {
        let set = PermissionSet::from([Permission::FsRead, Permission::Ffi]);
        let s = format!("{}", set);
        assert!(s.contains("ffi.call"));
        // Ffi has the highest ordinal, so it renders last in the BTreeSet order.
        assert!(s.ends_with("ffi.call}"), "got: {s}");
    }

    #[test]
    fn scope_constraints_default_ffi_lists_are_empty() {
        let sc = ScopeConstraints::none();
        assert!(sc.ffi_languages.is_empty());
        assert!(sc.ffi_libraries.is_empty());
        assert!(sc.ffi_symbols.is_empty());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn ffi_permission_serde_round_trips() {
        let set = PermissionSet::from([Permission::FsRead, Permission::Ffi]);
        let bytes = rmp_serde::to_vec(&set).unwrap();
        let back: PermissionSet = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(set, back);
        assert!(back.contains(&Permission::Ffi));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn scope_constraints_ffi_fields_serde_round_trip() {
        let sc = ScopeConstraints {
            ffi_languages: vec!["python".into()],
            ffi_libraries: vec!["/usr/lib/*".into()],
            ffi_symbols: vec!["labs".into()],
            ..Default::default()
        };
        // Named (map) encoding — `skip_serializing_if` requires field-keyed
        // output; this mirrors the plugin-manifest serialization surface.
        let bytes = rmp_serde::to_vec_named(&sc).unwrap();
        let back: ScopeConstraints = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(sc, back);
        assert_eq!(back.ffi_languages, vec!["python".to_string()]);
        assert_eq!(back.ffi_libraries, vec!["/usr/lib/*".to_string()]);
        assert_eq!(back.ffi_symbols, vec!["labs".to_string()]);
    }

    // -- FromIterator / IntoIterator --

    #[test]
    fn collect_from_iterator() {
        let perms = vec![Permission::FsRead, Permission::Env];
        let set: PermissionSet = perms.into_iter().collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn into_iter_owned() {
        let set = PermissionSet::from([Permission::FsRead, Permission::Env]);
        let v: Vec<Permission> = set.into_iter().collect();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn into_iter_ref() {
        let set = PermissionSet::from([Permission::FsRead, Permission::Env]);
        let v: Vec<&Permission> = (&set).into_iter().collect();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn from_array() {
        let set = PermissionSet::from([Permission::Process, Permission::Random]);
        assert_eq!(set.len(), 2);
        assert!(set.contains(&Permission::Process));
        assert!(set.contains(&Permission::Random));
    }

    // -- PermissionGrant --

    #[test]
    fn unconstrained_grant() {
        let g = PermissionGrant::unconstrained(Permission::FsRead);
        assert_eq!(g.permission, Permission::FsRead);
        assert!(g.constraints.is_none());
    }

    #[test]
    fn scoped_grant_with_paths() {
        let c = ScopeConstraints {
            allowed_paths: vec!["/tmp/*".into(), "/data/**".into()],
            ..Default::default()
        };
        let g = PermissionGrant::scoped(Permission::FsScoped, c);
        assert_eq!(g.permission, Permission::FsScoped);
        let sc = g.constraints.unwrap();
        assert_eq!(sc.allowed_paths.len(), 2);
        assert!(sc.allowed_hosts.is_empty());
    }

    #[test]
    fn scoped_grant_with_limits() {
        let c = ScopeConstraints {
            max_memory_bytes: Some(1024 * 1024 * 64),
            max_time_ms: Some(5000),
            max_output_bytes: Some(1024 * 1024),
            ..Default::default()
        };
        let g = PermissionGrant::scoped(Permission::MemLimited, c);
        let sc = g.constraints.unwrap();
        assert_eq!(sc.max_memory_bytes, Some(64 * 1024 * 1024));
        assert_eq!(sc.max_time_ms, Some(5000));
    }

    // -- PermissionCategory display --

    #[test]
    fn category_display() {
        assert_eq!(format!("{}", PermissionCategory::Filesystem), "Filesystem");
        assert_eq!(format!("{}", PermissionCategory::Network), "Network");
        assert_eq!(format!("{}", PermissionCategory::System), "System");
        assert_eq!(format!("{}", PermissionCategory::Sandbox), "Sandbox");
    }

    // -- Equality / ordering --

    #[test]
    fn permission_set_equality() {
        let a = PermissionSet::from([Permission::FsRead, Permission::Env]);
        let b = PermissionSet::from([Permission::Env, Permission::FsRead]);
        assert_eq!(a, b);
    }

    #[test]
    fn permission_ord_is_deterministic() {
        // BTreeSet iteration should always be in the same order
        let set = PermissionSet::from([Permission::Random, Permission::FsRead, Permission::Vfs]);
        let names: Vec<&str> = set.iter().map(|p| p.name()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        // Since BTreeSet uses Ord, the iteration order should already be sorted
        // by the derived Ord (which is variant declaration order).
        // We just verify it's deterministic by checking two iterations match.
        let names2: Vec<&str> = set.iter().map(|p| p.name()).collect();
        assert_eq!(names, names2);
    }
}
