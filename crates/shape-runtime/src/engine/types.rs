//! Types for execution results and metrics

use crate::query_result::QueryType;
use serde::{Deserialize, Serialize};
use shape_wire::WireValue;
use shape_wire::metadata::TypeInfo;

/// Snapshot of preloaded engine state (semantic + runtime context).
///
/// This is used by services (e.g. shape-server) to avoid reloading stdlib
/// from disk on every request while still creating an isolated engine per run.
#[derive(Clone)]
pub struct EngineBootstrapState {
    pub semantic: crate::snapshot::SemanticSnapshot,
    pub context: crate::context::ExecutionContext,
}

/// Result from executing Shape code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The value returned by the execution
    pub value: WireValue,
    /// Type information for the return value
    pub type_info: Option<TypeInfo>,
    /// Type of execution
    pub execution_type: ExecutionType,
    /// Performance metrics
    pub metrics: ExecutionMetrics,
    /// Any warnings or info messages
    pub messages: Vec<Message>,
    /// JSON representation of a Content node (if the result is Content)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_json: Option<serde_json::Value>,
    /// HTML representation of a Content node (if the result is Content)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_html: Option<String>,
    /// Terminal (ANSI) representation of a Content node (if the result is Content)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_terminal: Option<String>,
}

/// Type of execution performed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionType {
    /// Query execution (find, scan, analyze, etc.)
    Query(QueryType),
    /// Function execution
    Function(String),
    /// Script/expression evaluation
    Script,
    /// REPL command
    Repl,
}

/// Performance metrics for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    /// Total execution time in milliseconds
    pub execution_time_ms: u64,
    /// Parse time in milliseconds
    pub parse_time_ms: u64,
    /// Analysis time in milliseconds
    pub analysis_time_ms: u64,
    /// Runtime execution time in milliseconds
    pub runtime_time_ms: u64,
    /// Memory used in bytes
    pub memory_used_bytes: Option<usize>,
    /// Number of rows processed
    pub rows_processed: Option<usize>,
}

/// Messages from execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub level: MessageLevel,
    pub text: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageLevel {
    Info,
    Warning,
    Error,
}
