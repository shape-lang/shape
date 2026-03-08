//! Program compilation and execution logic.
//!
//! Contains the VM execution loop, module_binding variable synchronization,
//! snapshot resume, compilation pipeline, and trait implementations
//! for `ProgramExecutor` and `ExpressionEvaluator`.

use std::sync::Arc;

use crate::VMExecutionResult;
use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use crate::configuration::BytecodeExecutor;
use crate::executor::SNAPSHOT_FUTURE_ID;
use crate::executor::debugger_integration::DebuggerIntegration;
use crate::executor::{ForeignFunctionHandle, VMConfig, VirtualMachine};
use shape_value::{HeapValue, ValueWord};

use shape_ast::Program;
use shape_runtime::context::ExecutionContext;
use shape_runtime::engine::{ExecutionType, ProgramExecutor, ShapeEngine};
use shape_runtime::error::Result;
use shape_runtime::event_queue::{SuspensionState, WaitCondition};
use shape_value::{EnumPayload, EnumValue};
use shape_wire::{AnyError as WireAnyError, WireValue, render_any_error_plain};

impl BytecodeExecutor {
    /// Load variables from ExecutionContext and ModuleBindingRegistry into VM module_bindings
    fn load_module_bindings_from_context(
        vm: &mut VirtualMachine,
        ctx: &ExecutionContext,
        module_binding_registry: &Arc<std::sync::RwLock<shape_runtime::ModuleBindingRegistry>>,
        module_binding_names: &[String],
    ) {
        for (idx, name) in module_binding_names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }

            // Check ModuleBindingRegistry first
            if let Some(value) = module_binding_registry.read().unwrap().get_by_name(name) {
                // Skip functions - they're already compiled into the bytecode
                if value
                    .as_heap_ref()
                    .is_some_and(|h| matches!(h, HeapValue::Closure { .. }))
                {
                    continue;
                }
                vm.set_module_binding(idx, value);
                continue;
            }

