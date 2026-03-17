//! Program compilation with multiple functions

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};
use std::collections::{BTreeMap, HashMap};

use super::setup::JITCompiler;
use crate::context::{JittedFn, JittedStrategyFn};
use crate::mixed_table::{FunctionEntry, MixedFunctionTable};
use crate::numeric_compiler::compile_numeric_program;
use crate::translator::BytecodeToIR;
use crate::translator::types::InlineCandidate;
use shape_vm::bytecode::{BytecodeProgram, OpCode, Operand};

#[derive(Default)]
struct NumericOpcodeStats {
    typed: usize,
    generic: usize,
    typed_breakdown: BTreeMap<String, usize>,
    generic_breakdown: BTreeMap<String, usize>,
}

fn bump_breakdown(map: &mut BTreeMap<String, usize>, opcode: OpCode) {
    let key = format!("{:?}", opcode);
    *map.entry(key).or_insert(0) += 1;
}

fn collect_numeric_opcode_stats(program: &BytecodeProgram) -> NumericOpcodeStats {
    let mut stats = NumericOpcodeStats::default();
    for instr in &program.instructions {
        match instr.opcode {
            // Typed arithmetic opcodes
            OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber
            // Typed comparisons
            | OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::EqInt
            | OpCode::EqNumber
            | OpCode::NeqInt
            | OpCode::NeqNumber => {
                stats.typed += 1;
                bump_breakdown(&mut stats.typed_breakdown, instr.opcode);
            }
            // Generic arithmetic/comparison opcodes
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::Eq
            | OpCode::Neq => {
                stats.generic += 1;
                bump_breakdown(&mut stats.generic_breakdown, instr.opcode);
            }
            _ => {}
        }
    }
    stats
}

fn maybe_emit_numeric_metrics(program: &BytecodeProgram) {
    if std::env::var_os("SHAPE_JIT_METRICS").is_none() {
        return;
    }
    let static_stats = collect_numeric_opcode_stats(program);
    let static_total = static_stats.typed + static_stats.generic;
    let static_coverage_pct = if static_total == 0 {
        100.0
    } else {
        (static_stats.typed as f64 * 100.0) / (static_total as f64)
    };
    // Report effective coverage conservatively: generic opcodes remain generic
    // unless the frontend/runtime has concretely emitted typed variants.
    let effective_typed = static_stats.typed;
    let effective_generic = static_stats.generic;
    let effective_coverage_pct = static_coverage_pct;
    eprintln!(
        "[shape-jit-metrics] typed_numeric_ops={} generic_numeric_ops={} typed_numeric_coverage_pct={:.2} static_typed_numeric_ops={} static_generic_numeric_ops={} static_typed_numeric_coverage_pct={:.2}",
        effective_typed,
        effective_generic,
        effective_coverage_pct,
        static_stats.typed,
        static_stats.generic,
        static_coverage_pct
    );
    if std::env::var_os("SHAPE_JIT_METRICS_DETAIL").is_some() {
        let fmt_breakdown = |breakdown: &BTreeMap<String, usize>| -> String {
            breakdown
                .iter()
                .map(|(name, count)| format!("{}:{}", name, count))
                .collect::<Vec<_>>()
                .join(",")
        };
        eprintln!(
            "[shape-jit-metrics-detail] typed_breakdown={} generic_breakdown={}",
            fmt_breakdown(&static_stats.typed_breakdown),
            fmt_breakdown(&static_stats.generic_breakdown)
        );
    }
}

fn stack_effect_for_param_analysis(op: OpCode) -> Option<(i32, i32)> {
    let effect = match op {
        OpCode::LoadLocal
        | OpCode::LoadModuleBinding
        | OpCode::LoadClosure
        | OpCode::PushConst
        | OpCode::PushNull
        | OpCode::DerefLoad => (0, 1),
        OpCode::IntToNumber | OpCode::NumberToInt | OpCode::Neg | OpCode::Not => (1, 1),
        OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Div
        | OpCode::Mod
        | OpCode::Pow
        | OpCode::AddInt
        | OpCode::SubInt
        | OpCode::MulInt
        | OpCode::DivInt
        | OpCode::ModInt
        | OpCode::PowInt
        | OpCode::AddNumber
        | OpCode::SubNumber
        | OpCode::MulNumber
        | OpCode::DivNumber
        | OpCode::ModNumber
        | OpCode::PowNumber
        | OpCode::Gt
        | OpCode::Lt
        | OpCode::Gte
        | OpCode::Lte
        | OpCode::Eq
        | OpCode::Neq
        | OpCode::GtInt
        | OpCode::LtInt
        | OpCode::GteInt
        | OpCode::LteInt
        | OpCode::GtNumber
        | OpCode::LtNumber
        | OpCode::GteNumber
        | OpCode::LteNumber
        | OpCode::EqInt
        | OpCode::EqNumber
        | OpCode::NeqInt
        | OpCode::NeqNumber
        | OpCode::GetProp => (2, 1),
        OpCode::Dup => (1, 2),
        OpCode::Swap => (2, 2),
        OpCode::StoreLocal | OpCode::StoreLocalTyped | OpCode::Pop => (1, 0),
        _ => return None,
    };
    Some(effect)
}

