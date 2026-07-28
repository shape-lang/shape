//! Program compilation with multiple functions

use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};
use std::collections::HashMap;

use super::function_abi::prove_user_function_abi;
use super::program_finalize::finalize_program_definitions;
use super::program_metrics::maybe_emit_numeric_metrics;
use super::setup::JITCompiler;
use crate::context::{JittedFn, JittedStrategyFn};
use crate::mixed_table::{FunctionEntry, MixedFunctionTable};
use crate::numeric_compiler::compile_numeric_program;
use shape_vm::bytecode::BytecodeProgram;

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
        let mut user_func_return_kinds: HashMap<u16, shape_vm::type_tracking::NativeKind> =
            HashMap::new();

        for (idx, func) in program.functions.iter().enumerate() {
            if let Some(return_kind) = func.frame_descriptor.as_ref().and_then(|fd| fd.return_kind)
            {
                user_func_return_kinds.insert(idx as u16, return_kind);
            }
            let func_name = format!("{}_{}", name, func.name.replace("::", "__"));
            let abi = prove_user_function_abi(self.module.make_signature(), func)?;
            let func_id = self
                .module
                .declare_function(&func_name, Linkage::Local, &abi.signature)
                .map_err(|e| format!("Failed to pre-declare function {}: {}", func.name, e))?;
            user_func_ids.insert(idx as u16, func_id);
            user_func_arities.insert(idx as u16, abi.native_arity);
        }

        let main_func_id = self.compile_strategy_with_user_funcs(
            name,
            program,
            &user_func_ids,
            &user_func_arities,
            &user_func_return_kinds,
        )?;

        for (idx, func) in program.functions.iter().enumerate() {
            let func_name = format!("{}_{}", name, func.name.replace("::", "__"));
            self.compile_function_with_user_funcs(
                &func_name,
                program,
                idx,
                &user_func_ids,
                &user_func_arities,
                &user_func_return_kinds,
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
        user_func_return_kinds: &HashMap<u16, shape_vm::type_tracking::NativeKind>,
    ) -> Result<(), String> {
        let func = &program.functions[func_idx];
        let func_id = *user_func_ids
            .get(&(func_idx as u16))
            .ok_or_else(|| format!("Function {} not pre-declared", name))?;

        let abi = prove_user_function_abi(self.module.make_signature(), func)?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = abi.signature;

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

            // #117 / R15: announce native entry for this unit before any body
            // instruction, so a body that early-returns still counts as
            // dispatched. No-op unless a witness session is collecting.
            self.emit_native_witness_entry(&mut builder, func_idx)?;

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
                value_call_return_concrete_types: Default::default(),
                operator_trait_dispatch_sites: Default::default(),
                top_level_mir: None,
                top_level_has_comptime: false,
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
                has_imported_const_inline: program.has_imported_const_inline,
                has_w17_marshal_residual: program.has_w17_marshal_residual,
                has_try_unwrap_residual: program.has_try_unwrap_residual,
                has_reference_escape_promotion: program.has_reference_escape_promotion,
                has_null_coalesce_residual: program.has_null_coalesce_residual,
                // `functions` above is deliberately empty — this sub-program is
                // one rebased function body, so the parent's function-index
                // attribution does not apply here. The caller already refused
                // to compile any residual-bearing function, so an empty map is
                // the accurate statement about THIS body.
                jit_residuals: Default::default(),
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
                let concrete_types: Vec<shape_value::v2::ConcreteType> = program
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
                let mut mir_compiler =
                    crate::mir_compiler::MirToIR::new_with_closure_layouts_and_function_returns(
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
                        user_func_return_kinds.clone(),
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
                // W10 jit-call-method-user-trait-fix (2026-05-17): install
                // the bytecode compiler's operator-trait-dispatch side-
                // table so the per-user-function MirToIR consumer can
                // re-emit `Rvalue::BinaryOp` / `Rvalue::UnaryOp` at
                // trait-dispatch spans as method-call IR.
                mir_compiler.set_operator_trait_dispatch_sites(
                    program.operator_trait_dispatch_sites.clone(),
                );
                // Bounds-check elision: install the per-function plan
                // before MIR codegen so `Place::Index` lowering can
                // bypass the inline bounds check on trusted (arr, iv)
                // pairs. Default empty plan keeps every access checked.
                let elision_plan = crate::mir_compiler::bounds_elision::analyze(&mir_data.mir);
                mir_compiler.set_bounds_elision_plan(elision_plan);
                // W14.2-E-followup SURFACE-A2 fix (2026-05-19, v0.3-gating
                // SOUNDNESS BUG per supervisor ratify): pre-populate
                // `field_byte_offsets` from the program's
                // `type_schema_registry` so trait-impl method bodies (and
                // any function that reads typed-object fields without
                // emitting a local `ObjectStore`) can resolve field byte
                // offsets at JIT compile time. Without this pre-pass,
                // `try_resolve_field_byte_offset` returns `None` for impl
                // bodies and `Place::Field` falls through to
                // `jit_get_prop`, whose `heap_kind(obj_bits)` predicate
                // returns `None` under ADR-006 §2.7.5 raw `Box::into_raw`
                // typed-object carriers — empirically returning `TAG_NULL`
                // for every `self.field` read (`vm_trait_method_self_field
                // _access_n0` reproducer's garbage NaN-bits root cause).
                //
                // Per ADR-006 §2.7.5 producer-side stamp: schema field
                // positions are stamped at AST→bytecode-compile time in
                // the canonical schema registry. The JIT consumes the
                // stamp through `populate_field_byte_offsets_from_schemas`
                // — a derived index, not a runtime decode.
                mir_compiler
                    .populate_field_byte_offsets_from_schemas(&program.type_schema_registry);
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
                mir_compiler.validate_shared_cell_kinds()?;
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
                mir_compiler.initialize_shared_local_slots()?;

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
                            let kind = mir_compiler.local_storage_kind(mir_slot);
                            let param_val = entry_params[native_idx];
                            let converted = match kind {
                                shape_vm::type_tracking::NativeKind::Float64 => mir_compiler
                                    .builder
                                    .ins()
                                    .bitcast(types::F64, MemFlags::new(), param_val),
                                shape_vm::type_tracking::NativeKind::Int32
                                | shape_vm::type_tracking::NativeKind::UInt32 => {
                                    mir_compiler.builder.ins().ireduce(types::I32, param_val)
                                }
                                shape_vm::type_tracking::NativeKind::Bool
                                | shape_vm::type_tracking::NativeKind::Int8
                                | shape_vm::type_tracking::NativeKind::UInt8 => {
                                    mir_compiler.builder.ins().ireduce(types::I8, param_val)
                                }
                                shape_vm::type_tracking::NativeKind::Int16
                                | shape_vm::type_tracking::NativeKind::UInt16 => {
                                    mir_compiler.builder.ins().ireduce(types::I16, param_val)
                                }
                                _ => param_val,
                            };
                            mir_compiler.builder.def_var(var, converted);
                        }
                    }
                }
                mir_compiler.compile_body()?;
                tracing::debug!(
                    target: "shape_jit",
                    func_name = %func.name,
                    "jit-mir compiled function via MirToIR",
                );
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
        use super::accessors::{function_body_module_binding_accesses, preflight_instructions};

        maybe_emit_numeric_metrics(program);

        let module_binding_accesses = function_body_module_binding_accesses(program);
        // WHOLE-PROGRAM-BAIL[construct]: function-body-module-binding — W39 F1: module bindings are not MIR places, so a native top-level plus an interpreted function would read an unsynchronized module-binding array
        if let Some(first) = module_binding_accesses.first() {
            shape_vm::native_witness::record_program_fallback(
                shape_vm::native_witness::FallbackReasonClass::ModuleBindingFunctionBody,
                format!(
                    "function `{}` contains {:?} at instruction {}",
                    first.function_name, first.opcode, first.instruction_index
                ),
            );
            return Err(format!(
                "W39 F1 module-binding function-body SURFACE (ADR-006 §2.7.14): \
                 function '{}' contains {:?} at bytecode instruction {}. \
                 Module bindings are not MIR places, so the JIT function-body \
                 lowering has no compile-time side table for this storage. \
                 Running native top-level code and then interpreting such a \
                 function through the trampoline VM would read an unsynchronized \
                 module-binding array (observed VM=100 / JIT=0 on \
                 f1-shared-module-binding.shape). Whole-program deopting to the \
                 bytecode interpreter via the existing `[jit-fallback]` path \
                 preserves VM == JIT semantics until module-binding lowering is \
                 rebuilt with static metadata. total_accesses={}",
                first.function_name,
                first.opcode,
                first.instruction_index,
                module_binding_accesses.len(),
            ));
        }

        // Phase 1: Per-function preflight to classify each function.
        // A function is JIT-compatible if its bytecode passes instruction
        // preflight OR it has MIR data that passes MirToIR preflight.
        // MirToIR is the compilation path — bytecode preflight only gates eligibility.
        let mut jit_compatible: Vec<bool> = Vec::with_capacity(program.functions.len());

        for (_idx, func) in program.functions.iter().enumerate() {
            if func.body_length == 0 && func.mir_data.is_none() {
                // #117 / R15: a covered fallback is a truthful record, not a
                // missing witness — say which function and why.
                shape_vm::native_witness::record_function_fallback(
                    _idx,
                    shape_vm::native_witness::FallbackReasonClass::NoCompilableBody,
                    format!("`{}` has neither a bytecode body nor MIR data", func.name),
                );
                jit_compatible.push(false);
                continue;
            }
            let func_end = func.entry_point + func.body_length;
            let instructions = &program.instructions[func.entry_point..func_end];
            let report = preflight_instructions(instructions);
            let bytecode_ok = report.can_jit();
            let mir_ok = func
                .mir_data
                .as_ref()
                .is_some_and(|md| crate::mir_compiler::preflight(md).can_compile);
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
            if !(bytecode_ok || mir_ok) {
                // #117 / R15. `vm_only_opcodes` is the dominant class: it names
                // the opcode the JIT deliberately does not lower (`CallForeign`,
                // an `as` cast, an outer-scope `var` cell). An unsupported
                // builtin is reported when that is the only blocker.
                let class = if report.vm_only_opcodes.is_empty() {
                    shape_vm::native_witness::FallbackReasonClass::UnsupportedBuiltin
                } else {
                    shape_vm::native_witness::FallbackReasonClass::VmOnlyOpcode
                };
                shape_vm::native_witness::record_function_fallback(
                    _idx,
                    class,
                    format!("`{}`: {}", func.name, report.blockers_summary()),
                );
            }
            jit_compatible.push(bytecode_ok || mir_ok);
        }

        // Phase 1a-bis (ADR-018 §2 / #187): demote functions carrying a
        // residual construct — `?`, `??`, an inlined imported `pub const`, a
        // direct imported-stdlib call, or a §2.7.30 escape-promoted reference
        // return. Each has a diagnosed VM/JIT divergence, so its owner never
        // runs native. Before #187 the JIT refused the ENTIRE program on any
        // of them; the refusal is unchanged in strength and narrowed in scope
        // to the function that actually holds the construct. Top-level
        // residuals are still whole-program (`JITExecutor::execute_with_jit`)
        // because top-level IS the entry the JIT compiles as `main`.
        //
        // The demoted function gets a null `function_table` slot and an
        // `Interpreted` entry below, so every call to it routes through
        // `dispatch_call_via_trampoline_vm` into the bytecode interpreter —
        // the same interpreter the whole-program deopt used to reach.
        //
        // `Program`-scoped residuals never reach here: `execute_with_jit`
        // refuses the whole program before compiling. Demoting only their
        // owner would be unsound for the reason each one records.
        for idx in 0..program.functions.len() {
            if !program.jit_residuals.function_is_residual_bearing(idx) {
                continue;
            }
            jit_compatible[idx] = false;
            for residual in program.jit_residuals.for_function(idx) {
                // #117 / R15: a demoted function is a COVERED fallback, not a
                // missing witness. Recording it is what lets a consumer assert
                // "this one fell back" as a positive fact, and what stops the
                // sibling's native claim being read as covering it too.
                shape_vm::native_witness::record_function_fallback(
                    idx,
                    residual.witness_class(),
                    residual.reason(),
                );
                tracing::info!(
                    target: "shape_jit::fallback",
                    function = %program.functions[idx].name,
                    function_index = idx,
                    residual = residual.stable_id(),
                    reason = residual.reason(),
                    "jit-deopt-function: interpreted, siblings keep native code",
                );
            }
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
            // WHOLE-PROGRAM-BAIL[construct]: main-code-preflight — top-level (non-function-body) instructions failed preflight
            if !main_report.can_jit() {
                shape_vm::native_witness::record_program_fallback(
                    shape_vm::native_witness::FallbackReasonClass::MainCodeUnsupportedConstruct,
                    main_report.blockers_summary(),
                );
                return Err(format!(
                    "Main code contains unsupported constructs: {:?}",
                    main_report
                ));
            }
        }

        // v0.3 WS-6: a generic free function specialized on a struct type
        // argument (`fn id<T>(x: T) -> T` called as `id(P { .. })`) produces
        // a `<base>::struct_<name>` specialization. The JIT MIR codegen for
        // a struct value flowing out of such a specialization is currently
        // unsound — the returned `HeapKind::TypedObject` handle is
        // mishandled when the result is stored to a slot and a field is
        // later read, producing a use-after-free. The bytecode VM handles
        // this case correctly. Per the CLAUDE.md surface-and-stop discipline
        // (refuse what cannot be lowered soundly rather than emit crashing
        // native code), surface here so `--mode jit` cleanly falls back to
        // the interpreter for the whole program. Enum / Option / Result /
        // Array / HashMap monomorphizations are unaffected — only the
        // struct-typed free-function specialization is gated. (Generic
        // struct args were rejected outright at the compile stage before
        // WS-6, so this is a strict improvement: such programs now run
        // correctly on the interpreter rather than failing to compile.)
        // WHOLE-PROGRAM-BAIL[construct]: generic-struct-specialization — WS-6: JIT struct-value codegen for a `<base>::struct_<name>` specialization is unsound (use-after-free on a later field read)
        if program
            .functions
            .iter()
            .any(|func| func.name.contains("::struct_"))
        {
            shape_vm::native_witness::record_program_fallback(
                shape_vm::native_witness::FallbackReasonClass::GenericStructSpecialization,
                "a generic free function is specialized on a struct type argument \
                 (`<base>::struct_<name>`)",
            );
            return Err(
                "WS-6 surface-and-stop: program uses a generic free function \
                 specialized on a struct type argument; the JIT struct-value \
                 codegen for that specialization is not yet sound — falling \
                 back to the bytecode interpreter"
                    .to_string(),
            );
        }

        // Phase 2: Pre-declare ALL functions (both JIT and interpreted) in
        // Cranelift so that JIT functions can call other JIT functions.
        // Interpreted functions get declared too (for uniform call tables)
        // but won't have a body defined - they'll use the trampoline.
        let mut user_func_arities: HashMap<u16, u16> = HashMap::new();
        let mut user_func_ids: HashMap<u16, cranelift_module::FuncId> = HashMap::new();
        let mut user_func_return_kinds: HashMap<u16, shape_vm::type_tracking::NativeKind> =
            HashMap::new();

        for (idx, func) in program.functions.iter().enumerate() {
            if let Some(return_kind) = func.frame_descriptor.as_ref().and_then(|fd| fd.return_kind)
            {
                user_func_return_kinds.insert(idx as u16, return_kind);
            }
            if !jit_compatible[idx] {
                user_func_arities.insert(idx as u16, func.arity);
                continue;
            }
            // Use function index in the name to avoid collisions between
            // closures with the same auto-generated name but different arities
            // (e.g., multiple __closure_0 from different stdlib modules).
            let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
            let abi = prove_user_function_abi(self.module.make_signature(), func)?;
            let func_id = self
                .module
                .declare_function(&func_name, Linkage::Local, &abi.signature)
                .map_err(|e| format!("Failed to pre-declare function {}: {}", func.name, e))?;
            user_func_ids.insert(idx as u16, func_id);
            user_func_arities.insert(idx as u16, abi.native_arity);
        }

        // Phase 3: Compile main strategy body.
        let main_func_id = self.compile_strategy_with_user_funcs(
            name,
            program,
            &user_func_ids,
            &user_func_arities,
            &user_func_return_kinds,
        )?;

        // Phase 4: Compile only JIT-compatible function bodies.
        // Functions that fail to compile are demoted to interpreted fallback.
        let mut compile_failures = Vec::<(String, String)>::new();
        for (idx, func) in program.functions.iter().enumerate() {
            if jit_compatible[idx] || func.mir_data.is_some() {
                tracing::debug!(
                    target: "shape_jit",
                    idx,
                    func_name = %func.name,
                    jit_compat = jit_compatible[idx],
                    has_mir = func.mir_data.is_some(),
                    "jit-mir per-function classification",
                );
            }
            if !jit_compatible[idx] {
                continue;
            }
            let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
            if func.mir_data.is_some() {
                tracing::debug!(
                    target: "shape_jit",
                    idx,
                    func_name = %func.name,
                    "jit-mir compiling function",
                );
            }
            if let Err(e) = self.compile_function_with_user_funcs(
                &func_name,
                program,
                idx,
                &user_func_ids,
                &user_func_arities,
                &user_func_return_kinds,
            ) {
                tracing::debug!(
                    target: "shape_jit",
                    func_name = %func.name,
                    error = %e,
                    "jit-mir compile failed",
                );
                // Leave the failed body undefined. Finalization distinguishes an
                // unreferenced demotion from a reachable compile-stage refusal;
                // see `program_finalize` for the exactly-once rationale.
                shape_vm::native_witness::record_function_fallback(
                    idx,
                    shape_vm::native_witness::FallbackReasonClass::FunctionCodegenFailed,
                    format!("`{}`: {}", func.name, e),
                );
                compile_failures.push((func_name, e));
                jit_compatible[idx] = false;
            }
        }

        // Undefined failed bodies are harmless when unreferenced. If a native
        // relocation reaches one, finalization returns the originating refusal
        // before any native side effect can run.
        finalize_program_definitions(&mut self.module, &compile_failures)?;

        let main_code_ptr = self.module.get_finalized_function(main_func_id);
        self.compiled_functions
            .insert(name.to_string(), main_code_ptr);
        // #117 / R15 installation event for the top-level unit, taken at the
        // finalization site rather than inferred from "compilation succeeded".
        // The witness stores no code address, so #209's per-module page leak
        // cannot make a recorded installation outlive its meaning.
        shape_vm::native_witness::record_installation(program.functions.len());

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
                    // #117 / R15 installation event: native code for this unit
                    // is finalized and linked into the function table.
                    shape_vm::native_witness::record_installation(idx);
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