            // Fall back to ExecutionContext
            if let Ok(Some(value)) = ctx.get_variable(name) {
                // Skip functions - they're already compiled
                if value
                    .as_heap_ref()
                    .is_some_and(|h| matches!(h, HeapValue::Closure { .. }))
                {
                    continue;
                }
                vm.set_module_binding(idx, value);
            }
        }
    }

    /// Save VM module_bindings back to ExecutionContext
    fn save_module_bindings_to_context(
        vm: &VirtualMachine,
        ctx: &mut ExecutionContext,
        module_binding_names: &[String],
    ) {
        let module_bindings = vm.module_binding_values();
        for (idx, name) in module_binding_names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            if idx < module_bindings.len() {
                let value = module_bindings[idx].clone();
                // Use set_variable which creates or updates the variable
                // Note: This preserves existing format hints since set_variable
                // only updates the value, not the metadata
                let _ = ctx.set_variable(name, value);
            }
        }
    }

    /// Extract format hints from AST and store in ExecutionContext
    ///
    /// Format hints are now handled via the meta system with type aliases.
    /// This function is kept for API compatibility but is a no-op.
    fn extract_and_store_format_hints(_program: &Program, _ctx: Option<&mut ExecutionContext>) {
        // No-op: legacy @ format hints removed, use type aliases with meta instead
    }

    /// Shared execution loop for both execute_program and resume_snapshot.
    ///
    /// Runs the VM in a suspend/resume loop, handling snapshot suspension,
    /// Ctrl+C interrupts, and errors. If `initial_push` is Some, pushes
    /// that value onto the VM stack before the first execution cycle.
    fn run_vm_loop(
        &self,
        vm: &mut VirtualMachine,
        engine: &mut ShapeEngine,
        module_binding_names: &[String],
        bytecode_for_snapshot: &BytecodeProgram,
        initial_push: Option<ValueWord>,
    ) -> Result<ValueWord> {
        engine.get_runtime_mut().clear_last_runtime_error();

        let mut first_run = initial_push.is_some();
        let initial_value = initial_push;

        let result = loop {
            let runtime = engine.get_runtime_mut();
            let mut ctx = runtime.persistent_context_mut();

            if first_run {
                if let Some(ref val) = initial_value {
                    let _ = vm.push_vw(val.clone());
                }
                first_run = false;
            }

            match vm.execute_with_suspend(ctx.as_deref_mut()) {
                Ok(VMExecutionResult::Completed(value)) => break value,
                Ok(VMExecutionResult::Suspended {
                    future_id,
                    resume_ip,
                }) => {
                    let wait = if future_id == SNAPSHOT_FUTURE_ID {
                        WaitCondition::Snapshot
                    } else {
                        WaitCondition::Future { id: future_id }
                    };

                    if let Some(ctx) = ctx.as_mut() {
                        Self::save_module_bindings_to_context(vm, ctx, module_binding_names);
                        ctx.set_suspension_state(SuspensionState::new(wait, resume_ip));
                    }

                    drop(ctx);

                    if future_id == SNAPSHOT_FUTURE_ID {
                        let store = engine.snapshot_store().ok_or_else(|| {
                            shape_runtime::error::ShapeError::RuntimeError {
                                message: "Snapshot store not configured".to_string(),
                                location: None,
                            }
                        })?;
                        let vm_snapshot = vm.snapshot(store).map_err(|e| {
                            shape_runtime::error::ShapeError::RuntimeError {
                                message: e.to_string(),
                                location: None,
                            }
                        })?;
                        let vm_hash = engine.store_snapshot_blob(&vm_snapshot)?;
                        let bytecode_hash = engine.store_snapshot_blob(bytecode_for_snapshot)?;
                        let snapshot_hash =
                            engine.snapshot_with_hashes(Some(vm_hash), Some(bytecode_hash))?;

                        let hash_str_nb =
                            ValueWord::from_string(Arc::new(snapshot_hash.hex().to_string()));
                        let hash_nb = vm
                            .create_typed_enum_nb("Snapshot", "Hash", vec![hash_str_nb.clone()])
                            .unwrap_or_else(|| {
                                let hash_nb = ValueWord::from_string(Arc::new(
                                    snapshot_hash.hex().to_string(),
                                ));
                                ValueWord::from_enum(EnumValue {
                                    enum_name: "Snapshot".to_string(),
                                    variant: "Hash".to_string(),
                                    payload: EnumPayload::Tuple(vec![hash_nb]),
                                })
                            });
                        let _ = vm.push_vw(hash_nb);
                        continue;
                    }

                    break ValueWord::none();
                }
                Err(shape_value::VMError::Interrupted) => {
                    drop(ctx);
                    let snapshot_hash = if let Some(store) = engine.snapshot_store() {
                        match vm.snapshot(store) {
                            Ok(vm_snapshot) => {
                                let vm_hash = engine.store_snapshot_blob(&vm_snapshot).ok();
                                let bc_hash =
                                    engine.store_snapshot_blob(bytecode_for_snapshot).ok();
                                if let (Some(vh), Some(bh)) = (vm_hash, bc_hash) {
                                    engine
                                        .snapshot_with_hashes(Some(vh), Some(bh))
                                        .ok()
                                        .map(|h| h.hex().to_string())
                                } else {
                                    None
                                }
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    return Err(shape_runtime::error::ShapeError::Interrupted { snapshot_hash });
                }
                Err(e) => {
                    let mut location = vm.last_error_line().map(|line| {
                        let mut loc = shape_ast::error::SourceLocation::new(line as usize, 1);
                        if let Some(file) = vm.last_error_file() {
                            loc = loc.with_file(file.to_string());
                        }
                        loc
                    });
                    let mut message = e.to_string();
                    let mut runtime_error_payload = None;

                    if let Some(any_error_nb) = vm.take_last_uncaught_exception() {
                        let any_error_wire = if let Some(exec_ctx) = ctx.as_deref() {
                            shape_runtime::wire_conversion::nb_to_wire(&any_error_nb, exec_ctx)
                        } else {
                            let fallback_ctx =
                                shape_runtime::context::ExecutionContext::new_empty();
                            shape_runtime::wire_conversion::nb_to_wire(&any_error_nb, &fallback_ctx)
                        };
                        runtime_error_payload = Some(any_error_wire.clone());

                        if let Some(rendered) = render_any_error_plain(&any_error_wire) {
                            message = rendered;
                        }

                        if let Some(parsed) = WireAnyError::from_wire(&any_error_wire)
                            && let Some(frame) = parsed.primary_location()
                            && let Some(line) = frame.line
                        {
                            let mut loc = shape_ast::error::SourceLocation::new(
                                line,
                                frame.column.unwrap_or(1),
                            );
                            if let Some(file) = frame.file {
                                loc = loc.with_file(file);
                            }
                            location = Some(loc);
                        }
                    }

                    drop(ctx);
                    engine
                        .get_runtime_mut()
                        .set_last_runtime_error(runtime_error_payload);

                    return Err(shape_runtime::error::ShapeError::RuntimeError {
                        message,
                        location,
                    });
                }
            }
        };

        Ok(result)
    }

    /// Finalize execution: save module_bindings back to context and convert result to wire format.
    fn finalize_result(
        vm: &VirtualMachine,
        engine: &mut ShapeEngine,
        module_binding_names: &[String],
        result_nb: &ValueWord,
    ) -> (
        WireValue,
        Option<shape_wire::metadata::TypeInfo>,
        Option<serde_json::Value>,
        Option<String>,
        Option<String>,
    ) {
        let (content_json, content_html, content_terminal) =
            shape_runtime::wire_conversion::nb_extract_content(result_nb);

        let runtime = engine.get_runtime_mut();
        let mut ctx = runtime.persistent_context_mut();
        let mut type_info = None;
        let wire_value = if let Some(ctx) = ctx.as_mut() {
            Self::save_module_bindings_to_context(vm, ctx, module_binding_names);
            let type_name = result_nb.type_name();
            type_info = Some(
                shape_runtime::wire_conversion::nb_to_envelope(result_nb, type_name, ctx).type_info,
            );
            shape_runtime::wire_conversion::nb_to_wire(result_nb, ctx)
        } else {
            WireValue::Null
        };
        (
            wire_value,
            type_info,
            content_json,
            content_html,
            content_terminal,
        )
    }

    /// Resume execution from a snapshot
    pub fn resume_snapshot(
        &self,
        engine: &mut ShapeEngine,
        vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        mut bytecode: BytecodeProgram,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        let store = engine.snapshot_store().ok_or_else(|| {
            shape_runtime::error::ShapeError::RuntimeError {
                message: "Snapshot store not configured".to_string(),
                location: None,
            }
        })?;

        // Reconstruct VM from snapshot
        let mut vm =
            VirtualMachine::from_snapshot(bytecode.clone(), &vm_snapshot, store).map_err(|e| {
                shape_runtime::error::ShapeError::RuntimeError {
                    message: e.to_string(),
                    location: None,
                }
            })?;
        vm.set_interrupt(self.interrupt.clone());

        // Register extensions and built-in module_bindings
        for ext in &self.extensions {
            vm.register_extension(ext.clone());
        }
        vm.populate_module_objects();

        let module_binding_names = bytecode.module_binding_names.clone();
        let bytecode_for_snapshot = bytecode;

        // Build the Snapshot::Resumed marker to push before first execution cycle
        let resumed = vm
            .create_typed_enum_nb("Snapshot", "Resumed", vec![])
            .unwrap_or_else(|| {
                ValueWord::from_enum(EnumValue {
                    enum_name: "Snapshot".to_string(),
                    variant: "Resumed".to_string(),
                    payload: EnumPayload::Unit,
                })
            });

        let result = self.run_vm_loop(
            &mut vm,
            engine,
            &module_binding_names,
            &bytecode_for_snapshot,
            Some(resumed),
        )?;
        let (wire_value, type_info, content_json, content_html, content_terminal) =
            Self::finalize_result(&vm, engine, &module_binding_names, &result);

        Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value,
            type_info,
            execution_type: ExecutionType::Script,
            content_json,
            content_html,
            content_terminal,
        })
    }

    /// Compile a program to bytecode without executing it.
    ///
    /// This performs the same compilation pipeline as `execute_program`
    /// (merging core stdlib, extensions, virtual modules) but stops
    /// before creating a VM or executing.
    pub(crate) fn compile_program_impl(
        &self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<BytecodeProgram> {
        let source_for_compilation = engine.current_source().map(|s| s.to_string());

        // Check bytecode cache before expensive compilation
        if let (Some(cache), Some(source)) = (&self.bytecode_cache, &source_for_compilation) {
            if let Some(cached) = cache.get(source) {
                return Ok(cached);
            }
        }

        let runtime = engine.get_runtime_mut();

        let known_bindings: Vec<String> = if let Some(ctx) = runtime.persistent_context() {
            let names = ctx.root_scope_binding_names();
            if names.is_empty() {
                crate::stdlib::core_binding_names()
            } else {
                names
            }
        } else {
            crate::stdlib::core_binding_names()
        };

        Self::extract_and_store_format_hints(program, runtime.persistent_context_mut());

        let module_binding_registry = runtime.module_binding_registry();
        let imported_program = Self::create_program_from_imports(&module_binding_registry)?;

        let mut merged_program = imported_program;
        merged_program.items.extend(program.items.clone());
        crate::module_resolution::prepend_prelude_items(&mut merged_program);
        self.append_imported_module_items(&mut merged_program);

        let mut compiler = BytecodeCompiler::new();
        compiler.register_known_bindings(&known_bindings);

        if !self.extensions.is_empty() {
            compiler.extension_registry = Some(Arc::new(self.extensions.clone()));
        }

        if let Ok(cwd) = std::env::current_dir() {
            compiler.set_source_dir(cwd);
        }

        let bytecode = if let Some(source) = &source_for_compilation {
            compiler.compile_with_source(&merged_program, source)?
        } else {
            compiler.compile(&merged_program)?
        };

        // Store in bytecode cache (best-effort, ignore errors)
        if let (Some(cache), Some(source)) = (&self.bytecode_cache, &source_for_compilation) {
            let _ = cache.put(source, &bytecode);
        }

        Ok(bytecode)
    }

    /// Compile a program with the same pipeline as execution, but do not run it.
    ///
    /// The returned bytecode includes tooling artifacts such as
    /// `expanded_function_defs` for comptime inspection.
    pub fn compile_program_for_inspection(
        &self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<BytecodeProgram> {
        self.compile_program_impl(engine, program)
    }

    /// Recompile source and resume from a snapshot.
    ///
    /// Compiles the new program to bytecode, finds the snapshot() call
    /// position in both old and new bytecodes, adjusts the VM snapshot's
    /// instruction pointer, then resumes execution from the snapshot point
    /// using the new bytecode.
    pub fn recompile_and_resume(
        &self,
        engine: &mut ShapeEngine,
        mut vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        old_bytecode: BytecodeProgram,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        use crate::bytecode::{BuiltinFunction, OpCode, Operand};

        let new_bytecode = self.compile_program_impl(engine, program)?;

        // Find snapshot() call positions (BuiltinCall with Snapshot operand) in old bytecode
        let old_snapshot_ips: Vec<usize> = old_bytecode
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| {
                instr.opcode == OpCode::BuiltinCall
                    && matches!(
                        &instr.operand,
                        Some(Operand::Builtin(BuiltinFunction::Snapshot))
                    )
            })
            .map(|(i, _)| i)
            .collect();

        // Same for new bytecode
        let new_snapshot_ips: Vec<usize> = new_bytecode
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| {
                instr.opcode == OpCode::BuiltinCall
                    && matches!(
                        &instr.operand,
                        Some(Operand::Builtin(BuiltinFunction::Snapshot))
                    )
            })
            .map(|(i, _)| i)
            .collect();

        // VmSnapshot.ip points to the instruction AFTER the BuiltinCall(Snapshot).
        // Find which snapshot() in the old bytecode corresponds to the saved IP.
        let old_snapshot_idx = old_snapshot_ips
            .iter()
            .position(|&ip| ip + 1 == vm_snapshot.ip)
            .ok_or_else(|| shape_runtime::error::ShapeError::RuntimeError {
                message: format!(
                    "Could not find snapshot() call in original bytecode at IP {} \
                     (snapshot calls found at: {:?})",
                    vm_snapshot.ip, old_snapshot_ips
                ),
                location: None,
            })?;

        // Map to the corresponding snapshot() in the new bytecode (by ordinal)
        let &new_snapshot_ip = new_snapshot_ips.get(old_snapshot_idx).ok_or_else(|| {
            shape_runtime::error::ShapeError::RuntimeError {
                message: format!(
                    "Recompiled source has {} snapshot() call(s) but resuming from \
                     snapshot #{} (0-indexed)",
                    new_snapshot_ips.len(),
                    old_snapshot_idx
                ),
                location: None,
            }
        })?;

        // Check for non-empty call stack — recompile mode can only adjust the
        // top-level IP; return addresses inside function frames would be stale.
        if !vm_snapshot.call_stack.is_empty() {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: "Recompile-and-resume is only supported when snapshot() is called \
                          at the top level (call stack is non-empty)"
                    .to_string(),
                location: None,
            });
        }

        // Adjust the snapshot's IP to point after the snapshot() call in new bytecode
        vm_snapshot.ip = new_snapshot_ip + 1;

        eprintln!(
            "Remapped snapshot IP: {} -> {} (snapshot #{})",
            old_snapshot_ips[old_snapshot_idx] + 1,
            vm_snapshot.ip,
            old_snapshot_idx
        );

        self.resume_snapshot(engine, vm_snapshot, new_bytecode)
    }
}

