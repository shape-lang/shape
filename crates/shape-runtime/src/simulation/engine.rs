//! Generic Simulation Engine
//!
//! This module provides a domain-agnostic simulation engine for event-driven
//! processing over time series data.

use shape_ast::error::Result;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// Type alias: simulation engine uses ValueWord as its runtime Value type.
type Value = ValueWord;

/// Mode of simulation execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimulationMode {
    /// Process all data at once
    #[default]
    Batch,
    /// Process data as it arrives (streaming)
    Stream,
}

/// Event emitted during simulation
#[derive(Debug, Clone)]
pub struct SimulationEvent {
    /// Index of the data point that triggered this event
    pub index: usize,
    /// Type of event (user-defined string)
    pub event_type: String,
    /// Event payload
    pub data: Value,
}

/// Result of a simulation step
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Updated state after this step
    pub state: Value,
    /// Events emitted during this step
    pub events: Vec<SimulationEvent>,
    /// Whether to continue processing
    pub continue_processing: bool,
}

impl StepResult {
    /// Create a result that just updates state
    pub fn with_state(state: Value) -> Self {
        Self {
            state,
            events: vec![],
            continue_processing: true,
        }
    }

    /// Create a result with state and events
    pub fn with_events(state: Value, events: Vec<SimulationEvent>) -> Self {
        Self {
            state,
            events,
            continue_processing: true,
        }
    }

    /// Create a result that stops processing
    pub fn stop(state: Value) -> Self {
        Self {
            state,
            events: vec![],
            continue_processing: false,
        }
    }
}

/// Handler function type for processing each element
pub type StepHandler = Box<dyn Fn(&Value, &Value, usize) -> Result<StepResult>>;

/// Configuration for the simulation engine
pub struct SimulationEngineConfig {
    /// Execution mode
    pub mode: SimulationMode,
    /// Initial state
    pub initial_state: Value,
    /// Maximum number of events to collect (0 = unlimited)
    pub max_events: usize,
    /// Whether to track state history
    pub track_state_history: bool,
}

impl Default for SimulationEngineConfig {
    fn default() -> Self {
        Self {
            mode: SimulationMode::default(),
            initial_state: ValueWord::none(),
            max_events: 0,
            track_state_history: false,
        }
    }
}

impl SimulationEngineConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_initial_state(mut self, state: Value) -> Self {
        self.initial_state = state;
        self
    }

    pub fn with_mode(mut self, mode: SimulationMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_max_events(mut self, max: usize) -> Self {
        self.max_events = max;
        self
    }

    pub fn tracking_state_history(mut self) -> Self {
        self.track_state_history = true;
        self
    }
}

/// Result of running a complete simulation
#[derive(Debug, Clone)]
pub struct SimulationEngineResult {
    /// Final state after all processing
    pub final_state: Value,
    /// All events emitted during simulation
    pub events: Vec<SimulationEvent>,
    /// State history if tracking was enabled
    pub state_history: Option<Vec<Value>>,
    /// Number of elements processed
    pub elements_processed: usize,
    /// Whether simulation completed normally
    pub completed: bool,
}

impl SimulationEngineResult {
    /// Convert to a Shape Value
    pub fn to_value(&self) -> Value {
        // Convert events to ValueWord array
        let events_value: Vec<ValueWord> = self
            .events
            .iter()
            .map(|e| {
                crate::type_schema::typed_object_from_nb_pairs(&[
                    ("index", ValueWord::from_f64(e.index as f64)),
                    (
                        "type",
                        ValueWord::from_string(Arc::new(e.event_type.clone())),
                    ),
                    ("data", e.data.clone()),
                ])
            })
            .collect();

        let mut nb_pairs: Vec<(&str, ValueWord)> = vec![
            ("final_state", self.final_state.clone()),
            (
                "elements_processed",
                ValueWord::from_f64(self.elements_processed as f64),
            ),
            ("completed", ValueWord::from_bool(self.completed)),
            ("events", ValueWord::from_array(shape_value::vmarray_from_vec(events_value))),
        ];

        if let Some(history) = &self.state_history {
            let history_nb: Vec<ValueWord> = history.iter().map(|v| v.clone()).collect();
            nb_pairs.push(("state_history", ValueWord::from_array(shape_value::vmarray_from_vec(history_nb))));
        }

        crate::type_schema::typed_object_from_nb_pairs(&nb_pairs).clone()
    }
}

