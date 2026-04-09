//! Program compilation with multiple functions

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};
use std::collections::{BTreeMap, HashMap};

use super::setup::JITCompiler;
use crate::context::{JittedFn, JittedStrategyFn};
use crate::mixed_table::{FunctionEntry, MixedFunctionTable};
use crate::numeric_compiler::compile_numeric_program;
use shape_vm::bytecode::{BytecodeProgram, OpCode};

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
            | OpCode::NeqNumber
            | OpCode::EqString
            | OpCode::GtString
            | OpCode::LtString
            | OpCode::GteString
            | OpCode::LteString
            | OpCode::EqDecimal
            | OpCode::IsNull
            | OpCode::NegInt
            | OpCode::NegNumber => {
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
            | OpCode::EqDynamic
            | OpCode::NeqDynamic => {
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
            let func_name = format!("{}_{}", name, func.name.replace("::", "__"));
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
            let func_name = format!("{}_{}", name, func.name.replace("::", "__"));
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
            let func_name = format!("{}_{}", name, func.name.replace("::", "__"));
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
        // Closures receive captures as leading native args, followed by user params.
        let effective_arity = func.captures_count + func.arity;
        for _ in 0..effective_arity {
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
                top_level_mir: None,
                compiled_annotations: program.compiled_annotations.clone(),
                trait_method_symbols: program.trait_method_symbols.clone(),
                expanded_function_defs: program.expanded_function_defs.clone(),
                string_index: Default::default(),
                foreign_functions: program.foreign_functions.clone(),
                native_struct_layouts: program.native_struct_layouts.clone(),
                content_addressed: None,
                function_blob_hashes: Vec::new(),
                monomorphization_keys: Vec::new(),
            };

            // MirToIR is the ONLY JIT compilation path (Phase 4: BytecodeToIR removed).
            // All functions must have valid MIR data. If not, report the error.
            let mir_data = func.mir_data.as_ref().ok_or_else(|| {
                format!("MirToIR: function '{}' has no MIR data (bytecode-only functions are no longer supported)", func.name)
            })?;
            let preflight = crate::mir_compiler::preflight(mir_data);
            if !preflight.can_compile {
                return Err(format!(
                    "MirToIR: function '{}' failed preflight: {}",
                    func.name,
                    preflight.blockers.join("; ")
                ));
            }

            {
                let slot_kinds = func
                    .frame_descriptor
                    .as_ref()
                    .map(|fd| fd.slots.clone())
                    .unwrap_or_default();
                // v2: per-slot ConcreteTypes for the v2 typed-array fast path.
                // The bytecode-program-level side-table is in flux upstream
                // (other Phase 3.1 agents are refactoring it), so we pass an
                // empty vec for now — MirToIR's v2 fast path falls through to
                // the legacy NaN-boxed path on `None`. Wire-up will happen
                // once Agent 1 lands the BytecodeProgram concrete-types vec.
                let concrete_types: Vec<shape_value::v2::ConcreteType> = Vec::new();
                let _ = func_idx; // silence dead-binding warning until wire-up.
                // Build function name → index map for Call terminator resolution.
                // Use the original program's functions (sub_program has empty functions list).
                let function_indices: std::collections::HashMap<String, u16> = program
                    .functions
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.name.clone(), i as u16))
                    .collect();
                let mut mir_compiler = crate::mir_compiler::MirToIR::new_with_concrete_types(
                    &mut builder,
                    ctx_ptr,
                    ffi,
                    mir_data,
                    slot_kinds,
                    concrete_types,
                    &sub_program.strings,
                    entry_block,
                    &function_indices,
                    user_func_refs.clone(),
                    user_func_arities.clone(),
                );
                // Set up blocks and locals, then store function parameters.
                mir_compiler.create_blocks();
                mir_compiler.declare_locals();

                // Store function parameters to MIR local variables.
                // MIR slot layout: [return_slot(0), param0(1), param1(2), ..., locals...]
                // Entry block params: [ctx_ptr, capture0..N, param0..N]
                // Use mir.param_slots to map params to their actual MIR slots.
                let entry_params = mir_compiler.builder.block_params(entry_block).to_vec();
                let param_slots = &mir_data.mir.param_slots;

                // Initialize ALL locals with type-appropriate defaults.
                mir_compiler.initialize_locals();

                // Store function parameters (including captures) to MIR local variables.
                // MIR param_slots includes capture slots followed by user param slots.
                // Entry block params: [ctx_ptr, capture0..N, param0..M]
                // param_slots aligns 1:1 with captures+params, so native_idx = param_idx + 1.
                // Entry params are I64 (NaN-boxed); convert to native type if needed.
                for (param_idx, &mir_slot) in param_slots.iter().enumerate() {
                    let native_idx = param_idx + 1; // +1 for ctx_ptr
                    if native_idx < entry_params.len() {
                        if let Some(&var) = mir_compiler.locals.get(&mir_slot) {
                            let kind = crate::mir_compiler::types::slot_kind_for_local(
                                &mir_compiler.slot_kinds,
                                mir_slot.0,
                            );
                            let param_val = entry_params[native_idx];
                            let converted =
                                mir_compiler.unbox_from_nanboxed(param_val, kind);
                            mir_compiler.builder.def_var(var, converted);
                        }
                    }
                }
                mir_compiler.compile_body()?;
                if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                    eprintln!("[jit-mir] Compiled function '{}' via MirToIR", func.name);
                }
            }
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
    /// This path previously used BytecodeToIR which has been removed.
    /// Tier 1 JIT is deprecated; use compile_program_selective instead.
    pub fn compile_single_function(
        &mut self,
        _program: &BytecodeProgram,
        _func_index: usize,
        _feedback: Option<shape_vm::feedback::FeedbackVector>,
    ) -> Result<
        (
            *const u8,
            Vec<shape_vm::bytecode::DeoptInfo>,
            Vec<shape_value::shape_graph::ShapeId>,
        ),
        String,
    > {
        Err("Tier 1 JIT is deprecated".to_string())
    }

    /// Compile a function for Tier 2 optimizing JIT with feedback-guided speculation.
    ///
    /// This path previously used BytecodeToIR which has been removed.
    /// Optimizing JIT is deprecated; use compile_program_selective instead.
    pub fn compile_optimizing_function(
        &mut self,
        _program: &BytecodeProgram,
        _func_index: usize,
        _feedback: shape_vm::feedback::FeedbackVector,
        _callee_feedback: &HashMap<u16, shape_vm::feedback::FeedbackVector>,
    ) -> Result<
        (
            *const u8,
            Vec<shape_vm::bytecode::DeoptInfo>,
            Vec<shape_value::shape_graph::ShapeId>,
        ),
        String,
    > {
        Err("Optimizing JIT is deprecated".to_string())
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
        // A function is JIT-compatible if its bytecode passes instruction
        // preflight OR it has MIR data that passes MirToIR preflight.
        // MirToIR is the compilation path — bytecode preflight only gates eligibility.
        let mut jit_compatible: Vec<bool> = Vec::with_capacity(program.functions.len());

        for (_idx, func) in program.functions.iter().enumerate() {
            if func.body_length == 0 {
                jit_compatible.push(false);
                continue;
            }
            let func_end = func.entry_point + func.body_length;
            let instructions = &program.instructions[func.entry_point..func_end];
            let report = preflight_instructions(instructions);
            let bytecode_ok = report.can_jit();
            let mir_ok = func.mir_data.as_ref().is_some_and(|md| {
                crate::mir_compiler::preflight(md).can_compile
            });
            jit_compatible.push(bytecode_ok || mir_ok);
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
            let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
            let mut user_sig = self.module.make_signature();
            user_sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
            // Closures receive captures as leading native args, followed by user params.
            let effective_arity = func.captures_count + func.arity;
            for _ in 0..effective_arity {
                user_sig.params.push(AbiParam::new(types::I64));
            }
            user_sig.returns.push(AbiParam::new(types::I32));
            let func_id = self
                .module
                .declare_function(&func_name, Linkage::Local, &user_sig)
                .map_err(|e| format!("Failed to pre-declare function {}: {}", func.name, e))?;
            user_func_ids.insert(idx as u16, func_id);
            // Store user-visible arity (without captures) for CallValue arg count checks
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
        // Functions that fail to compile are demoted to interpreted fallback.
        for (idx, func) in program.functions.iter().enumerate() {
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some()
                && (jit_compatible[idx] || func.mir_data.is_some())
            {
                eprintln!(
                    "[jit-mir] func[{}]='{}' jit_compat={} has_mir={}",
                    idx, func.name, jit_compatible[idx], func.mir_data.is_some()
                );
            }
            if !jit_compatible[idx] {
                continue;
            }
            let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
            if std::env::var_os("SHAPE_JIT_DEBUG").is_some() && func.mir_data.is_some() {
                eprintln!("[jit-mir] compiling '{}' idx={}", func.name, idx);
            }
            if let Err(e) = self.compile_function_with_user_funcs(
                &func_name,
                program,
                idx,
                &user_func_ids,
                &user_func_arities,
            ) {
                if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                    eprintln!("[jit-mir] compile failed for '{}': {}", func.name, e);
                }
                // Define a stub body so Cranelift doesn't panic on undefined symbol.
                // The stub returns signal -1 (error), causing the caller to deopt.
                if let Some(&fid) = user_func_ids.get(&(idx as u16)) {
                    let mut stub_sig = self.module.make_signature();
                    stub_sig.params.push(AbiParam::new(types::I64));
                    let effective_arity = func.captures_count + func.arity;
                    for _ in 0..effective_arity {
                        stub_sig.params.push(AbiParam::new(types::I64));
                    }
                    stub_sig.returns.push(AbiParam::new(types::I32));
                    let mut stub_ctx = self.module.make_context();
                    stub_ctx.func.signature = stub_sig;
                    let mut stub_builder_ctx = FunctionBuilderContext::new();
                    {
                        let mut b = FunctionBuilder::new(&mut stub_ctx.func, &mut stub_builder_ctx);
                        let block = b.create_block();
                        b.append_block_params_for_function_params(block);
                        b.switch_to_block(block);
                        b.seal_block(block);
                        let neg = b.ins().iconst(types::I32, -1);
                        b.ins().return_(&[neg]);
                        b.finalize();
                    }
                    let _ = self.module.define_function(fid, &mut stub_ctx);
                    self.module.clear_context(&mut stub_ctx);
                }
                jit_compatible[idx] = false;
            }
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
                    let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
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