impl shape_runtime::engine::ExpressionEvaluator for BytecodeExecutor {
    fn eval_statements(
        &self,
        stmts: &[shape_ast::Statement],
        ctx: &mut ExecutionContext,
    ) -> Result<ValueWord> {
        // Wrap statements as a program
        let items: Vec<shape_ast::Item> = stmts
            .iter()
            .map(|s| shape_ast::Item::Statement(s.clone(), shape_ast::Span::DUMMY))
            .collect();
        let mut program = Program {
            items,
            docs: shape_ast::ast::ProgramDocs::default(),
        };

        // Inject prelude and resolve imports
        crate::module_resolution::prepend_prelude_items(&mut program);

        // Compile and execute
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program)?;

        let module_binding_names = bytecode.module_binding_names.clone();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        // Register extensions before built-in module_bindings so extensions are also available
        for ext in &self.extensions {
            vm.register_extension(ext.clone());
        }
        vm.populate_module_objects();

        // Load variables from context
        for (idx, name) in module_binding_names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            if let Ok(Some(value)) = ctx.get_variable(name) {
                let is_closure = value
                    .as_heap_ref()
                    .is_some_and(|h| matches!(h, HeapValue::Closure { .. }));
                if !is_closure {
                    vm.set_module_binding(idx, value);
                }
            }
        }

        let result_nb =
            vm.execute(Some(ctx))
                .map_err(|e| shape_runtime::error::ShapeError::RuntimeError {
                    message: e.to_string(),
                    location: None,
                })?;

        // Save back modified module_bindings
        Self::save_module_bindings_to_context(&vm, ctx, &module_binding_names);

        Ok(result_nb.clone())
    }

    fn eval_expr(&self, expr: &shape_ast::Expr, ctx: &mut ExecutionContext) -> Result<ValueWord> {
        // Wrap expression as an expression statement
        let stmt = shape_ast::Statement::Expression(expr.clone(), shape_ast::Span::DUMMY);
        self.eval_statements(&[stmt], ctx)
    }
}