/// Generic simulation engine for event-driven processing
pub struct SimulationEngine {
    config: SimulationEngineConfig,
}

impl SimulationEngine {
    /// Create a new simulation engine with the given configuration
    pub fn new(config: SimulationEngineConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(SimulationEngineConfig::default())
    }

    /// Run simulation over a sequence of values
    ///
    /// The handler is called for each value with (value, current_state, index)
    /// and returns a StepResult containing the new state and any events.
    pub fn run<F>(&self, data: &[Value], handler: F) -> Result<SimulationEngineResult>
    where
        F: Fn(&Value, &Value, usize) -> Result<StepResult>,
    {
        let mut state = self.config.initial_state.clone();
        let mut events = Vec::new();
        let mut state_history = if self.config.track_state_history {
            Some(Vec::with_capacity(data.len()))
        } else {
            None
        };
        let mut completed = true;

        for (i, value) in data.iter().enumerate() {
            // Run the step
            let step_result = handler(value, &state, i)?;

            // Update state
            state = step_result.state;

            // Track state history if enabled
            if let Some(ref mut history) = state_history {
                history.push(state.clone());
            }

            // Collect events (respecting max_events limit)
            for event in step_result.events {
                if self.config.max_events == 0 || events.len() < self.config.max_events {
                    events.push(event);
                }
            }

            // Check if we should stop
            if !step_result.continue_processing {
                completed = false;
                break;
            }
        }

        Ok(SimulationEngineResult {
            final_state: state,
            events,
            state_history,
            elements_processed: data.len(),
            completed,
        })
    }

    /// Run simulation with pre-processing and post-processing hooks
    pub fn run_with_hooks<F, Pre, Post>(
        &self,
        data: &[Value],
        pre_process: Pre,
        handler: F,
        post_process: Post,
    ) -> Result<SimulationEngineResult>
    where
        F: Fn(&Value, &Value, usize) -> Result<StepResult>,
        Pre: Fn(&Value) -> Result<Value>,
        Post: Fn(SimulationEngineResult) -> Result<SimulationEngineResult>,
    {
        // Pre-process initial state
        let initial_state = pre_process(&self.config.initial_state)?;

        // Create a modified config with pre-processed state
        let modified_engine = SimulationEngine::new(SimulationEngineConfig {
            initial_state,
            ..self.config.clone()
        });

        // Run simulation
        let result = modified_engine.run(data, handler)?;

        // Post-process result
        post_process(result)
    }
}

// Allow cloning the config
impl Clone for SimulationEngineConfig {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode,
            initial_state: self.config_value_clone(&self.initial_state),
            max_events: self.max_events,
            track_state_history: self.track_state_history,
        }
    }
}

impl SimulationEngineConfig {
    fn config_value_clone(&self, value: &Value) -> Value {
        value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_simulation_engine_basic() {
        let config = SimulationEngineConfig::new().with_initial_state(ValueWord::from_f64(0.0));

        let engine = SimulationEngine::new(config);

        let data = vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ];

        // Sum all values
        let result = engine
            .run(&data, |value, state, _idx| {
                let v = value.as_f64().unwrap_or(0.0);
                let s = state.as_f64().unwrap_or(0.0);
                Ok(StepResult::with_state(ValueWord::from_f64(v + s)))
            })
            .unwrap();

        assert_eq!(result.elements_processed, 3);
        assert!(result.completed);
        assert_eq!(result.final_state.as_f64(), Some(6.0));
    }

