//! Generic Simulation Engine
//!
//! This module provides a domain-agnostic simulation engine that can be used
//! for any kind of event-driven processing over time series data.
//!
//! The engine is designed to be:
//! - Domain-agnostic: No knowledge of finance, IoT, or any specific domain
//! - Composable: Can be wrapped by domain-specific engines
//! - Efficient: Supports batch and streaming modes
//!
//! Finance-specific features like position tracking, fill simulation, and
//! P&L calculation should be implemented in Shape stdlib, not here.
//!
//! # High-Performance Mode (TypedObject)
//!
//! For maximum performance (>10M ticks/sec), simulation state should be a
//! TypedObject - a fixed-layout object created from a type declaration.
//! Use `require_typed_state()` to enforce this at runtime.
//!
//! ```shape
//! type BacktestState {
//!     cash: f64,
//!     position: f64,
//!     entry_price: f64
//! }
//!
//! let state = BacktestState { cash: 100000.0, position: 0.0, entry_price: 0.0 }
//! simulate(data, state, strategy)  // state will use TypedObject optimization
//! ```

// Module declarations
pub mod correlated_kernel;
pub mod dense_kernel;
pub mod engine;
pub mod event_scheduler;
pub mod hybrid_kernel;
pub mod parallel;
pub mod validation;

// Re-export all public types for backward compatibility
pub use validation::{require_typed_state_with_schema, validate_typed_state};

pub use engine::{
    SimulationEngine, SimulationEngineConfig, SimulationEngineResult, SimulationEvent,
    SimulationMode, StepHandler, StepResult,
};

pub use dense_kernel::{
    DenseKernel, DenseKernelConfig, DenseKernelResult, KernelCompileConfig, KernelCompiler,
    SimulationKernelFn, simulate,
};

pub use correlated_kernel::{
    CorrelatedKernel, CorrelatedKernelConfig, CorrelatedKernelFn, CorrelatedKernelResult,
    TableSchema, simulate_correlated,
};

pub use event_scheduler::{EventQueue, ScheduledEvent};

pub use hybrid_kernel::{
    EventHandlerFn, HybridKernel, HybridKernelConfig, HybridKernelResult, simulate_hybrid,
};

pub use parallel::{ParallelSweepResult, par_run, par_run_with_config, param_grid, param_grid3};