fn source_local_for_stack_pos(
    program: &BytecodeProgram,
    before_idx: usize,
    mut pos_from_top: i32,
) -> Option<u16> {
    for j in (0..before_idx).rev() {
        let instr = &program.instructions[j];
        let (pops, pushes) = stack_effect_for_param_analysis(instr.opcode)?;
        if pos_from_top < pushes {
            return match instr.opcode {
                OpCode::LoadLocal => match &instr.operand {
                    Some(Operand::Local(idx)) => Some(*idx),
                    _ => None,
                },
                _ => None,
            };
        }
        pos_from_top = pos_from_top - pushes + pops;
        if pos_from_top < 0 {
            return None;
        }
    }
    None
}

fn collect_numeric_param_hints(
    program: &BytecodeProgram,
    arity: u16,
    ref_params: &[bool],
) -> std::collections::HashSet<u16> {
    let mut params = std::collections::HashSet::new();
    for (i, instr) in program.instructions.iter().enumerate() {
        let is_numeric_consumer = matches!(
            instr.opcode,
            OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Div
                | OpCode::Mod
                | OpCode::Pow
                | OpCode::AddInt
                | OpCode::SubInt
                | OpCode::MulInt
                | OpCode::DivInt
                | OpCode::ModInt
                | OpCode::PowInt
                | OpCode::AddNumber
                | OpCode::SubNumber
                | OpCode::MulNumber
                | OpCode::DivNumber
                | OpCode::ModNumber
                | OpCode::PowNumber
                | OpCode::Gt
                | OpCode::Lt
                | OpCode::Gte
                | OpCode::Lte
                | OpCode::Eq
                | OpCode::Neq
                | OpCode::GtInt
                | OpCode::LtInt
                | OpCode::GteInt
                | OpCode::LteInt
                | OpCode::GtNumber
                | OpCode::LtNumber
                | OpCode::GteNumber
                | OpCode::LteNumber
                | OpCode::EqInt
                | OpCode::EqNumber
                | OpCode::NeqInt
                | OpCode::NeqNumber
        );
        if !is_numeric_consumer {
            continue;
        }
        for pos in 0..2 {
            if let Some(local_idx) = source_local_for_stack_pos(program, i, pos) {
                if local_idx >= arity {
                    continue;
                }
                let is_ref = ref_params.get(local_idx as usize).copied().unwrap_or(false);
                if !is_ref {
                    params.insert(local_idx);
                }
            }
        }
    }
    params
}

