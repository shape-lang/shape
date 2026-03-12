// ShapeError carries location info for good diagnostics, making it larger than clippy's threshold.
#![allow(clippy::result_large_err)]

//! Shape Virtual Machine
//!
//! A stack-based bytecode VM for executing Shape programs efficiently.
//!
//! Shape is a general-purpose scientific computing language for high-speed
//! time-series analysis. This VM provides fast execution of Shape programs
//! across any domain (finance, IoT, sensors, healthcare, manufacturing, etc.).
//!
//! # Module Structure
//! - `configuration` - BytecodeExecutor struct, constructors, extension registration
//! - `module_resolution` - Module loading, virtual modules, file-based import handling
//! - `execution` - Compilation pipeline, VM execution loop, snapshot resume

pub mod blob_cache_v2;
pub mod bundle_compiler;
pub mod bytecode;
pub mod bytecode_cache;
pub mod compiler;
mod configuration;
pub mod constants;
pub mod debugger;
pub mod deopt;
mod execution;
pub mod executor;
pub mod feature_matrix;
pub mod feature_tests;
pub mod feedback;
pub mod hot_reload;
pub mod linker;
pub mod megamorphic_cache;
pub mod mir;
#[cfg(feature = "jit")]
compile_error!(
    "The `shape-vm/jit` feature is deprecated. JIT functionality moved to the `shape-jit` crate."
);
pub mod memory;
pub mod metrics;
pub mod module_resolution;
pub mod remote;
pub mod resource_limits;
pub mod stdlib;
pub mod tier;
pub mod type_tracking;

pub use bytecode::{BytecodeProgram, Instruction, OpCode, StringId};
pub use compiler::BytecodeCompiler;
pub use configuration::BytecodeExecutor;
pub use debugger::{DebugCommand, VMDebugger};
#[cfg(feature = "quic")]
pub use executor::clear_quic_transport_config;
#[cfg(feature = "quic")]
pub use executor::configure_quic_transport;
pub use executor::{
    CallFrame, DebugVMState, ExecutionResult as VMExecutionResult, VMConfig, VirtualMachine,
    reset_transport_provider, set_transport_provider,
};
pub use feature_matrix::{FeatureCategory, FeatureTest};
pub use memory::{GCConfig, GCResult, GarbageCollector, ObjectId};
pub use type_tracking::{FrameDescriptor, SlotKind, StorageHint, TypeTracker, VariableTypeInfo};

// Re-export ValueWord and related types from shape-value
pub use shape_value::{ErrorLocation, LocatedVMError, Upvalue, VMContext, VMError};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
