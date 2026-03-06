//! Stream executor for real-time data processing
//!
//! This module executes StreamDef blocks by:
//! 1. Connecting to data providers via plugins
//! 2. Managing subscriptions
//! 3. Invoking handlers (on_tick, on_bar, on_connect, on_disconnect, on_error)
//! 4. Maintaining stream state across callbacks

use crate::context::ExecutionContext;
use crate::data::Timeframe;
use crate::plugins::PluginLoader;
use parking_lot::RwLock;
use shape_ast::ast::{Statement, StreamDef, VariableDecl};
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Stream execution state
#[derive(Debug)]
pub struct StreamState {
    /// Stream name
    pub name: String,
    /// State variables (stored as ValueWord for efficiency)
    pub variables: HashMap<String, ValueWord>,
    /// Active subscription IDs
    pub subscriptions: Vec<u64>,
    /// Is the stream running
    pub running: bool,
}

/// Message types for stream callbacks
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Generic tick/update data (single data point)
    Tick {
        /// Data identifier (e.g., symbol, device_id, sensor_id)
        id: String,
        /// Dynamic field data (price, temperature, power, etc.)
        fields: std::collections::HashMap<String, ValueWord>,
        /// Timestamp of the update
        timestamp: i64,
    },
    /// Generic aggregated data (bar, candle, summary, etc.)
    Data {
        /// Data identifier (e.g., symbol, device_id, sensor_id)
        id: String,
        /// Optional timeframe/period (e.g., 1m, 5m, 1h)
        timeframe: Option<Timeframe>,
        /// Dynamic field data (open/high/low/close/volume or ANY fields)
        fields: std::collections::HashMap<String, ValueWord>,
        /// Timestamp when aggregation occurred
        timestamp: i64,
    },
    /// Connection established
    Connected,
    /// Connection lost
    Disconnected,
    /// Error occurred
    Error { message: String },
    /// Shutdown signal
    Shutdown,
}

/// Stream executor manages real-time data streams
pub struct StreamExecutor {
    /// Plugin loader for data sources
    plugin_loader: Arc<RwLock<PluginLoader>>,
    /// Expression evaluator for executing handlers
    evaluator: Option<Arc<dyn crate::engine::ExpressionEvaluator>>,
    /// Active streams
    streams: HashMap<String, StreamState>,
    /// Event channel for receiving stream events
    event_rx: Option<mpsc::Receiver<(String, StreamEvent)>>,
    /// Event sender for sending stream events
    event_tx: mpsc::Sender<(String, StreamEvent)>,
}

