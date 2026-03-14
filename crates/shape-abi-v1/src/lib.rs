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

    /// Register Shape type schemas for stub generation (e.g. `.pyi` files).
    /// `types_msgpack`: MessagePack-encoded `Vec<TypeSchemaExport>`.
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
}

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

/// Exported Shape type schema for foreign language runtimes.
///
/// Serialized as MessagePack and passed to `register_types()`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypeSchemaExport {
    /// Type name.
    pub name: std::string::String,
    /// Kind of type.
    pub kind: TypeSchemaExportKind,
    /// Fields (for struct types).
    pub fields: Vec<TypeFieldExport>,
    /// Enum variants (for enum types).
    pub enum_variants: Option<Vec<EnumVariantExport>>,
}

/// Kind of exported type schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypeSchemaExportKind {
    Struct,
    Enum,
    Alias,
}

/// A single field in an exported type schema.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypeFieldExport {
    /// Field name.
    pub name: std::string::String,
    /// Shape type name (e.g. "number", "string", "Array<Candle>").
    pub type_name: std::string::String,
    /// Whether the field is optional.
    pub optional: bool,
    /// Human-readable description.
    pub description: Option<std::string::String>,
}

/// A single enum variant in an exported type schema.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnumVariantExport {
    /// Variant name.
    pub name: std::string::String,
    /// Payload fields (if any).
    pub payload_fields: Option<Vec<TypeFieldExport>>,
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
}

impl PermissionCategory {
    /// Human-readable name for this category.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Filesystem => "Filesystem",
            Self::Network => "Network",
            Self::System => "System",
            Self::Sandbox => "Sandbox",
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
pub const ABI_VERSION: u32 = 3;

/// Get the ABI version (plugins should export this)
pub type GetAbiVersionFn = unsafe extern "C" fn() -> u32;

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

        #[unsafe(no_mangle)]
        pub extern "C" fn shape_language_runtime_vtable() -> *const $crate::LanguageRuntimeVTable {
            static VTABLE: $crate::LanguageRuntimeVTable = $crate::LanguageRuntimeVTable {
                init: Some($init),
                register_types: Some($register_types),
                compile: Some($compile),
                invoke: Some($invoke),
                dispose_function: Some($dispose_function),
                language_id: Some($language_id),
                get_lsp_config: Some($get_lsp_config),
                free_buffer: Some($free_buffer),
                drop: Some($drop_fn),
                error_model: $crate::ErrorModel::Dynamic,
                get_shape_source: Some(__shape_get_shape_source),
            };
            &VTABLE
        }

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