    #[test]
    fn test_simulation_engine_with_events() {
        let config = SimulationEngineConfig::new().with_initial_state(ValueWord::from_f64(0.0));

        let engine = SimulationEngine::new(config);

        let data = vec![
            ValueWord::from_f64(5.0),
            ValueWord::from_f64(15.0), // This triggers event
            ValueWord::from_f64(8.0),
        ];

        // Emit event when value > 10
        let result = engine
            .run(&data, |value, state, idx| {
                let mut events = vec![];

                if let Some(v) = value.as_f64() {
                    if v > 10.0 {
                        events.push(SimulationEvent {
                            index: idx,
                            event_type: "threshold_exceeded".to_string(),
                            data: value.clone(),
                        });
                    }
                }

                Ok(StepResult::with_events(state.clone(), events))
            })
            .unwrap();

        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].event_type, "threshold_exceeded");
        assert_eq!(result.events[0].index, 1);
    }

    #[test]
    fn test_simulation_engine_early_stop() {
        let config = SimulationEngineConfig::new().with_initial_state(ValueWord::from_f64(0.0));

        let engine = SimulationEngine::new(config);

        let data = vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(-1.0), // This stops processing
            ValueWord::from_f64(3.0),
        ];

        // Stop when negative value encountered
        let result = engine
            .run(&data, |value, state, _idx| {
                if let Some(v) = value.as_f64() {
                    if v < 0.0 {
                        return Ok(StepResult::stop(state.clone()));
                    }
                }
                Ok(StepResult::with_state(state.clone()))
            })
            .unwrap();

        // Completed should be false because we stopped early
        assert!(!result.completed);
    }

    #[test]
    fn test_simulation_result_to_value() {
        let result = SimulationEngineResult {
            final_state: ValueWord::from_f64(42.0),
            events: vec![SimulationEvent {
                index: 5,
                event_type: "test".to_string(),
                data: ValueWord::from_string(Arc::new("data".to_string())),
            }],
            state_history: None,
            elements_processed: 10,
            completed: true,
        };

        let value = result.to_value();
        let obj =
            crate::type_schema::typed_object_to_hashmap_nb(&value).expect("Expected TypedObject");
        assert!(obj.contains_key("final_state"));
        assert!(obj.contains_key("events"));
        assert!(obj.contains_key("elements_processed"));
        assert!(obj.contains_key("completed"));
    }

    #[test]
    fn test_state_history_tracking() {
        let config = SimulationEngineConfig::new()
            .with_initial_state(ValueWord::from_f64(0.0))
            .tracking_state_history();

        let engine = SimulationEngine::new(config);
        let data = vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ];

        let result = engine
            .run(&data, |value, state, _idx| {
                let v = value.as_f64().unwrap_or(0.0);
                let s = state.as_f64().unwrap_or(0.0);
                Ok(StepResult::with_state(ValueWord::from_f64(v + s)))
            })
            .unwrap();

        let history = result
            .state_history
            .as_ref()
            .expect("Should have state history");
        assert_eq!(history.len(), 3);
        // After step 0: 0 + 1 = 1
        assert_eq!(history[0].as_f64(), Some(1.0));
        // After step 1: 1 + 2 = 3
        assert_eq!(history[1].as_f64(), Some(3.0));
        // After step 2: 3 + 3 = 6
        assert_eq!(history[2].as_f64(), Some(6.0));
    }

    #[test]
    fn test_state_maintained_across_steps() {
        // Simulate a buy-hold-sell cycle using a TypedObject state
        let initial_state = crate::type_schema::typed_object_from_nb_pairs(&[
            ("cash", ValueWord::from_f64(10000.0)),
            ("position", ValueWord::from_f64(0.0)),
        ]);

        let config = SimulationEngineConfig::new()
            .with_initial_state(initial_state)
            .tracking_state_history();

        let engine = SimulationEngine::new(config);

        // Prices: buy at 100, hold at 105, sell at 110
        let data = vec![
            ValueWord::from_f64(100.0), // buy
            ValueWord::from_f64(105.0), // hold
            ValueWord::from_f64(110.0), // sell
        ];

        let result = engine
            .run(&data, |value, state, idx| {
                let price = match value.as_f64() {
                    Some(p) => p,
                    None => return Ok(StepResult::with_state(state.clone())),
                };
                let obj = crate::type_schema::typed_object_to_hashmap_nb(state).unwrap_or_default();
                let cash = obj.get("cash").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let position = obj.get("position").and_then(|v| v.as_f64()).unwrap_or(0.0);

                let (new_cash, new_position) = match idx {
                    0 => (cash - 10.0 * price, 10.0),    // Buy 10 shares at 100
                    2 => (cash + position * price, 0.0), // Sell 10 shares at 110
                    _ => (cash, position),               // Hold
                };
                let new_state = crate::type_schema::typed_object_from_nb_pairs(&[
                    ("cash", ValueWord::from_f64(new_cash)),
                    ("position", ValueWord::from_f64(new_position)),
                ]);
                Ok(StepResult::with_state(new_state))
            })
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.elements_processed, 3);

        // Final state: bought at 100, sold at 110, profit = 10 * 10 = 100
        let obj = crate::type_schema::typed_object_to_hashmap_nb(&result.final_state)
            .expect("Expected TypedObject for final state");
        {
            let cash = obj
                .get("cash")
                .and_then(|v| v.as_f64())
                .expect("Expected cash");
            let position = obj
                .get("position")
                .and_then(|v| v.as_f64())
                .expect("Expected position");
            // Started with 10000, bought 10 at 100 = -1000, sold 10 at 110 = +1100
            assert_eq!(cash, 10100.0);
            assert_eq!(position, 0.0);
        }

        // Verify state history tracks all three steps
        let history = result.state_history.as_ref().unwrap();
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn test_max_events_limit() {
        let config = SimulationEngineConfig::new()
            .with_initial_state(ValueWord::from_f64(0.0))
            .with_max_events(2);

        let engine = SimulationEngine::new(config);
        let data = vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
            ValueWord::from_f64(4.0),
            ValueWord::from_f64(5.0),
        ];

        let result = engine
            .run(&data, |value, state, idx| {
                let events = vec![SimulationEvent {
                    index: idx,
                    event_type: "tick".to_string(),
                    data: value.clone(),
                }];
                Ok(StepResult::with_events(state.clone(), events))
            })
            .unwrap();

        // Should only collect 2 events despite 5 being emitted
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].index, 0);
        assert_eq!(result.events[1].index, 1);
    }

    #[test]
    fn test_run_with_hooks() {
        let config = SimulationEngineConfig::new().with_initial_state(ValueWord::from_f64(0.0));

        let engine = SimulationEngine::new(config);
        let data = vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ];

        let result = engine
            .run_with_hooks(
                &data,
                // Pre-process: set initial state to 10
                |_initial| Ok(ValueWord::from_f64(10.0)),
                // Handler: add each value to state
                |value, state, _idx| {
                    let v = value.as_f64().unwrap_or(0.0);
                    let s = state.as_f64().unwrap_or(0.0);
                    Ok(StepResult::with_state(ValueWord::from_f64(v + s)))
                },
                // Post-process: double the final state
                |mut result| {
                    if let Some(n) = result.final_state.as_f64() {
                        result.final_state = ValueWord::from_f64(n * 2.0);
                    }
                    Ok(result)
                },
            )
            .unwrap();

        // Pre: 10.0, then +1+2+3 = 16.0, then post: * 2 = 32.0
        assert_eq!(result.final_state.as_f64(), Some(32.0));
    }

    #[test]
    fn test_empty_data() {
        let config = SimulationEngineConfig::new().with_initial_state(ValueWord::from_f64(42.0));

        let engine = SimulationEngine::new(config);
        let data: Vec<Value> = vec![];

        let result = engine
            .run(&data, |_value, _state, _idx| {
                panic!("Should not be called on empty data");
            })
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.elements_processed, 0);
        assert_eq!(result.final_state.as_f64(), Some(42.0));
    }

    #[test]
    fn test_result_to_value_with_history() {
        let result = SimulationEngineResult {
            final_state: ValueWord::from_f64(10.0),
            events: vec![],
            state_history: Some(vec![
                ValueWord::from_f64(1.0),
                ValueWord::from_f64(5.0),
                ValueWord::from_f64(10.0),
            ]),
            elements_processed: 3,
            completed: true,
        };

        let value = result.to_value();
        let obj =
            crate::type_schema::typed_object_to_hashmap_nb(&value).expect("Expected TypedObject");
        assert!(obj.contains_key("state_history"));
        if let Some(history_nb) = obj.get("state_history") {
            let history = history_nb
                .as_any_array()
                .expect("Expected array")
                .to_generic();
            assert_eq!(history.len(), 3);
        } else {
            panic!("Expected state_history field");
        }
    }
}
