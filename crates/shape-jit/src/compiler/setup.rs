//! JIT Compiler initialization and setup

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::context::{JITConfig, SimulationKernelConfig};
use crate::error::JitError;
use crate::ffi_symbols::{declare_ffi_functions, register_ffi_symbols};
use shape_runtime::simulation::{KernelCompileConfig, KernelCompiler, SimulationKernelFn};
use shape_vm::bytecode::BytecodeProgram;

pub struct JITCompiler {
    pub(super) module: JITModule,
    pub(super) builder_context: FunctionBuilderContext,
    #[allow(dead_code)]
    pub(super) config: JITConfig,
    pub(super) compiled_functions: HashMap<String, *const u8>,
    pub(super) ffi_funcs: HashMap<String, cranelift_module::FuncId>,
    pub(super) function_table: Vec<*const u8>,
    /// Maps function_id → FuncId of their `opt_dc_*` direct-call entry point.
    /// Used for cross-function speculative direct calls: when function A has
    /// monomorphic feedback for callee B, and B has already been Tier-2
    /// compiled, A can emit a direct `call` to B's direct-call entry.
    pub(super) compiled_dc_funcs: HashMap<u16, (cranelift_module::FuncId, u16)>,
}

impl JITCompiler {
    /// Borrow the underlying JITModule (for declaring/defining functions).
    pub fn module_mut(&mut self) -> &mut JITModule {
        &mut self.module
    }

    /// Borrow the FunctionBuilderContext (reused across compilations).
    pub fn builder_context_mut(&mut self) -> &mut FunctionBuilderContext {
        &mut self.builder_context
    }
}

impl JITCompiler {
    #[inline(always)]
    pub fn new(config: JITConfig) -> Result<Self, JitError> {
        let mut flag_builder = settings::builder();
        let opt_level_str = if config.opt_level >= 2 {
            "speed"
        } else {
            "speed_and_size"
        };
        flag_builder
            .set("opt_level", opt_level_str)
            .map_err(|e| JitError::Setup(format!("Failed to set opt_level: {}", e)))?;
        flag_builder
            .set("is_pic", "false")
            .map_err(|e| JitError::Setup(format!("Failed to set is_pic: {}", e)))?;

        let isa_builder = cranelift_native::builder()
            .map_err(|e| JitError::Setup(format!("Failed to create ISA builder: {}", e)))?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| JitError::Setup(format!("Failed to create ISA: {}", e)))?;

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

        register_ffi_symbols(&mut builder);

        let mut module = JITModule::new(builder);

        let ffi_funcs = declare_ffi_functions(&mut module);

        Ok(Self {
            module,
            builder_context: FunctionBuilderContext::new(),
            config,
            compiled_functions: HashMap::new(),
            ffi_funcs,
            function_table: Vec::new(),
            compiled_dc_funcs: HashMap::new(),
        })
    }
}

// ============================================================================
// KernelCompiler Trait Implementation
// ============================================================================

/// Thread-safe wrapper around JITCompiler for use with ExecutionContext.
///
/// This wrapper implements the `KernelCompiler` trait from `shape-runtime`,
/// enabling JIT kernel compilation to be injected into ExecutionContext without
/// circular dependencies.
pub struct JITKernelCompiler {
    /// Inner JIT compiler protected by mutex for thread safety
    compiler: Mutex<JITCompiler>,
}

impl JITKernelCompiler {
    /// Create a new JIT kernel compiler with default configuration.
    pub fn new() -> Result<Self, JitError> {
        Ok(Self {
            compiler: Mutex::new(JITCompiler::new(JITConfig::default())?),
        })
    }

    /// Create a new JIT kernel compiler with custom configuration.
    pub fn with_config(config: JITConfig) -> Result<Self, JitError> {
        Ok(Self {
            compiler: Mutex::new(JITCompiler::new(config)?),
        })
    }
}

impl Default for JITKernelCompiler {
    fn default() -> Self {
        Self::new().expect("Failed to create JIT compiler with default config")
    }
}

// Safety: JITKernelCompiler is thread-safe because:
// 1. All access to the inner JITCompiler is protected by a Mutex
// 2. The raw pointers in JITCompiler are function pointers to compiled code,
//    which are immutable after compilation
// 3. We never expose the raw pointers outside the Mutex
unsafe impl Send for JITKernelCompiler {}
unsafe impl Sync for JITKernelCompiler {}

impl KernelCompiler for JITKernelCompiler {
    fn compile_kernel(
        &self,
        name: &str,
        function_bytecode: &[u8],
        config: &KernelCompileConfig,
    ) -> Result<SimulationKernelFn, String> {
        // Deserialize bytecode (wire-native MessagePack first, JSON fallback for compatibility).
        let program: BytecodeProgram = rmp_serde::from_slice(function_bytecode)
            .or_else(|mp_err| {
                serde_json::from_slice(function_bytecode).map_err(|json_err| {
                    format!(
                        "Failed to deserialize bytecode as MessagePack ({mp_err}) or JSON ({json_err})"
                    )
                })
            })?;

        // Convert KernelCompileConfig to SimulationKernelConfig
        let mut jit_config =
            SimulationKernelConfig::new(config.state_schema_id, config.column_count);

        // Add state field offsets
        for (field_name, offset) in &config.state_field_offsets {
            jit_config
                .state_field_offsets
                .push((field_name.clone(), *offset));
        }

        // Add column mappings
        for (col_name, idx) in &config.column_map {
            jit_config.column_map.push((col_name.clone(), *idx));
        }

        // Acquire lock and compile
        let mut compiler = self
            .compiler
            .lock()
            .map_err(|e| format!("Failed to acquire JIT compiler lock: {}", e))?;

        // Call the JIT compiler's compile_simulation_kernel
        let kernel_fn = compiler.compile_simulation_kernel(name, &program, &jit_config)?;

        // The function pointer type is the same, just transmute
        // Safety: SimulationKernelFn in shape-jit has the same signature as in shape-runtime
        Ok(unsafe { std::mem::transmute(kernel_fn) })
    }

    fn supports_feature(&self, feature: &str) -> bool {
        match feature {
            "typed_object" => true,
            "closures" => false, // Phase 1: no closure support
            "multi_table" => true,
            _ => false,
        }
    }
}
