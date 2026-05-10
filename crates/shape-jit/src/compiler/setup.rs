//! JIT Compiler initialization and setup

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use std::collections::HashMap;

use crate::context::JITConfig;
use crate::error::JitError;
use crate::ffi_symbols::{declare_ffi_functions, register_ffi_symbols};

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
// KernelCompiler Trait Implementation — REMOVED (W11 / Phase-2c)
// ============================================================================
//
// The `JITKernelCompiler` wrapper and its `impl KernelCompiler for
// JITKernelCompiler` block are removed because the runtime-side
// `shape_runtime::simulation` module (with `KernelCompiler`,
// `KernelCompileConfig`, `SimulationKernelFn`) was bulldozed in
// commit `2601ba7` ("phase-2c: bulldoze simulation engine subtree").
// Per the 2026-05-06 defection entry, the simulation engine subtree
// is deferred to extensions workstream
// `simulation-kernel-extension-rebuild`.
//
// This is a deleted-runtime-side dependency, classified as W11 /
// deeper Phase-2c per the wave-10 jit-playbook §6 ("What's NOT in
// W10"). The JIT-side wrapper has no surviving consumer trait to
// implement, so the surface-and-stop response is wholesale removal,
// not a `todo!()` shell — there is no shape to stub.
//
// The compile_simulation_kernel entry point on `JITCompiler` itself
// remains (it is called only via the now-removed wrapper); its body
// references shape_runtime types that are also gone, so any future
// re-introduction must come through the simulation-kernel-extension-
// rebuild workstream.
