//! Plugin System for Shape
//!
//! Provides dynamic loading of data source and output sink plugins using
//! the stable C ABI defined in `shape-abi-v1`.
//!
//! # Overview
//!
//! Plugins are dynamically loaded shared libraries (.so/.dll/.dylib) that
//! implement the Shape plugin interface. This enables:
//!
//! - Runtime extension without recompilation
//! - Third-party data source integrations
//! - Custom alert/output sinks
//!
//! # Security Note
//!
//! Plugin loading executes arbitrary code. Only load plugins from trusted sources.

mod data_source;
pub mod language_runtime;
mod loader;
mod module_capability;
mod output_sink;

pub use data_source::{
    ParsedOutputField, ParsedOutputSchema, ParsedQueryParam, ParsedQuerySchema, PluginDataSource,
};
pub use language_runtime::{CompiledForeignFunction, PluginLanguageRuntime, RuntimeLspConfig};
pub use loader::{
    ClaimedSection, LoadedPlugin, PluginCapability, PluginLoader, parse_sections_manifest,
};
pub use module_capability::{
    ParsedModuleArtifact, ParsedModuleFunction, ParsedModuleSchema, PluginModule,
};
pub use output_sink::PluginOutputSink;

// Re-export ABI types for convenience
pub use shape_abi_v1::{
    ABI_VERSION, AlertSeverity, CapabilityKind, DataSourceVTable, OutputField, OutputSchema,
    OutputSinkVTable, ParamType, PluginError, PluginInfo, PluginType, QueryParam, QuerySchema,
    SectionClaim, SectionsManifest,
};
