//! Main compilation logic for BytecodeToIR

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use std::collections::HashMap;

use crate::context::*;
use crate::nan_boxing::*;
use shape_vm::bytecode::{BytecodeProgram, DeoptInfo, InlineFrameInfo, OpCode, Operand};
use shape_vm::feedback::FeedbackVector;
use shape_vm::type_tracking::{SlotKind, StorageHint};

use super::loop_analysis;
use super::types::{BytecodeToIR, CompilationMode, FFIFuncRefs, InlineCandidate};
use crate::optimizer;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    pub(crate) fn new(
        builder: &'a mut FunctionBuilder<'b>,
        program: &'a BytecodeProgram,
        ctx_ptr: Value,
        ffi: FFIFuncRefs,
        user_funcs: HashMap<u16, FuncRef>,
        user_func_arities: HashMap<u16, u16>,
    ) -> Self {
        // Pre-compute loop end targets by scanning for matching LoopStart/LoopEnd pairs
        let mut loop_ends = HashMap::new();
        let mut loop_starts = Vec::new();
        for (i, instr) in program.instructions.iter().enumerate() {
            match instr.opcode {
                OpCode::LoopStart => loop_starts.push(i),
                OpCode::LoopEnd => {
                    if let Some(start_idx) = loop_starts.pop() {
                        loop_ends.insert(start_idx, i);
                    }
                }
                _ => {}
            }
        }

        // Run loop analysis before code generation
        let loop_info = loop_analysis::analyze_loops(program);
        let optimization_plan = optimizer::build_function_plan(program, &loop_info);

        // Analyze which functions can be inlined at call sites
        let inline_candidates = Self::analyze_inline_candidates(program);
        let mut local_types = HashMap::new();
        for (idx, hint) in program
            .top_level_local_storage_hints
            .iter()
            .copied()
            .enumerate()
        {
            if hint != StorageHint::Unknown {
                local_types.insert(idx as u16, hint);
            }
        }
        let mut module_binding_types = HashMap::new();
        for (idx, hint) in program
            .module_binding_storage_hints
            .iter()
            .copied()
            .enumerate()
        {
            if hint != StorageHint::Unknown {
                module_binding_types.insert(idx as u16, hint);
            }
        }

        Self {
            builder,
            program,
            ctx_ptr,
            stack_depth: 0,
            stack_vars: HashMap::new(),
            locals: HashMap::new(),
            next_var: 0,
            blocks: HashMap::new(),
            current_block_idx: 0,
            ffi,
            loop_stack: Vec::new(),
            loop_ends,
            exit_block: None,
            compile_time_sp: 0,
            merge_blocks: std::collections::HashSet::new(),
            block_stack_depth: HashMap::new(),
            pending_data_offset: None,
            exception_handlers: Vec::new(),
            current_instr_idx: 0,
            user_funcs,
            user_func_arities,
            stack_types: HashMap::new(),
            local_types,
            module_binding_types,
            typed_stack: super::storage::TypedStack::new(),
            // Kernel mode fields (unused in standard mode)
            mode: CompilationMode::Standard,
            kernel_cursor_index: None,
            kernel_series_ptrs: None,
            kernel_state_ptr: None,
            kernel_config: None,
            loop_info,
            optimization_plan,
            hoisted_locals: HashMap::new(),
            local_f64_cache: HashMap::new(),
            // Function inlining
            inline_candidates,
            inline_local_base: 0,
            inline_depth: 0,
            // Reference tracking
            ref_stack_slots: HashMap::new(),
            // Integer unboxing
            unboxed_int_locals: std::collections::HashSet::new(),
            unboxed_int_module_bindings: std::collections::HashSet::new(),
            promoted_module_bindings: HashMap::new(),
            register_carried_module_bindings: std::collections::HashSet::new(),
            unboxed_loop_depth: 0,
            unboxed_scope_stack: Vec::new(),
            register_carried_loop_depth: 0,
            pending_rebox: None,
            pending_rebox_module_bindings: None,
            pending_flush_module_bindings: None,
            // Float unboxing
            unboxed_f64_locals: std::collections::HashSet::new(),
            f64_local_vars: HashMap::new(),
            pending_rebox_f64: None,
            precomputed_f64_for_invariant_int: HashMap::new(),
            precomputed_f64_scope_stack: Vec::new(),
            // Skip ranges (empty by default)
            skip_ranges: Vec::new(),
            // Array LICM
            hoisted_array_info: HashMap::new(),
            hoisted_ref_array_info: HashMap::new(),
            // Numeric parameter hints (compile-time)
            numeric_param_hints: std::collections::HashSet::new(),
            deopt_block: None,
            deopt_signal_var: None,
            // Deopt tracking
            deopt_points: Vec::new(),
            func_locals_count: 0,
            deferred_spills: Vec::new(),
            // Loop unrolling
            pending_unroll: None,
            trusted_array_push_local_sites: std::collections::HashSet::new(),
            trusted_array_push_local_iv_by_site: HashMap::new(),
            // Shape guard tracking
            shape_guards_emitted: Vec::new(),
            // Feedback-guided speculation (populated by Tier 2 requests)
            feedback: None,
            // Multi-frame inline deopt
            compiling_function_id: 0, // Set by caller (compile_optimizing_function)
            inline_frame_stack: Vec::new(),
        }
    }

    /// Create compiler in kernel mode for simulation hot path.
    ///
    /// Kernel mode bypasses JITContext and uses direct pointers:
    /// - cursor_index: Current row in the simulation (usize)
    /// - series_ptrs: Pointer to series data array (*const *const f64)
    /// - state_ptr: Pointer to TypedObject state buffer (*mut u8)
    ///
    /// This enables >10M ticks/sec by eliminating all indirection.
    pub(crate) fn new_kernel_mode(
        builder: &'a mut FunctionBuilder<'b>,
        program: &'a BytecodeProgram,
        cursor_index: Value,
        series_ptrs: Value,
        state_ptr: Value,
        ffi: FFIFuncRefs,
        config: SimulationKernelConfig,
    ) -> Self {
        // Pre-compute loop ends (same as standard mode)
        let mut loop_ends = HashMap::new();
        let mut loop_starts = Vec::new();
        for (i, instr) in program.instructions.iter().enumerate() {
            match instr.opcode {
                OpCode::LoopStart => loop_starts.push(i),
                OpCode::LoopEnd => {
                    if let Some(start_idx) = loop_starts.pop() {
                        loop_ends.insert(start_idx, i);
                    }
                }
                _ => {}
            }
        }

        // Run loop analysis for kernel mode too
        let loop_info = loop_analysis::analyze_loops(program);
        let optimization_plan = optimizer::build_function_plan(program, &loop_info);
        let mut local_types = HashMap::new();
        for (idx, hint) in program
            .top_level_local_storage_hints
            .iter()
            .copied()
            .enumerate()
        {
            if hint != StorageHint::Unknown {
                local_types.insert(idx as u16, hint);
            }
        }
        let mut module_binding_types = HashMap::new();
        for (idx, hint) in program
            .module_binding_storage_hints
            .iter()
            .copied()
            .enumerate()
        {
            if hint != StorageHint::Unknown {
                module_binding_types.insert(idx as u16, hint);
            }
        }

        Self {
            builder,
            program,
            ctx_ptr: cursor_index, // Reuse field (not used as ctx in kernel mode)
            stack_depth: 0,
            stack_vars: HashMap::new(),
            locals: HashMap::new(),
            next_var: 0,
            blocks: HashMap::new(),
            current_block_idx: 0,
            ffi,
            loop_stack: Vec::new(),
            loop_ends,
            exit_block: None,
            compile_time_sp: 0,
            merge_blocks: std::collections::HashSet::new(),
            block_stack_depth: HashMap::new(),
            pending_data_offset: None,
            exception_handlers: Vec::new(),
            current_instr_idx: 0,
            user_funcs: HashMap::new(),
            user_func_arities: HashMap::new(),
            stack_types: HashMap::new(),
            local_types,
            module_binding_types,
            typed_stack: super::storage::TypedStack::new(),
            // Kernel mode fields
            mode: CompilationMode::Kernel,
            kernel_cursor_index: Some(cursor_index),
            kernel_series_ptrs: Some(series_ptrs),
            kernel_state_ptr: Some(state_ptr),
            kernel_config: Some(config),
            loop_info,
            optimization_plan,
            hoisted_locals: HashMap::new(),
            local_f64_cache: HashMap::new(),
            // No inlining in kernel mode (no user functions)
            inline_candidates: HashMap::new(),
            inline_local_base: 0,
            inline_depth: 0,
            // Reference tracking
            ref_stack_slots: HashMap::new(),
            // Integer unboxing
            unboxed_int_locals: std::collections::HashSet::new(),
            unboxed_int_module_bindings: std::collections::HashSet::new(),
            promoted_module_bindings: HashMap::new(),
            register_carried_module_bindings: std::collections::HashSet::new(),
            unboxed_loop_depth: 0,
            unboxed_scope_stack: Vec::new(),
            register_carried_loop_depth: 0,
            pending_rebox: None,
            pending_rebox_module_bindings: None,
            pending_flush_module_bindings: None,
            // Float unboxing
            unboxed_f64_locals: std::collections::HashSet::new(),
            f64_local_vars: HashMap::new(),
            pending_rebox_f64: None,
            precomputed_f64_for_invariant_int: HashMap::new(),
            precomputed_f64_scope_stack: Vec::new(),
            // Skip ranges (empty by default)
            skip_ranges: Vec::new(),
            // Array LICM
            hoisted_array_info: HashMap::new(),
            hoisted_ref_array_info: HashMap::new(),
            // Numeric parameter hints (compile-time)
            numeric_param_hints: std::collections::HashSet::new(),
            deopt_block: None,
            deopt_signal_var: None,
            // Deopt tracking
            deopt_points: Vec::new(),
            func_locals_count: 0,
            deferred_spills: Vec::new(),
            // Loop unrolling
            pending_unroll: None,
            trusted_array_push_local_sites: std::collections::HashSet::new(),
            trusted_array_push_local_iv_by_site: HashMap::new(),
            // Shape guard tracking
            shape_guards_emitted: Vec::new(),
            // Feedback-guided speculation (not used in kernel mode)
            feedback: None,
            // Multi-frame inline deopt (not used in kernel mode)
            compiling_function_id: 0,
            inline_frame_stack: Vec::new(),
        }
    }

    /// Check if an instruction index falls within a skip range.
    fn is_skipped(&self, idx: usize) -> bool {
        self.skip_ranges
            .iter()
            .any(|&(start, end)| idx >= start && idx < end)
    }

    pub(crate) fn compile(&mut self) -> Result<Value, String> {
        // Phase 1: Find all jump targets and create basic blocks
        self.create_blocks_for_jumps();

        // Create an exit block for the epilogue - all paths will jump here
        let exit_block = self.builder.create_block();
        self.builder.append_block_param(exit_block, types::I64);
        self.exit_block = Some(exit_block);

        // Initialize function signal to success (0). Some guarded helper paths
        // may set this to a negative value and jump to exit_block.
        let deopt_signal_var = Variable::new(self.next_var);
        self.next_var += 1;
        self.builder.declare_var(deopt_signal_var, types::I32);
        let zero_i32 = self.builder.ins().iconst(types::I32, 0);
        self.builder.def_var(deopt_signal_var, zero_i32);
        self.deopt_signal_var = Some(deopt_signal_var);

        // Find the first non-skipped instruction and jump from entry to its block.
        let first_idx = (0..self.program.instructions.len())
            .find(|&i| !self.is_skipped(i))
            .unwrap_or(0);
        if let Some(&block0) = self.blocks.get(&first_idx) {
            if !self.numeric_param_hints.is_empty() {
                let mut params: Vec<u16> = self.numeric_param_hints.iter().copied().collect();
                params.sort_unstable();
                for local_idx in params {
                    self.local_types
                        .entry(local_idx)
                        .or_insert(StorageHint::Float64);
                }
            }
            self.builder.ins().jump(block0, &[]);
            self.block_stack_depth.insert(first_idx, 0);
        }

        // Phase 2: Compile instructions with control flow
        let instrs = self.program.instructions.clone();
        let mut need_fallthrough = false;
        let mut block_terminated = false;

        for (i, instr) in instrs.iter().enumerate() {
            // Skip function body instructions (compiled separately)
            if self.is_skipped(i) {
                continue;
            }

            if let Some(&block) = self.blocks.get(&i) {
                if need_fallthrough && !block_terminated {
                    self.block_stack_depth.entry(i).or_insert(self.stack_depth);
                    if self.merge_blocks.contains(&i) {
                        let val = self.stack_pop().unwrap_or_else(|| {
                            self.builder.ins().iconst(types::I64, TAG_NULL as i64)
                        });
                        self.builder.ins().jump(block, &[val]);
                    } else {
                        self.builder.ins().jump(block, &[]);
                    }
                }
                self.builder.switch_to_block(block);
                self.current_block_idx = i;
                need_fallthrough = false;
                block_terminated = false;

                // Integer unboxing: rebox raw i64 locals at loop exit.
                // compile_loop_end sets pending_rebox; the rebox code runs at the
                // start of the loop's end_block (the first block switch after LoopEnd).
                if let Some(rebox_locals) = self.pending_rebox.take() {
                    for &local_idx in &rebox_locals {
                        let var = self.get_or_create_local(local_idx);
                        let raw_int = self.builder.use_var(var);
                        let f64_val = self.builder.ins().fcvt_from_sint(types::F64, raw_int);
                        let boxed = self.f64_to_i64(f64_val);
                        self.builder.def_var(var, boxed);
                    }
                }

                // Float unboxing rebox: convert raw f64 → NaN-boxed i64.
                if let Some(rebox_f64s) = self.pending_rebox_f64.take() {
                    for &local_idx in &rebox_f64s {
                        if let Some(&f64_var) = self.f64_local_vars.get(&local_idx) {
                            let f64_val = self.builder.use_var(f64_var);
                            let boxed = self.f64_to_i64(f64_val);
                            let i64_var = self.get_or_create_local(local_idx);
                            self.builder.def_var(i64_var, boxed);
                        }
                        self.f64_local_vars.remove(&local_idx);
                    }
                    // Only clear all f64 vars when no outer scopes remain
                    if self.unboxed_scope_stack.is_empty() {
                        self.f64_local_vars.clear();
                    }
                }

                // Rebox promoted module bindings: convert raw i64 → NaN-boxed
                // and write back to ctx.locals[] memory.
                if let Some(rebox_mbs) = self.pending_rebox_module_bindings.take() {
                    for &mb_idx in &rebox_mbs {
                        if let Some(&var) = self.promoted_module_bindings.get(&mb_idx) {
                            let raw_int = self.builder.use_var(var);
                            let f64_val = self.builder.ins().fcvt_from_sint(types::F64, raw_int);
                            let boxed = self.f64_to_i64(f64_val);
                            // Write back to memory
                            let byte_offset = LOCALS_OFFSET + (mb_idx as i32 * 8);
                            self.builder.ins().store(
                                MemFlags::new(),
                                boxed,
                                self.ctx_ptr,
                                byte_offset,
                            );
                        }
                        self.promoted_module_bindings.remove(&mb_idx);
                        self.register_carried_module_bindings.remove(&mb_idx);
                    }
                }

                // Flush boxed, register-carried module bindings to ctx.locals[] at loop exit.
                if let Some(flush_mbs) = self.pending_flush_module_bindings.take() {
                    for &mb_idx in &flush_mbs {
                        if let Some(&var) = self.promoted_module_bindings.get(&mb_idx) {
                            let val = self.builder.use_var(var);
                            let byte_offset = LOCALS_OFFSET + (mb_idx as i32 * 8);
                            self.builder.ins().store(
                                MemFlags::new(),
                                val,
                                self.ctx_ptr,
                                byte_offset,
                            );
                        }
                        self.promoted_module_bindings.remove(&mb_idx);
                        self.register_carried_module_bindings.remove(&mb_idx);
                    }
                }

                if let Some(&expected_depth) = self.block_stack_depth.get(&i) {
                    self.stack_depth = expected_depth;
                    // Clear typed_stack at block boundaries: f64 SSA Values from
                    // predecessor blocks may not dominate this block, so cached
                    // shadows are invalid. The optimization still applies within
                    // basic blocks (where tight inner loops live).
                    self.typed_stack.clear();
                    // Clear local_f64_cache: cached f64 Values from predecessor
                    // blocks may not dominate this block.
                    self.local_f64_cache.clear();
                }

                if self.merge_blocks.contains(&i) {
                    let params = self.builder.block_params(block);
                    if !params.is_empty() {
                        self.stack_push(params[0]);
                    }
                }
            }

            if block_terminated {
                continue;
            }

            // Track current instruction index for property lookup in compile_get_prop
            self.current_instr_idx = i;
            self.compile_instruction(instr, i)?;

            match instr.opcode {
                OpCode::Jump
                | OpCode::Return
                | OpCode::ReturnValue
                | OpCode::Break
                | OpCode::Continue
                | OpCode::Throw => {
                    block_terminated = true;
                }
                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue => {
                    block_terminated = true;
                }
                _ => {
                    need_fallthrough = self.blocks.contains_key(&(i + 1));
                }
            }
        }

        if !block_terminated {
            let default_val = self
                .stack_pop_boxed()
                .unwrap_or_else(|| self.builder.ins().iconst(types::I64, TAG_NULL as i64));
            self.builder.ins().jump(exit_block, &[default_val]);
        }

        for block in self.blocks.values() {
            self.builder.seal_block(*block);
        }

        // Emit deferred per-guard spill blocks.
        // Each block stores live locals + operand stack to ctx_buf,
        // then jumps to the shared deopt block with its deopt_id.
        let deferred = std::mem::take(&mut self.deferred_spills);
        for spill in &deferred {
            self.builder.switch_to_block(spill.block);

            // Store live locals to ctx_buf[LOCALS_OFFSET + bc_idx * 8]
            // Unboxed locals need type-aware storage:
            // - f64 locals: bitcast(I64, f64_val) to get raw bits
            // - int locals: store directly (raw i64 fits in u64)
            // - NaN-boxed: store as-is
            for &(bc_idx, var) in &spill.live_locals {
                let val = self.builder.use_var(var);
                let store_val = if spill.f64_locals.contains(&bc_idx) {
                    // Float-unboxed local: val is Cranelift f64, bitcast to i64 bits
                    // Check if this local has an f64 variable
                    if let Some(&f64_var) = self.f64_local_vars.get(&bc_idx) {
                        let f64_val = self.builder.use_var(f64_var);
                        self.builder
                            .ins()
                            .bitcast(types::I64, MemFlags::new(), f64_val)
                    } else {
                        // Fallback: the regular variable holds NaN-boxed, use as-is
                        val
                    }
                } else {
                    // Int-unboxed or NaN-boxed: store directly.
                    // Int locals hold raw i64, which unmarshal_jit_result handles
                    // with SlotKind::Int64 → ValueWord::from_i64(bits as i64).
                    // NaN-boxed locals store as-is (SlotKind::Unknown passthrough).
                    val
                };
                let offset = LOCALS_OFFSET + (bc_idx as i32) * 8;
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), store_val, self.ctx_ptr, offset);
            }

            // Store on-stack operand values (via stack_vars)
            for i in 0..spill.on_stack_count {
                let var = self.get_or_create_stack_var(i);
                let val = self.builder.use_var(var);
                let offset = LOCALS_OFFSET + (128 + i as i32) * 8;
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), val, self.ctx_ptr, offset);
            }

            // Store extra pre-popped values (passed as block params)
            let block_params = self.builder.block_params(spill.block).to_vec();
            for (j, &param) in block_params.iter().enumerate() {
                let stack_pos = spill.on_stack_count + j;
                let offset = LOCALS_OFFSET + (128 + stack_pos as i32) * 8;
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), param, self.ctx_ptr, offset);
            }

            // Store inline frame locals for multi-frame deopt
            let mut ctx_buf_pos = 128u16 + (spill.on_stack_count + spill.extra_param_count) as u16;
            // Use the ctx_buf_positions from the DeoptInfo inline_frames
            for iframe in &spill.inline_frames {
                for &(_, var) in &iframe.live_locals {
                    let val = self.builder.use_var(var);
                    let offset = LOCALS_OFFSET + (ctx_buf_pos as i32) * 8;
                    self.builder
                        .ins()
                        .store(MemFlags::trusted(), val, self.ctx_ptr, offset);
                    ctx_buf_pos += 1;
                }
            }

            // Jump to shared deopt block
            let deopt = self.get_or_create_deopt_block();
            let deopt_id_val = self.builder.ins().iconst(types::I32, spill.deopt_id as i64);
            self.builder.ins().jump(deopt, &[deopt_id_val]);
            self.builder.seal_block(spill.block);
        }

        if let Some(deopt_block) = self.deopt_block {
            self.builder.switch_to_block(deopt_block);
            let deopt_signal_var = self
                .deopt_signal_var
                .expect("deopt_signal_var must be initialized in compile()");
            let deopt_id_i32 = self.builder.block_params(deopt_block)[0];
            let deopt_id_u64 = self.builder.ins().uextend(types::I64, deopt_id_i32);
            // VM deopt handler reads deopt_id from ctx word 0.
            self.builder
                .ins()
                .store(MemFlags::trusted(), deopt_id_u64, self.ctx_ptr, 0);
            let deopt_code = self.builder.ins().iconst(types::I32, (u32::MAX - 1) as i64);
            self.builder.def_var(deopt_signal_var, deopt_code);
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.builder.ins().jump(exit_block, &[null_val]);
            self.builder.seal_block(deopt_block);
        }

        self.builder.switch_to_block(exit_block);
        self.builder.seal_block(exit_block);

        let ret_val_i64 = self.builder.block_params(exit_block)[0];

        self.builder
            .ins()
            .store(MemFlags::trusted(), ret_val_i64, self.ctx_ptr, STACK_OFFSET);

        let one = self.builder.ins().iconst(types::I64, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), one, self.ctx_ptr, STACK_PTR_OFFSET);

        // Return signal (0 success, negative deopt).
        let signal_var = self
            .deopt_signal_var
            .expect("deopt_signal_var must be initialized in compile()");
        let signal = self.builder.use_var(signal_var);
        Ok(signal)
    }

    fn create_blocks_for_jumps(&mut self) {
        let mut block_starts: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut incoming_edges: HashMap<usize, usize> = HashMap::new();

        for (i, instr) in self.program.instructions.iter().enumerate() {
            if self.is_skipped(i) {
                continue;
            }
            match instr.opcode {
                OpCode::Jump => {
                    if let Some(Operand::Offset(offset)) = &instr.operand {
                        let target_idx = ((i as i32) + 1 + *offset) as usize;
                        if !self.is_skipped(target_idx) {
                            block_starts.insert(target_idx);
                            *incoming_edges.entry(target_idx).or_insert(0) += 1;
                        }
                    }
                }
                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue => {
                    if let Some(Operand::Offset(offset)) = &instr.operand {
                        let target_idx = ((i as i32) + 1 + *offset) as usize;
                        if !self.is_skipped(target_idx) {
                            block_starts.insert(target_idx);
                            *incoming_edges.entry(target_idx).or_insert(0) += 1;
                        }
                        let next_idx = i + 1;
                        if !self.is_skipped(next_idx) {
                            block_starts.insert(next_idx);
                            *incoming_edges.entry(next_idx).or_insert(0) += 1;
                        }
                    }
                }
                OpCode::LoopStart | OpCode::LoopEnd => {
                    block_starts.insert(i);
                    *incoming_edges.entry(i).or_insert(0) += 1;
                    let next_idx = i + 1;
                    if next_idx < self.program.instructions.len() && !self.is_skipped(next_idx) {
                        block_starts.insert(next_idx);
                        *incoming_edges.entry(next_idx).or_insert(0) += 1;
                    }
                }
                OpCode::SetupTry => {
                    if let Some(Operand::Offset(offset)) = &instr.operand {
                        let catch_idx = ((i as i32) + 1 + *offset) as usize;
                        if !self.is_skipped(catch_idx) {
                            block_starts.insert(catch_idx);
                            *incoming_edges.entry(catch_idx).or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        // Find the first non-skipped instruction index to use as block 0.
        // When stdlib is prepended, instruction 0 is in a skip range — we must
        // start from the first instruction the JIT will actually compile.
        let first_idx = (0..self.program.instructions.len())
            .find(|&i| !self.is_skipped(i))
            .unwrap_or(0);
        block_starts.insert(first_idx);
        *incoming_edges.entry(first_idx).or_insert(0) += 1;

        for (i, instr) in self.program.instructions.iter().enumerate() {
            if self.is_skipped(i) {
                continue;
            }
            let is_terminator = matches!(
                instr.opcode,
                OpCode::Jump
                    | OpCode::Return
                    | OpCode::ReturnValue
                    | OpCode::Break
                    | OpCode::Continue
                    | OpCode::Throw
            );
            let is_conditional = matches!(
                instr.opcode,
                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue
            );

            if !is_terminator && !is_conditional {
                let next_idx = i + 1;
                if next_idx < self.program.instructions.len()
                    && block_starts.contains(&next_idx)
                    && !self.is_skipped(next_idx)
                {
                    *incoming_edges.entry(next_idx).or_insert(0) += 1;
                }
            }
        }

        if !self.blocks.contains_key(&first_idx) {
            let block = self.builder.create_block();
            self.blocks.insert(first_idx, block);
        }

        for (i, instr) in self.program.instructions.iter().enumerate() {
            if self.is_skipped(i) {
                continue;
            }
            match instr.opcode {
                OpCode::Jump
                | OpCode::JumpIfFalse
                | OpCode::JumpIfFalseTrusted
                | OpCode::JumpIfTrue => {
                    if let Some(Operand::Offset(offset)) = &instr.operand {
                        let target_idx = ((i as i32) + 1 + *offset) as usize;
                        if !self.is_skipped(target_idx) && !self.blocks.contains_key(&target_idx) {
                            let block = self.builder.create_block();
                            let needs_merge_param = false;
                            if needs_merge_param {
                                self.builder.append_block_param(block, types::I64);
                                self.merge_blocks.insert(target_idx);
                            }
                            self.blocks.insert(target_idx, block);
                        }
                    }
                }
                OpCode::LoopStart | OpCode::LoopEnd => {
                    if !self.blocks.contains_key(&i) {
                        let block = self.builder.create_block();
                        self.blocks.insert(i, block);
                    }
                    let next_idx = i + 1;
                    if next_idx < self.program.instructions.len()
                        && !self.is_skipped(next_idx)
                        && !self.blocks.contains_key(&next_idx)
                    {
                        let block = self.builder.create_block();
                        let needs_merge_param = false;
                        if needs_merge_param {
                            self.builder.append_block_param(block, types::I64);
                            self.merge_blocks.insert(next_idx);
                        }
                        self.blocks.insert(next_idx, block);
                    }
                }
                OpCode::SetupTry => {
                    if let Some(Operand::Offset(offset)) = &instr.operand {
                        let catch_idx = ((i as i32) + 1 + *offset) as usize;
                        if !self.is_skipped(catch_idx) && !self.blocks.contains_key(&catch_idx) {
                            let block = self.builder.create_block();
                            self.builder.append_block_param(block, types::I64);
                            self.merge_blocks.insert(catch_idx);
                            self.blocks.insert(catch_idx, block);
                        }
                    }
                }
                _ => {}
            }
            if matches!(
                instr.opcode,
                OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue
            ) {
                let next_idx = i + 1;
                if !self.is_skipped(next_idx) && !self.blocks.contains_key(&next_idx) {
                    let block = self.builder.create_block();
                    let needs_merge_param = false;
                    if needs_merge_param {
                        self.builder.append_block_param(block, types::I64);
                        self.merge_blocks.insert(next_idx);
                    }
                    self.blocks.insert(next_idx, block);
                }
            }
        }
    }

    pub(crate) fn get_or_create_local(&mut self, idx: u16) -> Variable {
        // Apply inline base offset to avoid caller/callee local collisions
        let effective_idx = idx.wrapping_add(self.inline_local_base);
        if let Some(var) = self.locals.get(&effective_idx) {
            return *var;
        }

        let var = Variable::new(self.next_var);
        self.next_var += 1;
        self.builder.declare_var(var, types::I64);
        self.locals.insert(effective_idx, var);
        var
    }

    /// Analyze which functions are eligible for inlining at call sites.
    ///
    /// A function is an inline candidate if:
    /// - It has < 80 bytecode instructions
    /// - It is not a closure (no captured state)
    /// - It does not use CallValue (closure calls need captured state)
    /// - It is straight-line (no jumps, loops, or exception handlers)
    /// Non-leaf functions (with Call/CallMethod/BuiltinCall) ARE allowed.
    pub(crate) fn analyze_inline_candidates(
        program: &BytecodeProgram,
    ) -> HashMap<u16, InlineCandidate> {
        let mut candidates = HashMap::new();
        let num_funcs = program.functions.len();
        if num_funcs == 0 {
            return candidates;
        }

        for (fn_id, func) in program.functions.iter().enumerate() {
            let fn_id = fn_id as u16;

            // Skip closures — they have captured state
            if func.is_closure || func.body_length == 0 {
                continue;
            }

            let entry_point = func.entry_point;
            let func_end = entry_point + func.body_length;
            let instr_count = func.body_length;

            // Skip if too large or out of bounds
            if instr_count > 80 || instr_count == 0 {
                continue;
            }
            if entry_point >= program.instructions.len() || func_end > program.instructions.len() {
                continue;
            }

            let body = &program.instructions[entry_point..func_end];

            // Allow non-leaf functions (functions that call other functions).
            // Nested calls are handled by compile_call which respects inline_depth.
            // Only exclude CallValue (closure calls need captured state management
            // that may not be set up correctly in the inline namespace).
            let has_closure_calls = body.iter().any(|i| matches!(i.opcode, OpCode::CallValue));
            if has_closure_calls {
                continue;
            }

            // Must be straight-line (no branches, loops, exception handling,
            // or reference operations that create internal blocks)
            let has_control_flow = body.iter().any(|i| {
                matches!(
                    i.opcode,
                    OpCode::Jump
                        | OpCode::JumpIfFalse
                        | OpCode::JumpIfTrue
                        | OpCode::LoopStart
                        | OpCode::LoopEnd
                        | OpCode::Break
                        | OpCode::Continue
                        | OpCode::SetupTry
                        | OpCode::SetIndexRef  // Creates 4 internal blocks — cannot inline
                        | OpCode::MakeRef      // Creates stack slots + ref tracking
                        | OpCode::DerefLoad    // Reference dereference
                        | OpCode::DerefStore // Reference write-through
                )
            });
            if has_control_flow {
                continue;
            }

            candidates.insert(
                fn_id,
                InlineCandidate {
                    entry_point,
                    instruction_count: instr_count,
                    arity: func.arity,
                    locals_count: func.locals_count,
                },
            );
        }

        candidates
    }

    /// Compile bytecode to kernel IR (simplified linear compilation).
    ///
    /// Kernel mode uses a simplified compilation path:
    /// - Linear instruction stream (no complex control flow for V1)
    /// - Returns i32 result code (0 = continue, 1 = done, negative = error)
    /// - All data access goes through kernel_series_ptrs/kernel_state_ptr
    /// Record a deopt point for a non-speculative guard (shape guards,
    /// signal propagation, etc.).
    ///
    /// For speculative guards (arithmetic, property, call), prefer
    /// `emit_deopt_point_with_spill()` which creates a per-guard spill
    /// block that stores live locals and operand stack values to ctx_buf,
    /// enabling the VM to resume execution at the exact guard failure
    /// point instead of re-executing from function entry.
    ///
    /// `bytecode_ip` is sub-program-local (0-based within the function
    /// slice); the caller in `compile_optimizing_function` rebases it
    /// to global program IP after `take_deopt_points()`.
    ///
    /// # Returns
    /// Stable deopt point id (index into `deopt_points`) for this guard site.
    pub(crate) fn emit_deopt_point(
        &mut self,
        bytecode_ip: usize,
        live_locals: &[u16],
        local_kinds: &[SlotKind],
    ) -> usize {
        let deopt_id = self.deopt_points.len();
        let deopt_info = DeoptInfo {
            resume_ip: bytecode_ip,
            local_mapping: live_locals
                .iter()
                .enumerate()
                .map(|(jit_idx, &bc_idx)| (jit_idx as u16, bc_idx))
                .collect(),
            local_kinds: local_kinds.to_vec(),
            stack_depth: 0, // Filled by caller if needed
            innermost_function_id: None,
            inline_frames: Vec::new(),
        };
        self.deopt_points.push(deopt_info);
        deopt_id
    }

    /// Record a deopt point with a per-guard spill block.
    ///
    /// Creates a dedicated Cranelift block that stores all live locals
    /// and operand stack values to ctx_buf, then jumps to the shared
    /// deopt block. The returned `(deopt_id, spill_block)` tuple lets
    /// the caller emit `brif(cond, cont, [], spill_block, [extra_vals])`.
    ///
    /// `extra_stack_values`: Cranelift Values that were popped from the
    /// JIT operand stack before the guard but must be on the interpreter
    /// stack at resume. Passed as block parameters to the spill block.
    ///
    /// Handles unboxed int/f64 locals (marks them with proper SlotKind)
    /// and multi-frame inline deopt (captures caller frame state).
    pub(crate) fn emit_deopt_point_with_spill(
        &mut self,
        bytecode_ip: usize,
        extra_stack_values: &[Value],
    ) -> (usize, Option<Block>) {
        let locals_count = self.func_locals_count;

        // Snapshot live locals for the innermost (current) frame.
        // When inlining, locals use inline_local_base offset keys.
        let inline_base = self.inline_local_base;
        let live_locals: Vec<(u16, Variable)> = self
            .locals
            .iter()
            .filter(|(idx, _)| {
                if inline_base > 0 {
                    // Inlined frame: only include locals in the current inline namespace
                    **idx >= inline_base && **idx < inline_base + 128
                } else {
                    **idx < 128 // cap at DEOPT_STACK_CTX_BASE
                }
            })
            .map(|(idx, var)| {
                // Map back to bytecode-local index (subtract inline base)
                let bc_idx = idx.wrapping_sub(inline_base);
                (bc_idx, *var)
            })
            .collect();

        // Determine SlotKind for each local based on unboxing state
        let local_kinds: Vec<SlotKind> = live_locals
            .iter()
            .map(|&(bc_idx, _)| {
                if self.unboxed_int_locals.contains(&bc_idx) {
                    SlotKind::Int64
                } else if self.unboxed_f64_locals.contains(&bc_idx) {
                    SlotKind::Float64
                } else {
                    SlotKind::NanBoxed // boxed local: NaN-boxed passthrough
                }
            })
            .collect();

        // Track which locals are unboxed for the spill emission
        let f64_locals: std::collections::HashSet<u16> = live_locals
            .iter()
            .filter(|&&(bc_idx, _)| self.unboxed_f64_locals.contains(&bc_idx))
            .map(|&(bc_idx, _)| bc_idx)
            .collect();
        let int_locals: std::collections::HashSet<u16> = live_locals
            .iter()
            .filter(|&&(bc_idx, _)| self.unboxed_int_locals.contains(&bc_idx))
            .map(|&(bc_idx, _)| bc_idx)
            .collect();

        let on_stack_count = self.stack_depth;
        let extra_count = extra_stack_values.len();
        let total_stack_depth = on_stack_count + extra_count;

        // Build DeoptInfo with real data
        let deopt_id = self.deopt_points.len();
        let mut local_mapping = Vec::new();
        let mut all_kinds = Vec::new();

        // Locals: identity mapping (ctx_buf_position, bytecode_local_idx)
        for (i, &(bc_idx, _)) in live_locals.iter().enumerate() {
            local_mapping.push((bc_idx, bc_idx));
            all_kinds.push(local_kinds[i]);
        }
        // Operand stack: (DEOPT_STACK_CTX_BASE + i, locals_count + i)
        // Stack values are always NaN-boxed.
        for i in 0..total_stack_depth {
            local_mapping.push((128 + i as u16, locals_count + i as u16));
            all_kinds.push(SlotKind::NanBoxed); // operand stack: NaN-boxed passthrough
        }

        // Build inline_frames for multi-frame deopt
        let mut inline_frames = Vec::new();
        let mut deferred_inline_frames = Vec::new();
        if self.inline_depth > 0 {
            // Capture caller frame(s) from the inline_frame_stack.
            // inline_frame_stack is ordered outermost-first; DeoptInfo uses
            // the same outermost-first ordering ([0]=outermost physical function).
            let mut ctx_buf_offset = live_locals.len() as u16 + total_stack_depth as u16 + 128;
            for ictx in self.inline_frame_stack.iter() {
                let frame_mapping: Vec<(u16, u16)> = ictx
                    .locals_snapshot
                    .iter()
                    .enumerate()
                    .map(|(j, &(bc_idx, _))| {
                        let ctx_pos = ctx_buf_offset + j as u16;
                        (ctx_pos, bc_idx)
                    })
                    .collect();
                let frame_kinds = ictx.local_kinds.clone();

                inline_frames.push(InlineFrameInfo {
                    function_id: ictx.function_id,
                    resume_ip: ictx.call_site_ip,
                    local_mapping: frame_mapping,
                    local_kinds: frame_kinds.clone(),
                    stack_depth: ictx.stack_depth as u16,
                });

                deferred_inline_frames.push(super::types::DeferredInlineFrame {
                    live_locals: ictx.locals_snapshot.clone(),
                    local_kinds: frame_kinds,
                    f64_locals: ictx.f64_locals.clone(),
                    int_locals: ictx.int_locals.clone(),
                });

                ctx_buf_offset += ictx.locals_snapshot.len() as u16;
            }
        }

        // For multi-frame deopt, record the innermost (inlined callee) function ID
        // so the VM can push a synthetic frame for it.
        let innermost_function_id = if self.inline_depth > 0 {
            // The last entry on inline_frame_stack is the immediate caller.
            // The callee_fn_id of that entry is the function where the guard fired.
            self.inline_frame_stack.last().map(|ctx| ctx.callee_fn_id)
        } else {
            None
        };

        self.deopt_points.push(DeoptInfo {
            resume_ip: bytecode_ip,
            local_mapping,
            local_kinds: all_kinds,
            stack_depth: total_stack_depth as u16,
            innermost_function_id,
            inline_frames,
        });

        // Create per-guard spill block with block params for extra values
        let spill_block = self.builder.create_block();
        for _ in 0..extra_count {
            self.builder.append_block_param(spill_block, types::I64);
        }

        // Defer the spill block body emission to compile() epilogue
        self.deferred_spills.push(super::types::DeferredSpill {
            block: spill_block,
            deopt_id: deopt_id as u32,
            live_locals: live_locals.clone(),
            local_kinds,
            on_stack_count,
            extra_param_count: extra_count,
            f64_locals,
            int_locals,
            inline_frames: deferred_inline_frames,
        });

        (deopt_id, Some(spill_block))
    }

    /// Return the deopt points accumulated during compilation.
    ///
    /// This transfers ownership of the collected deopt metadata out of the
    /// compiler so it can be attached to the compilation result.
    pub(crate) fn take_deopt_points(&mut self) -> Vec<DeoptInfo> {
        std::mem::take(&mut self.deopt_points)
    }

    /// Verify deopt point metadata for consistency.
    ///
    /// Checks:
    /// - `local_mapping` and `local_kinds` have equal length
    /// - Unboxed locals are NOT tagged as `SlotKind::Unknown`
    /// - ctx_buf positions are within bounds
    ///
    /// Returns `Err` on validation failure, causing the JIT compile to abort
    /// and the function to fall back to the interpreter.
    pub(crate) fn verify_deopt_points(
        points: &[DeoptInfo],
        unboxed_ints: &std::collections::HashSet<u16>,
        unboxed_f64s: &std::collections::HashSet<u16>,
    ) -> Result<(), String> {
        // VM ctx_buf is 216 u64 words with locals starting at offset 8.
        // Max ctx_pos before overflow: 216 - 8 = 208.
        const CTX_BUF_LOCALS_MAX: u16 = 208;

        for (i, dp) in points.iter().enumerate() {
            if dp.local_mapping.len() != dp.local_kinds.len() {
                return Err(format!(
                    "DeoptInfo[{}]: local_mapping len {} != local_kinds len {}",
                    i,
                    dp.local_mapping.len(),
                    dp.local_kinds.len()
                ));
            }
            // Skip empty deopt points (generic fallback)
            if dp.local_mapping.is_empty() {
                continue;
            }
            for (j, &(ctx_pos, bc_idx)) in dp.local_mapping.iter().enumerate() {
                let kind = dp.local_kinds[j];

                // ctx_buf bounds check: ctx_pos must fit within VM's ctx_buf
                if ctx_pos >= CTX_BUF_LOCALS_MAX {
                    return Err(format!(
                        "DeoptInfo[{}] mapping[{}]: ctx_pos {} exceeds ctx_buf limit {}",
                        i, j, ctx_pos, CTX_BUF_LOCALS_MAX
                    ));
                }

                // Precise deopt points must not use SlotKind::Unknown.
                // Boxed locals → NanBoxed, unboxed int → Int64, unboxed f64 → Float64.
                if kind == SlotKind::Unknown {
                    return Err(format!(
                        "DeoptInfo[{}] mapping[{}]: slot (ctx_pos={}, bc_idx={}) tagged as Unknown \
                         in precise deopt path — use NanBoxed, Int64, or Float64",
                        i, j, ctx_pos, bc_idx
                    ));
                }

                // Unboxed int locals must be tagged Int64
                if unboxed_ints.contains(&bc_idx) && ctx_pos < 128 && kind != SlotKind::Int64 {
                    return Err(format!(
                        "DeoptInfo[{}] mapping[{}]: unboxed int local {} tagged as {:?}, expected Int64",
                        i, j, bc_idx, kind
                    ));
                }
                // Unboxed f64 locals must be tagged Float64
                if unboxed_f64s.contains(&bc_idx) && ctx_pos < 128 && kind != SlotKind::Float64 {
                    return Err(format!(
                        "DeoptInfo[{}] mapping[{}]: unboxed f64 local {} tagged as {:?}, expected Float64",
                        i, j, bc_idx, kind
                    ));
                }
            }

            // Also verify inline frames
            for (fi, iframe) in dp.inline_frames.iter().enumerate() {
                if iframe.local_mapping.len() != iframe.local_kinds.len() {
                    return Err(format!(
                        "DeoptInfo[{}].inline_frames[{}]: local_mapping len {} != local_kinds len {}",
                        i,
                        fi,
                        iframe.local_mapping.len(),
                        iframe.local_kinds.len()
                    ));
                }
                for (j, &(ctx_pos, bc_idx)) in iframe.local_mapping.iter().enumerate() {
                    if ctx_pos >= CTX_BUF_LOCALS_MAX {
                        return Err(format!(
                            "DeoptInfo[{}].inline_frames[{}] mapping[{}]: ctx_pos {} exceeds ctx_buf limit {}",
                            i, fi, j, ctx_pos, CTX_BUF_LOCALS_MAX
                        ));
                    }
                    let kind = iframe
                        .local_kinds
                        .get(j)
                        .copied()
                        .unwrap_or(SlotKind::Unknown);
                    if kind == SlotKind::Unknown {
                        return Err(format!(
                            "DeoptInfo[{}].inline_frames[{}] mapping[{}]: slot (ctx_pos={}, bc_idx={}) \
                             tagged as Unknown in precise path",
                            i, fi, j, ctx_pos, bc_idx
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Attach a feedback vector snapshot for feedback-guided speculation.
    ///
    /// When set, the compiler consults IC feedback at each eligible bytecode
    /// site (call, property access, arithmetic) to emit speculative guards
    /// with typed fast paths. Guard failures branch to the deopt block.
    pub(crate) fn set_feedback(&mut self, feedback: FeedbackVector) {
        self.feedback = Some(feedback);
    }

    /// Return the shape guard IDs accumulated during compilation.
    ///
    /// These should be registered as shape dependencies with the DeoptTracker
    /// so that shape transitions can invalidate stale JIT code.
    pub(crate) fn take_shape_guards(&mut self) -> Vec<shape_value::shape_graph::ShapeId> {
        std::mem::take(&mut self.shape_guards_emitted)
    }

    pub fn compile_kernel(&mut self) -> Result<Value, String> {
        assert!(
            self.mode == CompilationMode::Kernel,
            "compile_kernel() requires kernel mode"
        );

        // Kernel mode: simple linear instruction stream
        // For V1, we don't support complex control flow in kernels
        let instrs = self.program.instructions.clone();
        for (idx, instr) in instrs.iter().enumerate() {
            self.current_instr_idx = idx;
            self.compile_instruction(instr, idx)?;
        }

        // Return result: 0 (continue) or value from stack converted to i32
        let result = if self.stack_depth > 0 {
            let val = self.stack_pop().unwrap();
            // Convert NaN-boxed value to i32 result code
            // If it's a number, truncate to i32; otherwise return 0
            self.builder.ins().ireduce(types::I32, val)
        } else {
            self.builder.ins().iconst(types::I32, 0)
        };

        Ok(result)
    }
}

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;