impl StreamExecutor {
    /// Create a new stream executor
    pub fn new(plugin_loader: Arc<RwLock<PluginLoader>>) -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        Self {
            plugin_loader,
            evaluator: None,
            streams: HashMap::new(),
            event_rx: Some(event_rx),
            event_tx,
        }
    }

    /// Create a stream executor with an expression evaluator for handler execution
    pub fn with_evaluator(
        plugin_loader: Arc<RwLock<PluginLoader>>,
        evaluator: Arc<dyn crate::engine::ExpressionEvaluator>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        Self {
            plugin_loader,
            evaluator: Some(evaluator),
            streams: HashMap::new(),
            event_rx: Some(event_rx),
            event_tx,
        }
    }

    /// Get event sender for plugins to send events
    pub fn event_sender(&self) -> mpsc::Sender<(String, StreamEvent)> {
        self.event_tx.clone()
    }

    /// Start a stream from a StreamDef
    pub async fn start_stream(
        &mut self,
        stream_def: &StreamDef,
        ctx: &mut ExecutionContext,
    ) -> Result<()> {
        let stream_name = stream_def.name.clone();

        // Check if already running
        if self.streams.contains_key(&stream_name) {
            return Err(ShapeError::RuntimeError {
                message: format!("Stream '{}' is already running", stream_name),
                location: None,
            });
        }

        // Initialize state variables
        let mut state_vars = HashMap::new();
        for var_decl in &stream_def.state {
            let value = self.initialize_variable(var_decl, ctx)?;
            // Get all identifiers from the pattern
            for ident in var_decl.pattern.get_identifiers() {
                state_vars.insert(ident, value.clone());
            }
        }

        // Create stream state
        let stream_state = StreamState {
            name: stream_name.clone(),
            variables: state_vars,
            subscriptions: Vec::new(),
            running: true,
        };

        self.streams.insert(stream_name.clone(), stream_state);

        // Fire on_connect handler
        if let Some(on_connect) = &stream_def.on_connect {
            self.execute_handler(on_connect, &stream_name, HashMap::new(), ctx)?;
        }

        // Subscribe to data
        self.subscribe_to_data(stream_def, ctx).await?;

        Ok(())
    }

    /// Stop a running stream
    pub fn stop_stream(&mut self, name: &str) -> Result<()> {
        let stream_state = self
            .streams
            .get_mut(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Stream '{}' is not running", name),
                location: None,
            })?;

        stream_state.running = false;

        // Unsubscribe from all data sources
        let plugin_loader = self.plugin_loader.read();
        for sub_id in &stream_state.subscriptions {
            // Try to find the plugin and unsubscribe
            // Note: We'd need to track which plugin each subscription belongs to
            let _ = sub_id;
        }
        drop(plugin_loader);

        self.streams.remove(name);
        Ok(())
    }

    /// Handle an incoming event
    pub fn handle_event(
        &mut self,
        stream_name: &str,
        event: StreamEvent,
        stream_def: &StreamDef,
        ctx: &mut ExecutionContext,
    ) -> Result<()> {
        let stream_state =
            self.streams
                .get_mut(stream_name)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Stream '{}' is not running", stream_name),
                    location: None,
                })?;

        if !stream_state.running {
            return Ok(());
        }

        match event {
            StreamEvent::Tick {
                id,
                fields,
                timestamp,
            } => {
                if let Some(on_event) = &stream_def.on_event {
                    // Convert fields to ValueWord and add metadata
                    let mut nb_fields: Vec<(String, ValueWord)> =
                        fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    nb_fields.push(("id".to_string(), ValueWord::from_string(Arc::new(id))));
                    nb_fields.push((
                        "timestamp".to_string(),
                        ValueWord::from_f64(timestamp as f64),
                    ));

                    let pairs: Vec<(&str, ValueWord)> = nb_fields
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.clone()))
                        .collect();
                    let event_obj = crate::type_schema::typed_object_from_nb_pairs(&pairs);

                    let mut params = HashMap::new();
                    params.insert(on_event.event_param.clone(), event_obj.clone());

                    self.execute_handler(&on_event.body, stream_name, params, ctx)?;
                }
            }

            StreamEvent::Data {
                id,
                timeframe,
                fields,
                timestamp,
            } => {
                if let Some(on_window) = &stream_def.on_window {
                    // Convert fields to ValueWord and add metadata
                    let mut nb_fields: Vec<(String, ValueWord)> =
                        fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    nb_fields.push((
                        "timestamp".to_string(),
                        ValueWord::from_f64(timestamp as f64),
                    ));
                    if let Some(tf) = timeframe {
                        nb_fields.push(("timeframe".to_string(), ValueWord::from_timeframe(tf)));
                    }

                    let pairs: Vec<(&str, ValueWord)> = nb_fields
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.clone()))
                        .collect();
                    let window_obj = crate::type_schema::typed_object_from_nb_pairs(&pairs);

                    let mut params = HashMap::new();
                    params.insert(
                        on_window.key_param.clone(),
                        ValueWord::from_string(Arc::new(id.clone())),
                    );
                    params.insert(on_window.window_param.clone(), window_obj.clone());

                    self.execute_handler(&on_window.body, stream_name, params, ctx)?;
                }
            }

            StreamEvent::Connected => {
                if let Some(on_connect) = &stream_def.on_connect {
                    self.execute_handler(on_connect, stream_name, HashMap::new(), ctx)?;
                }
            }

            StreamEvent::Disconnected => {
                if let Some(on_disconnect) = &stream_def.on_disconnect {
                    self.execute_handler(on_disconnect, stream_name, HashMap::new(), ctx)?;
                }
            }

            StreamEvent::Error { message } => {
                if let Some(on_error) = &stream_def.on_error {
                    let mut params = HashMap::new();
                    params.insert(
                        on_error.error_param.clone(),
                        ValueWord::from_string(Arc::new(message)),
                    );
                    self.execute_handler(&on_error.body, stream_name, params, ctx)?;
                }
            }

            StreamEvent::Shutdown => {
                stream_state.running = false;
            }
        }

        Ok(())
    }

    /// Execute a handler with given parameters
    fn execute_handler(
        &mut self,
        statements: &[Statement],
        stream_name: &str,
        params: HashMap<String, ValueWord>,
        ctx: &mut ExecutionContext,
    ) -> Result<()> {
        // Get stream state variables
        let stream_state = self.streams.get_mut(stream_name);

        // Set up context with stream variables and parameters
        if let Some(state) = stream_state {
            for (name, value) in &state.variables {
                let _ = ctx.set_variable_nb(name, value.clone());
            }
        }

        // Add handler parameters to context
        for (name, value) in params {
            let _ = ctx.set_variable(&name, value);
        }

        // Execute statements via evaluator
        if let Some(ref evaluator) = self.evaluator {
            let _ = evaluator.eval_statements(statements, ctx)?;
        }

        // Update stream state variables from context
        if let Some(state) = self.streams.get_mut(stream_name) {
            for name in state.variables.keys().cloned().collect::<Vec<_>>() {
                if let Ok(Some(value)) = ctx.get_variable_nb(&name) {
                    state.variables.insert(name, value.clone());
                }
            }
        }

        Ok(())
    }

    /// Initialize a variable declaration
    fn initialize_variable(
        &self,
        var_decl: &VariableDecl,
        ctx: &mut ExecutionContext,
    ) -> Result<ValueWord> {
        if let Some(init_expr) = &var_decl.value {
            if let Some(ref evaluator) = self.evaluator {
                Ok(evaluator.eval_expr(init_expr, ctx)?)
            } else {
                Ok(ValueWord::none())
            }
        } else {
            Ok(ValueWord::none())
        }
    }

    /// Subscribe to data sources based on stream config
    async fn subscribe_to_data(
        &mut self,
        stream_def: &StreamDef,
        _ctx: &mut ExecutionContext,
    ) -> Result<()> {
        let config = &stream_def.config;
        let stream_name = stream_def.name.clone();

        // Verify the plugin exists
        let plugin_loader = self.plugin_loader.read();
        if plugin_loader
            .get_data_source_vtable(&config.provider)
            .is_err()
        {
            return Err(ShapeError::RuntimeError {
                message: format!("Data source plugin '{}' not found", config.provider),
                location: None,
            });
        }
        drop(plugin_loader);

        // Log subscription setup
        tracing::info!(
            "Stream '{}' subscribing to {} symbols via provider '{}'",
            stream_name,
            config.symbols.len(),
            config.provider
        );

        // TODO: Implement actual plugin subscription via FFI callback
        // For now, subscriptions are set up externally and events are pushed
        // via the event_sender() method

        Ok(())
    }

    /// Run the event loop (call this in an async context)
    pub async fn run_event_loop(
        &mut self,
        stream_defs: HashMap<String, StreamDef>,
        ctx: &mut ExecutionContext,
    ) -> Result<()> {
        let mut rx = self
            .event_rx
            .take()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "Event loop already running".to_string(),
                location: None,
            })?;

        while let Some((stream_name, event)) = rx.recv().await {
            if let Some(stream_def) = stream_defs.get(&stream_name) {
                if let Err(e) = self.handle_event(&stream_name, event.clone(), stream_def, ctx) {
                    tracing::error!("Error handling stream event: {}", e);
                }

                // Check for shutdown
                if matches!(event, StreamEvent::Shutdown) {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Get list of running streams
    pub fn list_streams(&self) -> Vec<&str> {
        self.streams.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a stream is running
    pub fn is_running(&self, name: &str) -> bool {
        self.streams.get(name).map(|s| s.running).unwrap_or(false)
    }

    /// Get stream state for inspection
    pub fn get_state(&self, name: &str) -> Option<&StreamState> {
        self.streams.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_types() {
        let mut fields = std::collections::HashMap::new();
        fields.insert("price".to_string(), ValueWord::from_f64(150.0));
        fields.insert("volume".to_string(), ValueWord::from_f64(1000.0));

        let tick = StreamEvent::Tick {
            id: "AAPL".to_string(),
            fields,
            timestamp: 1234567890,
        };

        match tick {
            StreamEvent::Tick { id, fields, .. } => {
                assert_eq!(id, "AAPL");
                assert_eq!(fields.get("price"), Some(&ValueWord::from_f64(150.0)));
            }
            _ => panic!("Expected tick event"),
        }
    }

    #[test]
    fn test_stream_state_creation() {
        let state = StreamState {
            name: "test_stream".to_string(),
            variables: HashMap::new(),
            subscriptions: Vec::new(),
            running: true,
        };

        assert!(state.running);
        assert_eq!(state.name, "test_stream");
    }
}
