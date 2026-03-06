//! Extension system surface for Shape runtime.
//!
//! This module is the public extension-first API. Internally, the current
//! implementation still lives in `plugins` while migration completes.

pub use crate::plugins::{
    ABI_VERSION, AlertSeverity, CapabilityKind, DataSourceVTable, OutputField, OutputSchema,
    OutputSinkVTable, ParamType, ParsedModuleArtifact, ParsedModuleFunction, ParsedModuleSchema,
    ParsedOutputField, ParsedOutputSchema, ParsedQueryParam, ParsedQuerySchema, PluginError,
    PluginInfo, PluginType, QueryParam, QuerySchema,
};

pub type LoadedExtension = crate::plugins::LoadedPlugin;
pub type ExtensionCapability = crate::plugins::PluginCapability;
pub type ExtensionLoader = crate::plugins::PluginLoader;
pub type ExtensionDataSource = crate::plugins::PluginDataSource;
pub type ExtensionOutputSink = crate::plugins::PluginOutputSink;
pub type ExtensionModule = crate::plugins::PluginModule;
