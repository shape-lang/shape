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
            // Generic arithmetic/comparison opcodes (DELETED in Phase 2 — left
            // here as a no-op arm for future-proofing if a generic class is
            // re-introduced).
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

            let ffi = self.build_ffi_refs(&mut builder)?;

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
                top_level_local_concrete_types: Vec::new(),
                function_local_concrete_types: Vec::new(),
                function_return_concrete_types: Vec::new(),
                monomorphized_method_call_sites: Default::default(),
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
                closure_function_layouts: program.closure_function_layouts.clone(),
                trait_vtables: program.trait_vtables.clone(),
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
                let slot_kinds: Vec<Option<shape_vm::type_tracking::NativeKind>> = func
                    .frame_descriptor
                    .as_ref()
                    .map(|fd| fd.slots.iter().copied().map(Some).collect())
                    .unwrap_or_default();
                // ADR-006 §2.7.5 conduit: thread the bytecode compiler's
                // proven per-MIR-slot `ConcreteType` for THIS user function
                // into MirToIR (W12-jit-aggregate-non-array close,
                // 2026-05-12). The producer
                // (`infer_top_level_concrete_types_from_mir`) was already
                // landed for top-level code by Round 3; its body is generic
                // over any MirFunction, and Round 5B extends the populate
                // site to per-user-function MIR via
                // `BytecodeProgram.function_local_concrete_types`. The
                // top-level conduit's user-visible benefit (Smoke 3
                // `Point{}` literal short-circuit) now extends to user
                // function bodies (`Ok(v)`/`Err(e)`/`Some(x)` inside
                // `divide` / `first_positive` / 28 stdlib helpers).
                //
                // Empty inner vec (function has no MIR data, or the conduit
                // couldn't prove a particular slot) → MirToIR's v2 fast
                // path falls through to the legacy NaN-boxed path / surfaces
                // honestly per ADR-006 §2.7.5.1 (no Bool-default).
                let concrete_types: Vec<shape_value::v2::ConcreteType> =
                    program
                        .function_local_concrete_types
                        .get(func_idx)
                        .cloned()
                        .unwrap_or_default();
                // Build function name → index map for Call terminator resolution.
                // Use the original program's functions (sub_program has empty functions list).
                let function_indices: std::collections::HashMap<String, u16> = program
                    .functions
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.name.clone(), i as u16))
                    .collect();
                // Closure-spec Phase H1: thread the per-function
                // ClosureLayout map into MirToIR so `emit_heap_closure`
                // can lay out captures at natural-width offsets without
                // going through the legacy `jit_make_closure` FFI.
                let closure_function_layouts: std::collections::HashMap<
                    u16,
                    std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>,
                > = program
                    .closure_function_layouts
                    .iter()
                    .enumerate()
                    .filter_map(|(i, opt)| opt.as_ref().map(|l| (i as u16, l.clone())))
                    .collect();
                let mut mir_compiler = crate::mir_compiler::MirToIR::new_with_closure_layouts(
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
                    closure_function_layouts,
                );
                // V3-S6c-jit-method-monomorph-routing (ADR-006 §2.7.5
                // stamp-at-compile-time; supervisor 2026-05-15 PATH α-prime
                // RATIFIED): thread the V3-S6b side-table from the ORIGINAL
                // `program: &BytecodeProgram` (the `sub_program` above
                // clears it at line ~305 to keep the per-function compile
                // scope minimal) so the Call-terminator pass can re-route
                // `MirConstant::Method` sites to direct FuncRef calls.
                //
                // Composite key `(call_site_span, caller_function_id)`:
                // `caller_function_id = Some(func_idx)` matches the
                // bytecode compiler's `self.current_function` at
                // specialization time (`expressions/function_calls.rs:3278`).
                mir_compiler.set_monomorph_routing_context(
                    program.monomorphized_method_call_sites.clone(),
                    Some(func_idx),
                );
                // Bounds-check elision: install the per-function plan
                // before MIR codegen so `Place::Index` lowering can
                // bypass the inline bounds check on trusted (arr, iv)
                // pairs. Default empty plan keeps every access checked.
                let elision_plan =
                    crate::mir_compiler::bounds_elision::analyze(&mir_data.mir);
                mir_compiler.set_bounds_elision_plan(elision_plan);
                // Track A.1D.2: flag the leading capture param slots whose
                // `ClosureLayout` marks them as `OwnedMutable`. `read_place`
                // / `write_place` then route through the cell pointer bits
                // stored in those slots, matching the interpreter's
                // `Load/StoreOwnedMutableCapture` handlers. The lookup is
                // keyed on this function's own `func_idx`, which doubles as
                // the closure body's `function_id` when it is a closure.
                // Non-closure functions hit no entry in the layout map →
                // the side-table stays empty, preserving pre-A.1D.2
                // behaviour for ordinary functions.
                if func.is_closure && func.captures_count > 0 {
                    if let Some(layout) = program
                        .closure_function_layouts
                        .get(func_idx)
                        .and_then(|o| o.as_ref())
                    {
                        mir_compiler.register_owned_mutable_capture_slots(
                            func.captures_count,
                            layout.as_ref(),
                        );
                    }
                }
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

                // Session 1 Commit 3: allocate Arc<SharedCell>s for
                // every SharedCow local slot (outer-scope `var` bindings
                // that escape into closures). After this call every
                // SharedCow slot's Cranelift var holds the raw
                // `*const SharedCell` pointer bits; subsequent
                // read_place / write_place route through the lock-gated
                // pointer-deref lowering, and `emit_drop` on the slot
                // emits `jit_arc_shared_release` to balance the share.
                mir_compiler.initialize_shared_local_slots();

                // Store function parameters (including captures) to MIR local variables.
                // MIR param_slots includes capture slots followed by user param slots.
                // Entry block params: [ctx_ptr, capture0..N, param0..M]
                // param_slots aligns 1:1 with captures+params, so native_idx = param_idx + 1.
                //
                // R4.2E: callee ABI delivers params as uniform I64 bit-patterns.
                // When the MIR slot is a native narrow type, reduce I64 → narrow
                // inline (bitcast for F64, ireduce for I32/I16/I8). No NaN-box
                // tag stripping — raw bit-patterns only.
                for (param_idx, &mir_slot) in param_slots.iter().enumerate() {
                    let native_idx = param_idx + 1; // +1 for ctx_ptr
                    if native_idx < entry_params.len() {
                        if let Some(&var) = mir_compiler.locals.get(&mir_slot) {
                            let kind = crate::mir_compiler::types::slot_kind_for_local(
                                &mir_compiler.slot_kinds,
                                mir_slot.0,
                            );
                            let param_val = entry_params[native_idx];
                            let converted = match kind {
                                Some(shape_vm::type_tracking::NativeKind::Float64) => mir_compiler
                                    .builder
                                    .ins()
                                    .bitcast(types::F64, MemFlags::new(), param_val),
                                Some(shape_vm::type_tracking::NativeKind::Int32)
                                | Some(shape_vm::type_tracking::NativeKind::UInt32) => {
                                    mir_compiler.builder.ins().ireduce(types::I32, param_val)
                                }
                                Some(shape_vm::type_tracking::NativeKind::Bool)
                                | Some(shape_vm::type_tracking::NativeKind::Int8)
                                | Some(shape_vm::type_tracking::NativeKind::UInt8) => {
                                    mir_compiler.builder.ins().ireduce(types::I8, param_val)
                                }
                                Some(shape_vm::type_tracking::NativeKind::Int16)
                                | Some(shape_vm::type_tracking::NativeKind::UInt16) => {
                                    mir_compiler.builder.ins().ireduce(types::I16, param_val)
                                }
                                _ => param_val,
                            };
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
            // Track A.1D / A.1D.2: the A.1B/A.1C.1/A.1C.3 mutable-cell
            // opcodes carry runtime semantics the MIR layer cannot
            // reconstruct from its slot-based model — MIR just sees
            // `LoadLocal` / `StoreLocal`, erasing the pointer-deref
            // semantics the cell opcodes encode.
            //
            // A.1D.2 closes the gap for `LoadOwnedMutableCapture` /
            // `StoreOwnedMutableCapture` via a JIT-side side-table that
            // patches `read_place` / `write_place` on flagged capture
            // slots (see `MirToIR::register_owned_mutable_capture_slots`).
            // Those two opcodes have been removed from
            // `vm_only_opcode_reason`, so `bytecode_ok` is now `true`
            // for functions whose only cell opcodes are OwnedMutable.
            //
            // A.1E closed the gap for the closure-body Shared-cell
            // opcodes (`LoadSharedCapture` / `StoreSharedCapture`) via
            // the `MirToIR::shared_capture_slots` side-table.
            //
            // Session 1 Commit 3 lands the MirToIR infrastructure for
            // the outer-scope `var` cell lifecycle — the
            // `MirToIR::shared_local_slots` side-table is populated
            // from `StoragePlan::slot_classes`, function entry
            // allocates one `Arc<SharedCell>` per SharedCow slot via
            // `jit_alloc_shared_cell`, and `read_place`/`write_place`
            // /`emit_drop` branch to lock-gated access +
            // `jit_arc_shared_release`. The preflight gate for the
            // four local opcodes (`AllocSharedLocal` /
            // `LoadSharedLocal` / `StoreSharedLocal` /
            // `DropSharedLocal`) REMAINS IN PLACE pending resolution
            // of the outer-frame cell-identity handshake —
            // lifting the gate prematurely segfaults the JIT'd
            // outer frame's interaction with closure dispatch (see
            // memory note `project_jit_closure_fix.md`).
            //
            // Still gated after this commit:
            //   * the four outer-scope `var` local opcodes above;
            //   * the three module-binding opcodes
            //     (`AllocSharedModuleBinding`,
            //     `LoadSharedModuleBinding`,
            //     `StoreSharedModuleBinding`) — per-module side-table,
            //     separate lowering (A.1C.3 follow-up).
            let _ = mir_ok;
            jit_compatible.push(bytecode_ok);
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
                //
                // W12-jit-linker-resolve (`docs/cluster-audits/w12-jit-linker-audit.md`):
                // Cranelift's `iconst` immediate-bounds rule requires the I32
                // immediate to be the unsigned bit-pattern, not the signed
                // value. `iconst.i32 -1` is rejected by the verifier because
                // `-1i64 as u64 = 0xFFFFFFFFFFFFFFFF` exceeds the I32 mask
                // `u32::MAX = 0xFFFFFFFF`. Pass the two's-complement unsigned
                // bit pattern instead — see `cranelift-codegen/src/verifier/
                // mod.rs:1644-1665` for the documented invariant.
                //
                // Also: previously the stub `define_function` failure was
                // silently swallowed via `let _ = ...`, which left the
                // declared FuncId with no body and caused `finalize_definitions`
                // to panic with `can't resolve symbol main_f{idx}_{name}` —
                // the very surface this audit traced. Surface the stub
                // failure under `SHAPE_JIT_DEBUG=1` so future regressions
                // don't hide beneath the linker panic.
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
                        // Cranelift I32 iconst convention: pass the unsigned
                        // bit-pattern, not the signed value. `-1i32` is
                        // `0xFFFFFFFF` as a `u32`.
                        let neg = b.ins().iconst(types::I32, (-1i32 as u32) as i64);
                        b.ins().return_(&[neg]);
                        b.finalize();
                    }
                    if let Err(stub_err) = self.module.define_function(fid, &mut stub_ctx) {
                        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
                            eprintln!(
                                "[jit-mir] stub define_function failed for '{}' (idx={}, fid={:?}): {:?}",
                                func.name, idx, fid, stub_err
                            );
                        }
                        // Surface-and-stop: a failed stub leaves the declared
                        // FuncId with no body, which propagates to
                        // `finalize_definitions` as a `can't resolve symbol`
                        // panic. Convert to a structured error here so the
                        // caller sees a typed JIT-compilation failure, not a
                        // panic through `catch_unwind`. The stub itself was
                        // supposed to be a recovery path; if recovery fails,
                        // the whole JIT compilation is unsound.
                        return Err(format!(
                            "JIT stub fallback failed for function '{}' (idx={}): {:?}. \
                             The Cranelift module is in an inconsistent state — \
                             this is a JIT-compiler bug, not a user-code error. \
                             See docs/cluster-audits/w12-jit-linker-audit.md.",
                            func.name, idx, stub_err
                        ));
                    }
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
