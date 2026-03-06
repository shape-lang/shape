//! Stream processing types for Shape AST

use serde::{Deserialize, Serialize};

use super::program::VariableDecl;
use super::span::Span;
use super::statements::Statement;
use super::time::Timeframe;

/// Stream definition for real-time data processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDef {
    pub name: String,
    pub name_span: Span,
    pub config: StreamConfig,
    pub state: Vec<VariableDecl>,
    pub on_connect: Option<Vec<Statement>>,
    pub on_disconnect: Option<Vec<Statement>>,
    pub on_event: Option<StreamOnEvent>,
    pub on_window: Option<StreamOnWindow>,
    pub on_error: Option<StreamOnError>,
}

/// Stream configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    pub provider: String,
    pub symbols: Vec<String>,
    pub timeframes: Vec<Timeframe>,
    pub buffer_size: Option<u32>,
    pub reconnect: Option<bool>,
    pub reconnect_delay: Option<f64>, // seconds
}

/// Handler for individual events (single data point)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOnEvent {
    pub event_param: String,
    pub body: Vec<Statement>,
}

/// Handler for windowed/aggregated data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOnWindow {
    pub key_param: String,
    pub window_param: String,
    pub body: Vec<Statement>,
}

/// Handler for error events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOnError {
    pub error_param: String,
    pub body: Vec<Statement>,
}
