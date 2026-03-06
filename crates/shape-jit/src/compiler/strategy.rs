//! Strategy compilation
//!
//! This module provides two compilation modes:
//!
//! 1. **Standard ABI** (`compile_strategy`): Uses `fn(*mut JITContext) -> i32`
//!    - Full access to VM features (closures, FFI, etc.)
//!    - Suitable for general-purpose JIT compilation
//!
//! 2. **Kernel ABI** (`compile_simulation_kernel`): Uses `fn(usize, *const *const f64, *mut u8) -> i32`
//!    - Zero-allocation hot path for simulation
//!    - Direct memory access to series data and state
//!    - Enables >10M ticks/sec performance

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};
use std::collections::HashMap;

use super::setup::JITCompiler;
use crate::context::{
    CorrelatedKernelFn, JittedStrategyFn, SimulationKernelConfig, SimulationKernelFn,
};
use crate::translator::BytecodeToIR;
use shape_vm::bytecode::BytecodeProgram;

impl JITCompiler {
    #[inline(always)]
    pub fn compile_strategy(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
    ) -> Result<JittedStrategyFn, String> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));

        let func_id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let ctx_ptr = builder.block_params(entry_block)[0];

            let ffi = self.build_ffi_refs(&mut builder);

            let mut compiler = BytecodeToIR::new(
                &mut builder,
                program,
                ctx_ptr,
                ffi,
                HashMap::new(),
                HashMap::new(),
            );
            let result = compiler.compile()?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function (strategy): {:?}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize (strategy): {:?}", e))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        self.compiled_functions.insert(name.to_string(), code_ptr);

        Ok(unsafe { std::mem::transmute(code_ptr) })
    }

    #[inline(always)]
    pub(super) fn compile_strategy_with_user_funcs(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
        user_func_ids: &HashMap<u16, cranelift_module::FuncId>,
        user_func_arities: &HashMap<u16, u16>,
    ) -> Result<cranelift_module::FuncId, String> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));

        let func_id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let ctx_ptr = builder.block_params(entry_block)[0];

            let mut user_func_refs: HashMap<u16, FuncRef> = HashMap::new();
            for (&fn_idx, &fn_id) in user_func_ids {
                let func_ref = self.module.declare_func_in_func(fn_id, builder.func);
                user_func_refs.insert(fn_idx, func_ref);
            }

            let ffi = self.build_ffi_refs(&mut builder);

            let mut compiler = BytecodeToIR::new(
                &mut builder,
                program,
                ctx_ptr,
                ffi,
                user_func_refs,
                user_func_arities.clone(),
            );

            // Set skip ranges so the main function compilation ignores function
            // body instructions (they are compiled separately via
            // compile_function_with_user_funcs). Without this, LoopStart/LoopEnd
            // and Jump targets inside function bodies create blocks in the main
            // function context, causing dead code compilation and stack corruption.
            compiler.skip_ranges = Self::compute_skip_ranges(program);

            let result = compiler.compile()?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function (strategy): {:?}", e))?;

        self.module.clear_context(&mut ctx);

        Ok(func_id)
    }

    /// Compute instruction index ranges to skip when compiling the main strategy.
    ///
    /// Bytecode layout for programs with user functions:
    /// ```text
    /// [0]             Jump → trampoline1     (skip func0 body)
    /// [entry0 .. t1)  func0 body
    /// [t1]            Jump → trampoline2     (skip func1 body)
    /// [entry1 .. t2)  func1 body
    /// ...
    /// [main_start ..) main code
    /// ```
    ///
    /// Returns the function body ranges (excluding trampoline jumps between them).
    pub(super) fn compute_skip_ranges(program: &BytecodeProgram) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();

        // Skip function bodies (they are compiled separately).
        for f in program.functions.iter() {
            if f.body_length == 0 {
                continue;
            }
            ranges.push((f.entry_point, f.entry_point + f.body_length));
        }

        ranges
    }

    // ========================================================================
    // Simulation Kernel Compilation (Zero-Allocation Hot Path)
    // ========================================================================

    /// Compile a simulation kernel with the specialized kernel ABI.
    ///
    /// The kernel ABI bypasses JITContext to achieve maximum throughput:
    /// - Direct pointer arithmetic for data access
    /// - No allocations in the hot path
    /// - Inlined field access with known offsets
    ///
    /// # Arguments
    /// * `name` - Function name for the compiled kernel
    /// * `program` - Bytecode program containing the strategy
    /// * `config` - Kernel configuration with field offset mappings
    ///
    /// # Returns
    /// A function pointer with signature: `fn(usize, *const *const f64, *mut u8) -> i32`
    ///
    /// # Generated Code Pattern
    ///
    /// For a strategy like:
    /// ```shape
    /// let price = candle.close
    /// if price > state.threshold {
    ///     state.signal = 1.0
    /// }
    /// ```
    ///
    /// The kernel generates:
    /// ```asm
    /// ; price = candle.close (column 3)
    /// mov rax, [series_ptrs + 3*8]     ; column pointer
    /// mov xmm0, [rax + cursor_index*8] ; price value
    ///
    /// ; state.threshold (offset 16)
    /// mov xmm1, [state_ptr + 16]       ; threshold value
    ///
    /// ; comparison and store
    /// ucomisd xmm0, xmm1
    /// jbe skip
    /// mov qword [state_ptr + 24], 1.0  ; state.signal
    /// skip:
    /// ```
    #[inline(always)]
    pub fn compile_simulation_kernel(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
        config: &SimulationKernelConfig,
    ) -> Result<SimulationKernelFn, String> {
        // Kernel ABI signature: fn(cursor_index: usize, series_ptrs: *const *const f64, state_ptr: *mut u8) -> i32
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // cursor_index
        sig.params.push(AbiParam::new(types::I64)); // series_ptrs
        sig.params.push(AbiParam::new(types::I64)); // state_ptr
        sig.returns.push(AbiParam::new(types::I32)); // result code

        let func_id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare kernel function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            // Get kernel parameters
            let cursor_index = builder.block_params(entry_block)[0];
            let series_ptrs = builder.block_params(entry_block)[1];
            let state_ptr = builder.block_params(entry_block)[2];

            // Build kernel-specific IR
            let result = self.build_kernel_ir(
                &mut builder,
                program,
                config,
                cursor_index,
                series_ptrs,
                state_ptr,
            )?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define kernel function: {:?}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize kernel: {:?}", e))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        self.compiled_functions.insert(name.to_string(), code_ptr);

        Ok(unsafe { std::mem::transmute(code_ptr) })
    }

    /// Build kernel IR using BytecodeToIR in kernel mode.
    ///
    /// This compiles bytecode to kernel ABI IR with direct memory access:
    /// - GetFieldTyped → state_ptr + offset
    /// - GetDataField → series_ptrs[col][cursor]
    /// - All locals as Cranelift variables
    fn build_kernel_ir(
        &mut self,
        builder: &mut FunctionBuilder,
        program: &BytecodeProgram,
        config: &SimulationKernelConfig,
        cursor_index: Value,
        series_ptrs: Value,
        state_ptr: Value,
    ) -> Result<Value, String> {
        // Build FFI refs (some may still be needed for complex operations)
        let ffi = self.build_ffi_refs(builder);

        // Create BytecodeToIR in kernel mode
        let mut compiler = BytecodeToIR::new_kernel_mode(
            builder,
            program,
            cursor_index,
            series_ptrs,
            state_ptr,
            ffi,
            config.clone(),
        );

        // Compile bytecode to kernel IR
        compiler.compile_kernel()
    }

    // ========================================================================
    // Correlated Kernel Compilation (Multi-Series Simulation)
    // ========================================================================

    /// Compile a correlated (multi-series) simulation kernel.
    ///
    /// This extends the simulation kernel to support multiple aligned time series,
    /// enabling cross-series strategies (e.g., SPY vs VIX, temperature vs pressure).
    ///
    /// # Arguments
    /// * `name` - Function name for the compiled kernel
    /// * `program` - Bytecode program containing the strategy
    /// * `config` - Kernel configuration with series mappings
    ///
    /// # Returns
    /// A function pointer with signature:
    /// `fn(cursor_index: usize, series_ptrs: *const *const f64, table_count: usize, state_ptr: *mut u8) -> i32`
    ///
    /// # Generated Code Pattern
    ///
    /// For a strategy like:
    /// ```shape
    /// let spy_price = context.spy    // series index 0
    /// let vix_level = context.vix    // series index 1
    /// if vix_level > 25.0 && state.position == 0 {
    ///     state.signal = 1.0
    /// }
    /// ```
    ///
    /// The kernel generates:
    /// ```asm
    /// ; spy_price = context.spy (series index 0)
    /// mov rax, [series_ptrs + 0*8]     ; series 0 pointer
    /// mov xmm0, [rax + cursor_index*8] ; spy value
    ///
    /// ; vix_level = context.vix (series index 1)
    /// mov rax, [series_ptrs + 1*8]     ; series 1 pointer
    /// mov xmm1, [rax + cursor_index*8] ; vix value
    ///
    /// ; comparison and conditional store
    /// mov xmm2, [const_25.0]
    /// ucomisd xmm1, xmm2
    /// jbe skip
    /// ; ... check state.position == 0 ...
    /// mov qword [state_ptr + signal_offset], 1.0
    /// skip:
    /// ```
    #[inline(always)]
    pub fn compile_correlated_kernel(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
        config: &SimulationKernelConfig,
    ) -> Result<CorrelatedKernelFn, String> {
        // Validate config is for multi-series mode
        if !config.is_multi_table() {
            return Err(
                "compile_correlated_kernel requires multi-series config (use new_multi_table)"
                    .to_string(),
            );
        }

        // Correlated kernel ABI:
        // fn(cursor_index: usize, series_ptrs: *const *const f64, table_count: usize, state_ptr: *mut u8) -> i32
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // cursor_index
        sig.params.push(AbiParam::new(types::I64)); // series_ptrs
        sig.params.push(AbiParam::new(types::I64)); // table_count
        sig.params.push(AbiParam::new(types::I64)); // state_ptr
        sig.returns.push(AbiParam::new(types::I32)); // result code

        let func_id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare correlated kernel function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            // Get kernel parameters
            let cursor_index = builder.block_params(entry_block)[0];
            let series_ptrs = builder.block_params(entry_block)[1];
            let _table_count = builder.block_params(entry_block)[2]; // For validation/debugging
            let state_ptr = builder.block_params(entry_block)[3];

            // Build correlated kernel IR
            // Note: table_count is known at compile time from config, used for validation
            let result = self.build_correlated_kernel_ir(
                &mut builder,
                program,
                config,
                cursor_index,
                series_ptrs,
                state_ptr,
            )?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define correlated kernel function: {:?}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize correlated kernel: {:?}", e))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        self.compiled_functions.insert(name.to_string(), code_ptr);

        Ok(unsafe { std::mem::transmute(code_ptr) })
    }

    /// Build correlated kernel IR for multi-series access.
    ///
    /// Handles series access via compile-time resolved indices:
    /// - `context.spy` → `series_ptrs[0][cursor_idx]` (if spy mapped to index 0)
    /// - `context.vix` → `series_ptrs[1][cursor_idx]` (if vix mapped to index 1)
    fn build_correlated_kernel_ir(
        &mut self,
        builder: &mut FunctionBuilder,
        program: &BytecodeProgram,
        config: &SimulationKernelConfig,
        cursor_index: Value,
        series_ptrs: Value,
        state_ptr: Value,
    ) -> Result<Value, String> {
        // Build FFI refs
        let ffi = self.build_ffi_refs(builder);

        // Create BytecodeToIR in correlated kernel mode
        // The translator will use config.table_map to resolve series names to indices
        let mut compiler = BytecodeToIR::new_kernel_mode(
            builder,
            program,
            cursor_index,
            series_ptrs,
            state_ptr,
            ffi,
            config.clone(),
        );

        // Compile bytecode to correlated kernel IR
        // The translator handles GetSeriesValue opcode by:
        // 1. Looking up series name in config.table_map to get index
        // 2. Generating: series_ptrs[index][cursor_index]
        compiler.compile_kernel()
    }
}