impl ProgramExecutor for BytecodeExecutor {
    fn execute_program(
        &self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        // Capture source text before getting runtime reference (for error messages)
        let source_for_compilation = engine.current_source().map(|s| s.to_string());

        // Phase 1: Compile and prepare bytecode (borrows runtime, then drops it)
        let (mut vm, module_binding_names, bytecode_for_snapshot) = {
            let runtime = engine.get_runtime_mut();

            // Get known module_binding variables from previous REPL sessions
            let known_bindings: Vec<String> = if let Some(ctx) = runtime.persistent_context() {
                ctx.root_scope_binding_names()
            } else {
                Vec::new()
            };

            // Extract format hints from variable declarations BEFORE compilation
            // This preserves metadata that bytecode doesn't carry
            Self::extract_and_store_format_hints(program, runtime.persistent_context_mut());

            // Extract imported functions from ModuleBindingRegistry and add them to the program
            let module_binding_registry = runtime.module_binding_registry();
            let imported_program = Self::create_program_from_imports(&module_binding_registry)?;

            // Merge imported functions into the main program
            let mut merged_program = imported_program;
            merged_program.items.extend(program.items.clone());
            crate::module_resolution::prepend_prelude_items(&mut merged_program);
            self.append_imported_module_items(&mut merged_program);

            // Compile AST to Bytecode with knowledge of existing module_bindings
            let mut compiler = BytecodeCompiler::new();
            compiler.register_known_bindings(&known_bindings);

            // Wire extension registry into compiler for comptime execution
            if !self.extensions.is_empty() {
                compiler.extension_registry = Some(Arc::new(self.extensions.clone()));
            }

            // Set source directory for compile-time schema validation in data-source calls
            if let Ok(cwd) = std::env::current_dir() {
                compiler.set_source_dir(cwd);
            }

            // Use compile_with_source if source text is available for better error messages
            let bytecode = if let Some(source) = &source_for_compilation {
                compiler.compile_with_source(&merged_program, source)?
            } else {
                compiler.compile(&merged_program)?
            };

            // Save the module_binding names for syncing (includes both new and existing)
            let module_binding_names = bytecode.module_binding_names.clone();

            // Execute Bytecode
            let mut vm = VirtualMachine::new(VMConfig::default());
            vm.set_interrupt(self.interrupt.clone());
            let bytecode_for_snapshot = bytecode.clone();
            vm.load_program(bytecode);
            for ext in &self.extensions {
                vm.register_extension(ext.clone());
            }
            vm.populate_module_objects();

            // Drop stale links from previous runs before relinking this program's foreign table.
            vm.foreign_fn_handles.clear();

            // Link foreign functions: compile foreign function bodies via language runtime extensions
            if !vm.program.foreign_functions.is_empty() {
                let entries = vm.program.foreign_functions.clone();
                let mut handles = Vec::with_capacity(entries.len());
                let mut native_library_cache: std::collections::HashMap<
                    String,
                    std::sync::Arc<libloading::Library>,
                > = std::collections::HashMap::new();
                let runtime_ctx = runtime.persistent_context();

                for (idx, entry) in entries.iter().enumerate() {
                    if let Some(native_spec) = &entry.native_abi {
                        let linked = crate::executor::native_abi::link_native_function(
                            native_spec,
                            &vm.program.native_struct_layouts,
                            &mut native_library_cache,
                        )
                        .map_err(|e| {
                            shape_runtime::error::ShapeError::RuntimeError {
                                message: format!(
                                    "Failed to link native function '{}': {}",
                                    entry.name, e
                                ),
                                location: None,
                            }
                        })?;

                        // Native ABI path is static by contract.
                        vm.program.foreign_functions[idx].dynamic_errors = false;
                        handles.push(Some(ForeignFunctionHandle::Native(std::sync::Arc::new(
                            linked,
                        ))));
                        continue;
                    }

                    let Some(ctx) = runtime_ctx.as_ref() else {
                        return Err(shape_runtime::error::ShapeError::RuntimeError {
                            message: format!(
                                "No runtime context available to link foreign function '{}'",
                                entry.name
                            ),
                            location: None,
                        });
                    };

                    if let Some(lang_runtime) = ctx.get_language_runtime(&entry.language) {
                        // Override the compile-time default with the actual
                        // runtime's error model now that we have the extension.
                        vm.program.foreign_functions[idx].dynamic_errors =
                            lang_runtime.has_dynamic_errors();

                        let compiled = lang_runtime.compile(
                            &entry.name,
                            &entry.body_text,
                            &entry.param_names,
                            &entry.param_types,
                            entry.return_type.as_deref(),
                            entry.is_async,
                        )?;
                        handles.push(Some(ForeignFunctionHandle::Runtime {
                            runtime: lang_runtime,
                            compiled,
                        }));
                    } else {
                        return Err(shape_runtime::error::ShapeError::RuntimeError {
                            message: format!(
                                "No language runtime registered for '{}'. \
                                 Install the {} extension to use `fn {} ...` blocks.",
                                entry.language, entry.language, entry.language
                            ),
                            location: None,
                        });
                    }
                }
                vm.foreign_fn_handles = handles;
            }

            let module_binding_registry = runtime.module_binding_registry();
            let mut ctx = runtime.persistent_context_mut();

            // Load existing variables from context and module_binding registry into VM before execution
            if let Some(ctx) = ctx.as_mut() {
                Self::load_module_bindings_from_context(
                    &mut vm,
                    ctx,
                    &module_binding_registry,
                    &module_binding_names,
                );
            }

            (vm, module_binding_names, bytecode_for_snapshot)
        }; // runtime borrow ends here

        // Phase 2: Execute bytecode (re-borrows runtime for ctx)
        let result = self.run_vm_loop(
            &mut vm,
            engine,
            &module_binding_names,
            &bytecode_for_snapshot,
            None,
        )?;

        // Phase 3: Save VM module_bindings back to context after execution
        let (wire_value, type_info, content_json, content_html, content_terminal) =
            Self::finalize_result(&vm, engine, &module_binding_names, &result);

        Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value,
            type_info,
            execution_type: ExecutionType::Script,
            content_json,
            content_html,
            content_terminal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::OpCode;
    use crate::bytecode::Operand;
    use crate::executor::VirtualMachine;
    use shape_runtime::snapshot::{SnapshotStore, VmSnapshot};

    #[test]
    fn snapshot_resume_keeps_snapshot_enum_matching_after_bytecode_roundtrip() {
        let source = r#"
from std::core::snapshot use { Snapshot }

function checkpointed(x) {
  let snap = snapshot()
  match snap {
    Snapshot::Hash(id) => id,
    Snapshot::Resumed => x + 1
  }
}

checkpointed(41)
"#;

        let temp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(temp.path()).expect("snapshot store");

        let mut engine = ShapeEngine::new().expect("engine");
        engine.load_stdlib().expect("load stdlib");
        engine.enable_snapshot_store(store.clone());

        let executor_first = BytecodeExecutor::new();
        let first_result = engine
            .execute(&executor_first, source)
            .expect("first execute should succeed");
        assert!(
            first_result.value.as_str().is_some(),
            "first run should return snapshot hash string from Snapshot::Hash arm, got {:?}",
            first_result.value
        );

        let snapshot_id = engine
            .last_snapshot()
            .cloned()
            .expect("snapshot id should be recorded");
        let (semantic, context, vm_hash, bytecode_hash) = engine
            .load_snapshot(&snapshot_id)
            .expect("load snapshot metadata");
        engine
            .apply_snapshot(semantic, context)
            .expect("apply snapshot context");

        let vm_hash = vm_hash.expect("vm hash should be present");
        let bytecode_hash = bytecode_hash.expect("bytecode hash should be present");
        let vm_snapshot: VmSnapshot = store.get_struct(&vm_hash).expect("deserialize vm snapshot");
        let bytecode: BytecodeProgram = store
            .get_struct(&bytecode_hash)
            .expect("deserialize bytecode");
        let resume_ip = vm_snapshot.ip;
        assert!(
            resume_ip < bytecode.instructions.len(),
            "snapshot resume ip should be within instruction stream"
        );
        assert_eq!(
            bytecode.instructions[resume_ip].opcode,
            OpCode::StoreLocal,
            "snapshot resume ip should point to StoreLocal consuming snapshot() value"
        );

        let snapshot_schema = bytecode
            .type_schema_registry
            .get("Snapshot")
            .expect("bytecode should contain Snapshot schema");
        let snapshot_schema_id = snapshot_schema.id as u16;
        let snapshot_by_id = bytecode
            .type_schema_registry
            .get_by_id(snapshot_schema.id)
            .expect("Snapshot schema id should resolve");
        assert_eq!(
            snapshot_by_id.name, "Snapshot",
            "schema id mapping should resolve back to Snapshot"
        );
        let resumed_variant_id = snapshot_schema
            .get_enum_info()
            .and_then(|info| info.variant_id("Resumed"))
            .expect("Snapshot::Resumed variant should exist");

        let typed_field_type_ids: Vec<u16> = bytecode
            .instructions
            .iter()
            .filter_map(|instruction| match instruction.operand {
                Some(Operand::TypedField {
                    type_id, field_idx, ..
                }) if field_idx == 0 => Some(type_id),
                _ => None,
            })
            .collect();
        assert!(
            typed_field_type_ids.contains(&snapshot_schema_id),
            "match bytecode should reference Snapshot schema id {} (found typed field ids {:?})",
            snapshot_schema_id,
            typed_field_type_ids
        );

        let vm_probe = VirtualMachine::from_snapshot(bytecode.clone(), &vm_snapshot, &store)
            .expect("vm probe");
        let resumed_probe = vm_probe
            .create_typed_enum_nb("Snapshot", "Resumed", vec![])
            .expect("create typed Snapshot::Resumed");
        let (probe_schema_id, probe_slots, _) = resumed_probe
            .as_typed_object()
            .expect("resumed marker should be typed object");
        assert_eq!(
            probe_schema_id as u16, snapshot_schema_id,
            "resume marker schema should match compiled Snapshot schema"
        );
        assert!(
            !probe_slots.is_empty(),
            "typed enum marker should include variant discriminator slot"
        );
        assert_eq!(
            probe_slots[0].as_i64() as u16,
            resumed_variant_id,
            "resume marker variant id should be Snapshot::Resumed"
        );

        // Intentionally use a fresh executor to mimic a new process / session
        // where stdlib schema IDs may differ.
        let executor_resume = BytecodeExecutor::new();
        let resumed_result = executor_resume
            .resume_snapshot(&mut engine, vm_snapshot, bytecode)
            .expect("resume should succeed");

        assert_eq!(
            resumed_result.wire_value.as_number(),
            Some(42.0),
            "resume should take Snapshot::Resumed arm"
        );
    }

    #[test]
    fn snapshot_resumed_variant_matches_without_resume_flow() {
        let source = r#"
from std::core::snapshot use { Snapshot }

let marker = Snapshot::Resumed
match marker {
  Snapshot::Hash(id) => 0,
  Snapshot::Resumed => 1
}
"#;

        let mut engine = ShapeEngine::new().expect("engine");
        engine.load_stdlib().expect("load stdlib");
        let executor = BytecodeExecutor::new();
        let result = engine.execute(&executor, source).expect("execute");
        assert_eq!(
            result.value.as_number(),
            Some(1.0),
            "Snapshot::Resumed pattern should match direct enum constructor value"
        );
    }

    #[test]
    fn snapshot_resume_direct_vm_from_snapshot_with_marker() {
        let source = r#"
from std::core::snapshot use { Snapshot }

function checkpointed(x) {
  let snap = snapshot()
  match snap {
    Snapshot::Hash(id) => id,
    Snapshot::Resumed => x + 1
  }
}

checkpointed(41)
"#;

        let temp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(temp.path()).expect("snapshot store");

        let mut engine = ShapeEngine::new().expect("engine");
        engine.load_stdlib().expect("load stdlib");
        engine.enable_snapshot_store(store.clone());

        let executor = BytecodeExecutor::new();
        let _ = engine.execute(&executor, source).expect("first execute");

        let snapshot_id = engine
            .last_snapshot()
            .cloned()
            .expect("snapshot id should be recorded");
        let (_semantic, _context, vm_hash, bytecode_hash) = engine
            .load_snapshot(&snapshot_id)
            .expect("load snapshot metadata");
        let vm_hash = vm_hash.expect("vm hash");
        let bytecode_hash = bytecode_hash.expect("bytecode hash");
        let vm_snapshot: VmSnapshot = store.get_struct(&vm_hash).expect("vm snapshot");
        let bytecode: BytecodeProgram = store.get_struct(&bytecode_hash).expect("bytecode");

        let mut vm = VirtualMachine::from_snapshot(bytecode, &vm_snapshot, &store).expect("vm");
        let resumed = vm
            .create_typed_enum_nb("Snapshot", "Resumed", vec![])
            .expect("typed resumed marker");
        vm.push_vw(resumed).expect("push marker");

        let result = vm.execute_with_suspend(None).expect("vm execute");
        let value = match result {
            crate::VMExecutionResult::Completed(v) => v,
            crate::VMExecutionResult::Suspended { .. } => panic!("unexpected suspension"),
        };
        assert_eq!(
            value.as_i64(),
            Some(42),
            "direct VM resume should return 42"
        );
    }
}