impl JITCompiler {
    #[inline(always)]
    pub fn compile(&mut self, name: &str, program: &BytecodeProgram) -> Result<JittedFn, String> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));

        let func_id = self
            .module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let stack_ptr = builder.block_params(entry_block)[0];
            let constants_ptr = builder.block_params(entry_block)[1];

            let result = compile_numeric_program(&mut builder, program, stack_ptr, constants_ptr)?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function: {}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {}", e))?;

        let code_ptr = self.module.get_finalized_function(func_id);
        self.compiled_functions.insert(name.to_string(), code_ptr);

        Ok(unsafe { std::mem::transmute(code_ptr) })
    }

    #[inline(always)]
    pub fn compile_program(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
    ) -> Result<JittedStrategyFn, String> {
        maybe_emit_numeric_metrics(program);

        let mut user_func_arities: HashMap<u16, u16> = HashMap::new();
        let mut user_func_ids: HashMap<u16, cranelift_module::FuncId> = HashMap::new();

        for (idx, func) in program.functions.iter().enumerate() {
            let func_name = format!("{}_{}", name, func.name);
            let mut user_sig = self.module.make_signature();
            user_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
            for _ in 0..func.arity {
                user_sig.params.push(AbiParam::new(types::I64));
            }
            user_sig.returns.push(AbiParam::new(types::I32));
            let func_id = self
                .module
                .declare_function(&func_name, Linkage::Local, &user_sig)
                .map_err(|e| format!("Failed to pre-declare function {}: {}", func.name, e))?;
            user_func_ids.insert(idx as u16, func_id);
            user_func_arities.insert(idx as u16, func.arity);
        }

        let main_func_id = self.compile_strategy_with_user_funcs(
            name,
            program,
            &user_func_ids,
            &user_func_arities,
        )?;

        for (idx, func) in program.functions.iter().enumerate() {
            let func_name = format!("{}_{}", name, func.name);
            self.compile_function_with_user_funcs(
                &func_name,
                program,
                idx,
                &user_func_ids,
                &user_func_arities,
            )?;
        }

        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize definitions: {:?}", e))?;

        let main_code_ptr = self.module.get_finalized_function(main_func_id);
        self.compiled_functions
            .insert(name.to_string(), main_code_ptr);

        self.function_table.clear();
        for (idx, func) in program.functions.iter().enumerate() {
            let func_name = format!("{}_{}", name, func.name);
            if let Some(&func_id) = user_func_ids.get(&(idx as u16)) {
                let ptr = self.module.get_finalized_function(func_id);
                while self.function_table.len() <= idx {
                    self.function_table.push(std::ptr::null());
                }
                self.function_table[idx] = ptr;
                self.compiled_functions.insert(func_name, ptr);
            }
        }

        Ok(unsafe { std::mem::transmute(main_code_ptr) })
    }

    fn compile_function_with_user_funcs(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
        func_idx: usize,
        user_func_ids: &HashMap<u16, cranelift_module::FuncId>,
        user_func_arities: &HashMap<u16, u16>,
    ) -> Result<(), String> {
        let func = &program.functions[func_idx];
        let func_id = *user_func_ids
            .get(&(func_idx as u16))
            .ok_or_else(|| format!("Function {} not pre-declared", name))?;

        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        for _ in 0..func.arity {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I32));

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

            let func_end = func.entry_point + func.body_length;
            let sub_instructions = &program.instructions[func.entry_point..func_end];
            let sub_program = BytecodeProgram {
                instructions: sub_instructions.to_vec(),
                constants: program.constants.clone(),
                strings: program.strings.clone(),
                // Use empty functions list: the sub_program only contains ONE function's
                // body, so the original entry points are meaningless in the rebased index
                // space. This prevents analyze_inline_candidates from using wrong instruction
                // ranges. Direct calls between functions use user_func_refs instead.
                functions: Vec::new(),
                debug_info: Default::default(),
                data_schema: program.data_schema.clone(),
                module_binding_names: program.module_binding_names.clone(),
                top_level_locals_count: program.top_level_locals_count,
                top_level_local_storage_hints: program
                    .function_local_storage_hints
                    .get(func_idx)
                    .cloned()
                    .unwrap_or_default(),
                type_schema_registry: program.type_schema_registry.clone(),
                module_binding_storage_hints: program.module_binding_storage_hints.clone(),
                function_local_storage_hints: Vec::new(),
                top_level_frame: None,
                compiled_annotations: program.compiled_annotations.clone(),
                trait_method_symbols: program.trait_method_symbols.clone(),
                expanded_function_defs: program.expanded_function_defs.clone(),
                string_index: Default::default(),
                foreign_functions: program.foreign_functions.clone(),
                native_struct_layouts: program.native_struct_layouts.clone(),
                content_addressed: None,
                function_blob_hashes: Vec::new(),
            };

            let mut compiler = BytecodeToIR::new(
                &mut builder,
                &sub_program,
                ctx_ptr,
                ffi,
                user_func_refs,
                user_func_arities.clone(),
            );
            compiler.numeric_param_hints =
                collect_numeric_param_hints(&sub_program, func.arity, &func.ref_params);

            let entry_params = compiler.builder.block_params(entry_block).to_vec();
            for arg_idx in 0..func.arity {
                let arg_val = entry_params[(arg_idx as usize) + 1];
                let var = compiler.get_or_create_local(arg_idx);
                compiler.builder.def_var(var, arg_val);
            }

            let result = compiler.compile()?;
            builder.ins().return_(&[result]);
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function: {:?}", e))?;

        self.module.clear_context(&mut ctx);

        Ok(())
    }

    /// Compile a single function for Tier 1 whole-function JIT.
    ///
    /// ABI matches JitFnPtr: `extern "C" fn(*mut u8, *const u8) -> u64`
    /// - param 0 (i64): ctx_ptr — pointer to a JITContext-shaped buffer
    /// - param 1 (i64): unused (kept for ABI compatibility with OSR)
    /// - return (i64): NaN-boxed result value, or u64::MAX for deopt
    ///
    /// Args are loaded from the ctx locals area at LOCALS_U64_OFFSET.
    /// Cross-function calls deopt to interpreter (empty user_funcs).
    ///
    /// Returns `(code_ptr, deopt_points, shape_guards)` on success.
    pub fn compile_single_function(
        &mut self,
        program: &BytecodeProgram,
        func_index: usize,
        feedback: Option<shape_vm::feedback::FeedbackVector>,
    ) -> Result<
        (
            *const u8,
            Vec<shape_vm::bytecode::DeoptInfo>,
            Vec<shape_value::shape_graph::ShapeId>,
        ),
        String,
    > {
        use cranelift_module::{Linkage, Module};

        let func = program
            .functions
            .get(func_index)
            .ok_or_else(|| format!("Function {} not found in program", func_index))?;

        let func_name = format!("tier1_fn_{}", func.name);

        // JitFnPtr ABI: (ctx_ptr: i64, unused: i64) -> i64
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        sig.params.push(AbiParam::new(types::I64)); // unused
        sig.returns.push(AbiParam::new(types::I64)); // NaN-boxed result

        let cr_func_id = self
            .module
            .declare_function(&func_name, Linkage::Export, &sig)
            .map_err(|e| format!("Failed to declare function: {}", e))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut deopt_points;
        let shape_guards;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let ctx_ptr = builder.block_params(entry_block)[0];
            // param 1 is unused

            let ffi = self.build_ffi_refs(&mut builder);

            // Compute function body range: next function's entry point, or
            // end of instruction stream if this is the last/only function.
            let func_end = program
                .functions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != func_index)
                .filter_map(|(_, f)| {
                    if f.entry_point > func.entry_point {
                        Some(f.entry_point)
                    } else {
                        None
                    }
                })
                .min()
                .unwrap_or(program.instructions.len());

            let sub_instructions = &program.instructions[func.entry_point..func_end];
            let sub_program = BytecodeProgram {
                instructions: sub_instructions.to_vec(),
                constants: program.constants.clone(),
                strings: program.strings.clone(),
                functions: Vec::new(),
                debug_info: Default::default(),
                data_schema: program.data_schema.clone(),
                module_binding_names: program.module_binding_names.clone(),
                top_level_locals_count: program.top_level_locals_count,
                top_level_local_storage_hints: program
                    .function_local_storage_hints
                    .get(func_index)
                    .cloned()
                    .unwrap_or_default(),
                type_schema_registry: program.type_schema_registry.clone(),
                module_binding_storage_hints: program.module_binding_storage_hints.clone(),
                function_local_storage_hints: Vec::new(),
                top_level_frame: None,
                compiled_annotations: program.compiled_annotations.clone(),
                trait_method_symbols: program.trait_method_symbols.clone(),
                expanded_function_defs: program.expanded_function_defs.clone(),
                string_index: Default::default(),
                foreign_functions: program.foreign_functions.clone(),
                native_struct_layouts: program.native_struct_layouts.clone(),
                content_addressed: None,
                function_blob_hashes: Vec::new(),
            };

            // Populate function arities for arity validation and speculation hints.
            // user_funcs (FuncRef map) stays empty: single-function compilation
            // can't pre-declare other functions in the Cranelift module without
            // defining them (which requires compiling all functions together, as
            // compile_program does). Speculative direct calls therefore deopt to
            // the interpreter. Speculative arithmetic works (only needs feedback).
            let user_func_arities: HashMap<u16, u16> = program
                .functions
                .iter()
                .enumerate()
                .map(|(i, f)| (i as u16, f.arity))
                .collect();
            let mut compiler = BytecodeToIR::new(
                &mut builder,
                &sub_program,
                ctx_ptr,
                ffi,
                HashMap::new(),
                user_func_arities,
            );
            compiler.numeric_param_hints =
                collect_numeric_param_hints(&sub_program, func.arity, &func.ref_params);
            compiler.func_locals_count = func.locals_count;

            // Attach feedback vector for Tier 2 speculation if available.
            // Rebase slot keys: interpreter records at absolute IP, but the
            // sub-program starts at index 0.
            if let Some(mut fv) = feedback {
                fv.rebase(func.entry_point);
                compiler.set_feedback(fv);
            }

            // Load args from JITContext locals area.
            // The executor marshals args into ctx_buf[LOCALS_U64_OFFSET + i].
            const LOCALS_BYTE_OFFSET: i32 = 64; // byte 64 in JITContext
            for arg_idx in 0..func.arity {
                let offset = LOCALS_BYTE_OFFSET + (arg_idx as i32) * 8;
                let arg_val =
                    compiler
                        .builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), ctx_ptr, offset);
                let var = compiler.get_or_create_local(arg_idx);
                compiler.builder.def_var(var, arg_val);
            }

            let signal = compiler.compile()?;
            deopt_points = compiler.take_deopt_points();
            // Rebase deopt resume_ip from sub-program-local to global program IP.
            for dp in &mut deopt_points {
                dp.resume_ip += func.entry_point;
            }
            shape_guards = compiler.take_shape_guards();
            // Drop compiler to release the mutable borrow on builder
            drop(compiler);

            // compile() returns an i32 signal (0 = success, negative = deopt)
            // and stores the NaN-boxed result in ctx[STACK_OFFSET].
            // For JitFnPtr ABI, we need to return:
            //   - u64::MAX on deopt (signal < 0)
            //   - NaN-boxed result (loaded from ctx stack) on success
            use crate::context::STACK_OFFSET;
            let zero = builder.ins().iconst(types::I32, 0);
            let is_deopt = builder.ins().icmp(IntCC::SignedLessThan, signal, zero);

            let deopt_block = builder.create_block();
            let success_block = builder.create_block();
            let merge_block = builder.create_block();
            builder.append_block_param(merge_block, types::I64);

            builder
                .ins()
                .brif(is_deopt, deopt_block, &[], success_block, &[]);

            // Deopt path: return u64::MAX
            builder.switch_to_block(deopt_block);
            builder.seal_block(deopt_block);
            let deopt_sentinel = builder.ins().iconst(types::I64, -1i64); // u64::MAX
            builder.ins().jump(merge_block, &[deopt_sentinel]);

            // Success path: load result from ctx stack
            builder.switch_to_block(success_block);
            builder.seal_block(success_block);
            let result_val =
                builder
                    .ins()
                    .load(types::I64, MemFlags::trusted(), ctx_ptr, STACK_OFFSET);
            builder.ins().jump(merge_block, &[result_val]);

            // Merge: return the selected value
            builder.switch_to_block(merge_block);
            builder.seal_block(merge_block);
            let ret_val = builder.block_params(merge_block)[0];
            builder.ins().return_(&[ret_val]);
            builder.finalize();
        }

        self.module
            .define_function(cr_func_id, &mut ctx)
            .map_err(|e| format!("Failed to define function: {:?}", e))?;

        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {:?}", e))?;

        let code_ptr = self.module.get_finalized_function(cr_func_id);
        self.compiled_functions.insert(func_name, code_ptr);

        Ok((code_ptr, deopt_points, shape_guards))
    }

    /// Compile a function for Tier 2 optimizing JIT with feedback-guided speculation.
    ///
    /// The target function's own FuncRef is declared with `Linkage::Local` for
    /// self-recursive direct calls. Cross-function monomorphic call sites get
    /// speculative direct calls when the callee has already been Tier-2 compiled:
    /// the callee's `opt_dc_*` FuncId is looked up in `compiled_dc_funcs` and
    /// a FuncRef is declared, enabling `compile_direct_call` to emit a true
    /// `call` instruction (not FFI). A callee identity guard protects every
    /// speculative call; on guard failure the JIT deopts to the interpreter.
    ///
    /// ABI: Returns a JitFnPtr wrapper `(ctx_ptr, unused) -> u64` that loads
    /// args from the ctx locals area, calls the direct-call function, and
    /// converts the result.
    pub fn compile_optimizing_function(
        &mut self,
        program: &BytecodeProgram,
        func_index: usize,
        feedback: shape_vm::feedback::FeedbackVector,
        callee_feedback: &HashMap<u16, shape_vm::feedback::FeedbackVector>,
    ) -> Result<
        (
            *const u8,
            Vec<shape_vm::bytecode::DeoptInfo>,
            Vec<shape_value::shape_graph::ShapeId>,
        ),
        String,
    > {
        use cranelift_module::{Linkage, Module};

        let func = program
            .functions
            .get(func_index)
            .ok_or_else(|| format!("Function {} not found in program", func_index))?;

        // Phase 1: Declare the TARGET function with direct-call ABI.
        //
        // Self-recursive calls get full direct-call via FuncRef.
        // Cross-function monomorphic calls get speculative direct calls when
        // the callee has already been Tier-2 compiled (FuncRef from
        // compiled_dc_funcs). Otherwise, guard + FFI fallthrough.
        let mut user_func_arities: HashMap<u16, u16> = HashMap::new();
        for (idx, f) in program.functions.iter().enumerate() {
            user_func_arities.insert(idx as u16, f.arity);
        }

        let dc_name = format!("opt_dc_{}_{}", func_index, func.name);
        let mut dc_sig = self.module.make_signature();
        dc_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        for _ in 0..func.arity {
            dc_sig.params.push(AbiParam::new(types::I64));
        }
        dc_sig.returns.push(AbiParam::new(types::I32)); // signal
        let target_dc_func_id = self
            .module
            .declare_function(&dc_name, Linkage::Local, &dc_sig)
            .map_err(|e| format!("Failed to declare function {}: {}", func.name, e))?;

        let mut deopt_points;
        let shape_guards;
        {
            let mut dc_sig = self.module.make_signature();
            dc_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
            for _ in 0..func.arity {
                dc_sig.params.push(AbiParam::new(types::I64));
            }
            dc_sig.returns.push(AbiParam::new(types::I32));

            let mut ctx = self.module.make_context();
            ctx.func.signature = dc_sig;

            let mut func_builder_ctx = FunctionBuilderContext::new();
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
                let entry_block = builder.create_block();
                builder.append_block_params_for_function_params(entry_block);
                builder.switch_to_block(entry_block);
                builder.seal_block(entry_block);

                let ctx_ptr = builder.block_params(entry_block)[0];

                // Self-recursive FuncRef for direct calls.
                let mut user_func_refs: HashMap<u16, FuncRef> = HashMap::new();
                {
                    let func_ref = self
                        .module
                        .declare_func_in_func(target_dc_func_id, builder.func);
                    user_func_refs.insert(func_index as u16, func_ref);
                }

                // Cross-function direct-call speculation: scan feedback for
                // monomorphic call targets that have already been Tier-2 compiled.
                // For each such callee, declare a FuncRef so compile_call_value
                // can emit a direct `call` instead of going through FFI.
                for (_slot_offset, slot) in feedback.slots.iter() {
                    if let shape_vm::feedback::FeedbackSlot::Call(fb) = slot {
                        if fb.state == shape_vm::feedback::ICState::Monomorphic {
                            if let Some(target) = fb.targets.first() {
                                let target_id = target.function_id;
                                if target_id != func_index as u16
                                    && !user_func_refs.contains_key(&target_id)
                                {
                                    // Check if this callee was already Tier-2 compiled
                                    if let Some(&(callee_func_id, _callee_arity)) =
                                        self.compiled_dc_funcs.get(&target_id)
                                    {
                                        let callee_ref = self
                                            .module
                                            .declare_func_in_func(callee_func_id, builder.func);
                                        user_func_refs.insert(target_id, callee_ref);
                                    }
                                }
                            }
                        }
                    }
                }

                let ffi = self.build_ffi_refs(&mut builder);

                // Build sub-program for target function.
                let func_end = program
                    .functions
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != func_index)
                    .filter_map(|(_, f)| {
                        if f.entry_point > func.entry_point {
                            Some(f.entry_point)
                        } else {
                            None
                        }
                    })
                    .min()
                    .unwrap_or(program.instructions.len());

                let target_instructions = &program.instructions[func.entry_point..func_end];
                let target_instr_count = target_instructions.len();

                // ---- Tier-2 inlining: find inline candidates and append callees ----
                //
                // analyze_inline_candidates needs the full program to determine
                // function boundaries and eligibility. We scan the target function's
                // bytecode for Call(fn_id) instructions referencing candidates, then
                // append those callees' instructions to the sub-program. The main
                // compilation loop uses skip_ranges to avoid compiling them;
                // compile_inline_call reads them via entry_point.
                let full_candidates = BytecodeToIR::analyze_inline_candidates(program);
                let mut sub_instructions_vec = target_instructions.to_vec();
                // Maps callee fn_id → rebased entry_point in the sub_program
                let mut callee_inline_map: HashMap<u16, InlineCandidate> = HashMap::new();
                // Skip ranges for appended callee instruction regions
                let mut callee_skip_ranges: Vec<(usize, usize)> = Vec::new();
                // Track callee offsets for feedback merging
                let mut callee_feedback_offsets: Vec<(u16, usize)> = Vec::new();

                for instr in target_instructions {
                    if let shape_vm::bytecode::OpCode::Call = instr.opcode {
                        if let Some(shape_vm::bytecode::Operand::Function(fn_id)) = &instr.operand {
                            let callee_id = fn_id.0;
                            // Skip self-recursive and already-processed callees
                            if callee_id == func_index as u16
                                || callee_inline_map.contains_key(&callee_id)
                            {
                                continue;
                            }
                            if let Some(candidate) = full_candidates.get(&callee_id) {
                                let callee_start = candidate.entry_point;
                                let callee_end = callee_start + candidate.instruction_count;
                                if callee_end <= program.instructions.len() {
                                    let callee_instrs =
                                        &program.instructions[callee_start..callee_end];
                                    let rebased_entry = sub_instructions_vec.len();
                                    sub_instructions_vec.extend_from_slice(callee_instrs);
                                    let rebased_end = sub_instructions_vec.len();

                                    callee_inline_map.insert(
                                        callee_id,
                                        InlineCandidate {
                                            entry_point: rebased_entry,
                                            instruction_count: candidate.instruction_count,
                                            arity: candidate.arity,
                                            locals_count: candidate.locals_count,
                                        },
                                    );
                                    callee_skip_ranges.push((rebased_entry, rebased_end));
                                    callee_feedback_offsets.push((callee_id, rebased_entry));
                                }
                            }
                        }
                    }
                }

                let sub_program = BytecodeProgram {
                    instructions: sub_instructions_vec,
                    constants: program.constants.clone(),
                    strings: program.strings.clone(),
                    functions: Vec::new(),
                    debug_info: Default::default(),
                    data_schema: program.data_schema.clone(),
                    module_binding_names: program.module_binding_names.clone(),
                    top_level_locals_count: program.top_level_locals_count,
                    top_level_local_storage_hints: program
                        .function_local_storage_hints
                        .get(func_index)
                        .cloned()
                        .unwrap_or_default(),
                    type_schema_registry: program.type_schema_registry.clone(),
                    module_binding_storage_hints: program.module_binding_storage_hints.clone(),
                    function_local_storage_hints: Vec::new(),
                    top_level_frame: None,
                    compiled_annotations: program.compiled_annotations.clone(),
                    trait_method_symbols: program.trait_method_symbols.clone(),
                    expanded_function_defs: program.expanded_function_defs.clone(),
                    string_index: Default::default(),
                    foreign_functions: program.foreign_functions.clone(),
                    native_struct_layouts: program.native_struct_layouts.clone(),
                    content_addressed: None,
                    function_blob_hashes: Vec::new(),
                };

                let mut compiler = BytecodeToIR::new(
                    &mut builder,
                    &sub_program,
                    ctx_ptr,
                    ffi,
                    user_func_refs,
                    user_func_arities.clone(),
                );
                // Only scan the outer function's instructions for numeric param hints,
                // not the appended callee instructions. Using the callee's Add/Mul etc.
                // would incorrectly mark outer's params as numeric.
                {
                    let mut outer_only = sub_program.clone();
                    outer_only.instructions.truncate(target_instr_count);
                    compiler.numeric_param_hints =
                        collect_numeric_param_hints(&outer_only, func.arity, &func.ref_params);
                }
                compiler.func_locals_count = func.locals_count;
                compiler.compiling_function_id = func_index as u16;

                // Inject inline candidates with rebased entry_points.
                // These are keyed by original fn_id so compile_call can find them.
                for (callee_id, candidate) in &callee_inline_map {
                    compiler
                        .inline_candidates
                        .insert(*callee_id, candidate.clone());
                }
                // Set skip_ranges so the main compilation loop doesn't process
                // appended callee instructions (they're only used by compile_inline_call).
                compiler.skip_ranges = callee_skip_ranges;

                // Rebase feedback slots from absolute IP to sub-program-local indices.
                let mut rebased_feedback = feedback;
                rebased_feedback.rebase(func.entry_point);
                // Merge callee feedback at their sub-program offsets so speculative
                // guards fire inside inlined code.
                for (callee_id, sub_offset) in &callee_feedback_offsets {
                    if let Some(callee_fv) = callee_feedback.get(callee_id) {
                        let mut callee_rebased = callee_fv.clone();
                        let callee_func = &program.functions[*callee_id as usize];
                        callee_rebased.rebase(callee_func.entry_point);
                        rebased_feedback.merge_at_offset(&callee_rebased, *sub_offset);
                    }
                }
                compiler.set_feedback(rebased_feedback);

                let entry_params = compiler.builder.block_params(entry_block).to_vec();
                for arg_idx in 0..func.arity {
                    let arg_val = entry_params[(arg_idx as usize) + 1];
                    let var = compiler.get_or_create_local(arg_idx);
                    compiler.builder.def_var(var, arg_val);
                }

                let result = compiler.compile()?;
                deopt_points = compiler.take_deopt_points();

                // Verify deopt metadata consistency before rebasing.
                BytecodeToIR::verify_deopt_points(
                    &deopt_points,
                    &compiler.unboxed_int_locals,
                    &compiler.unboxed_f64_locals,
                )?;

                // Rebase deopt resume_ip from sub-program-local to global program IP.
                //
                // Sub-program layout: [outer instrs @ 0..N] [callee1 @ N..N+M] ...
                // Each region maps back to a different base in the original program:
                //   - outer region [0, target_instr_count): rebase by +func.entry_point
                //   - callee region [rebased_entry, rebased_end): rebase to callee.entry_point
                //
                // rebase_ip closure handles both regions.
                let rebase_ip = |sub_ip: usize| -> usize {
                    for (callee_id, candidate) in &callee_inline_map {
                        let re = candidate.entry_point;
                        let re_end = re + candidate.instruction_count;
                        if sub_ip >= re && sub_ip < re_end {
                            let callee_entry = program.functions[*callee_id as usize].entry_point;
                            return callee_entry + (sub_ip - re);
                        }
                    }
                    // Falls in the outer function's region
                    func.entry_point + sub_ip
                };

                for dp in &mut deopt_points {
                    dp.resume_ip = rebase_ip(dp.resume_ip);
                    for iframe in &mut dp.inline_frames {
                        iframe.resume_ip = rebase_ip(iframe.resume_ip);
                    }
                }
                shape_guards = compiler.take_shape_guards();
                drop(compiler);

                builder.ins().return_(&[result]);
                builder.finalize();
            }

            self.module
                .define_function(target_dc_func_id, &mut ctx)
                .map_err(|e| format!("Failed to define target function: {:?}", e))?;
            self.module.clear_context(&mut ctx);

            // Record this function's direct-call FuncId for cross-function
            // speculation by future compilations.
            self.compiled_dc_funcs
                .insert(func_index as u16, (target_dc_func_id, func.arity));
        }

        // Phase 2: Create JitFnPtr wrapper that marshals from (ctx_ptr, unused) -> u64
        // to (ctx_ptr, args...) -> i32 ABI.
        let wrapper_name = format!("opt_wrapper_{}", func.name);
        let mut wrapper_sig = self.module.make_signature();
        wrapper_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        wrapper_sig.params.push(AbiParam::new(types::I64)); // unused
        wrapper_sig.returns.push(AbiParam::new(types::I64)); // NaN-boxed result

        let wrapper_func_id = self
            .module
            .declare_function(&wrapper_name, Linkage::Export, &wrapper_sig)
            .map_err(|e| format!("Failed to declare wrapper: {}", e))?;

        {
            let mut ctx = self.module.make_context();
            ctx.func.signature = wrapper_sig;

            let mut func_builder_ctx = FunctionBuilderContext::new();
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_builder_ctx);
                let entry_block = builder.create_block();
                builder.append_block_params_for_function_params(entry_block);
                builder.switch_to_block(entry_block);
                builder.seal_block(entry_block);

                let ctx_ptr = builder.block_params(entry_block)[0];

                // Import the target direct-call function
                let target_ref = self
                    .module
                    .declare_func_in_func(target_dc_func_id, builder.func);

                // Load args from JITContext locals area and call the target
                const LOCALS_BYTE_OFFSET: i32 = 64;
                let mut call_args = vec![ctx_ptr];
                for arg_idx in 0..func.arity {
                    let offset = LOCALS_BYTE_OFFSET + (arg_idx as i32) * 8;
                    let arg_val =
                        builder
                            .ins()
                            .load(types::I64, MemFlags::trusted(), ctx_ptr, offset);
                    call_args.push(arg_val);
                }

                let inst = builder.ins().call(target_ref, &call_args);
                let signal = builder.inst_results(inst)[0]; // i32

                // Convert signal to JitFnPtr return value:
                // signal < 0 → u64::MAX (deopt), signal >= 0 → load result from ctx stack
                use crate::context::STACK_OFFSET;
                let zero = builder.ins().iconst(types::I32, 0);
                let is_deopt = builder.ins().icmp(IntCC::SignedLessThan, signal, zero);

                let deopt_block = builder.create_block();
                let success_block = builder.create_block();
                let merge_block = builder.create_block();
                builder.append_block_param(merge_block, types::I64);

                builder
                    .ins()
                    .brif(is_deopt, deopt_block, &[], success_block, &[]);

                builder.switch_to_block(deopt_block);
                builder.seal_block(deopt_block);
                let deopt_sentinel = builder.ins().iconst(types::I64, -1i64);
                builder.ins().jump(merge_block, &[deopt_sentinel]);

                builder.switch_to_block(success_block);
                builder.seal_block(success_block);
                let result_val =
                    builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), ctx_ptr, STACK_OFFSET);
                builder.ins().jump(merge_block, &[result_val]);

                builder.switch_to_block(merge_block);
                builder.seal_block(merge_block);
                let ret_val = builder.block_params(merge_block)[0];
                builder.ins().return_(&[ret_val]);
                builder.finalize();
            }

            self.module
                .define_function(wrapper_func_id, &mut ctx)
                .map_err(|e| format!("Failed to define wrapper: {:?}", e))?;
            self.module.clear_context(&mut ctx);
        }

        // Phase 3: Finalize and return the wrapper code pointer.
        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize: {:?}", e))?;

        let code_ptr = self.module.get_finalized_function(wrapper_func_id);
        self.compiled_functions.insert(wrapper_name, code_ptr);

        Ok((code_ptr, deopt_points, shape_guards))
    }

    /// Selectively compile a program, JIT-compiling compatible functions and
    /// falling back to interpreter entries for incompatible ones.
    ///
    /// Returns a `MixedFunctionTable` mapping each function index to either
    /// a `Native` pointer (JIT-compiled) or `Interpreted` marker.
    ///
    /// The main strategy body is always compiled. Only user-defined functions
    /// go through per-function preflight.
    pub fn compile_program_selective(
        &mut self,
        name: &str,
        program: &BytecodeProgram,
    ) -> Result<(JittedStrategyFn, MixedFunctionTable), String> {
        use super::accessors::preflight_instructions;

        maybe_emit_numeric_metrics(program);

        // Phase 1: Per-function preflight to classify each function.
        let mut jit_compatible: Vec<bool> = Vec::with_capacity(program.functions.len());

        for (_idx, func) in program.functions.iter().enumerate() {
            if func.body_length == 0 {
                jit_compatible.push(false);
                continue;
            }
            let func_end = func.entry_point + func.body_length;
            let instructions = &program.instructions[func.entry_point..func_end];
            let report = preflight_instructions(instructions);
            jit_compatible.push(report.can_jit());
        }

        // Phase 1b: Preflight main code (non-stdlib, non-function-body instructions).
        // Without this, unsupported builtins in top-level code slip through.
        {
            let skip_ranges = Self::compute_skip_ranges(program);
            let main_instructions: Vec<_> = program
                .instructions
                .iter()
                .enumerate()
                .filter(|(i, _)| !skip_ranges.iter().any(|(s, e)| *i >= *s && *i < *e))
                .map(|(_, instr)| instr.clone())
                .collect();
            let main_report = preflight_instructions(&main_instructions);
            if !main_report.can_jit() {
                return Err(format!(
                    "Main code contains unsupported constructs: {:?}",
                    main_report
                ));
            }
        }

        // Phase 2: Pre-declare ALL functions (both JIT and interpreted) in
        // Cranelift so that JIT functions can call other JIT functions.
        // Interpreted functions get declared too (for uniform call tables)
        // but won't have a body defined - they'll use the trampoline.
        let mut user_func_arities: HashMap<u16, u16> = HashMap::new();
        let mut user_func_ids: HashMap<u16, cranelift_module::FuncId> = HashMap::new();

        for (idx, func) in program.functions.iter().enumerate() {
            if !jit_compatible[idx] {
                user_func_arities.insert(idx as u16, func.arity);
                continue;
            }
            // Use function index in the name to avoid collisions between
            // closures with the same auto-generated name but different arities
            // (e.g., multiple __closure_0 from different stdlib modules).
            let func_name = format!("{}_f{}_{}", name, idx, func.name);
            let mut user_sig = self.module.make_signature();
            user_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
            for _ in 0..func.arity {
                user_sig.params.push(AbiParam::new(types::I64));
            }
            user_sig.returns.push(AbiParam::new(types::I32));
            let func_id = self
                .module
                .declare_function(&func_name, Linkage::Local, &user_sig)
                .map_err(|e| format!("Failed to pre-declare function {}: {}", func.name, e))?;
            user_func_ids.insert(idx as u16, func_id);
            user_func_arities.insert(idx as u16, func.arity);
        }

        // Phase 3: Compile main strategy body.
        let main_func_id = self.compile_strategy_with_user_funcs(
            name,
            program,
            &user_func_ids,
            &user_func_arities,
        )?;

        // Phase 4: Compile only JIT-compatible function bodies.
        for (idx, func) in program.functions.iter().enumerate() {
            if !jit_compatible[idx] {
                continue;
            }
            let func_name = format!("{}_f{}_{}", name, idx, func.name);
            self.compile_function_with_user_funcs(
                &func_name,
                program,
                idx,
                &user_func_ids,
                &user_func_arities,
            )?;
        }

        self.module
            .finalize_definitions()
            .map_err(|e| format!("Failed to finalize definitions: {:?}", e))?;

        let main_code_ptr = self.module.get_finalized_function(main_func_id);
        self.compiled_functions
            .insert(name.to_string(), main_code_ptr);

        // Phase 5: Build the MixedFunctionTable.
        let mut mixed_table = MixedFunctionTable::with_capacity(program.functions.len());

        self.function_table.clear();
        for (idx, func) in program.functions.iter().enumerate() {
            if jit_compatible[idx] {
                if let Some(&func_id) = user_func_ids.get(&(idx as u16)) {
                    let ptr = self.module.get_finalized_function(func_id);
                    while self.function_table.len() <= idx {
                        self.function_table.push(std::ptr::null());
                    }
                    self.function_table[idx] = ptr;
                    let func_name = format!("{}_f{}_{}", name, idx, func.name);
                    self.compiled_functions.insert(func_name, ptr);
                    mixed_table.insert(idx, FunctionEntry::Native(ptr));
                }
            } else {
                while self.function_table.len() <= idx {
                    self.function_table.push(std::ptr::null());
                }
                // Leave function_table[idx] as null for interpreted functions.
                mixed_table.insert(idx, FunctionEntry::Interpreted(idx as u16));
            }
        }

        let jit_fn = unsafe { std::mem::transmute(main_code_ptr) };
        Ok((jit_fn, mixed_table))
    }
}
